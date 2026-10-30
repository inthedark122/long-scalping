use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use long_scalping::model::MrcBands;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ta::indicators::MeanReversionChannel;
use ta::{Close, High, Low, Next};

const BASE_URL: &str = "https://market-data-api.livtorgex.com/api/klines";
const API_PAGE_LIMIT: usize = 500;
pub(crate) const MRC_PERIOD: usize = 200;
const CACHE_VERSION: u32 = 4;
const CACHE_DIRECTORY: &str = "./tmp/cache";
const ROLLING_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CandleRow {
    pub(crate) time_ms: i64,
    pub(crate) open: f64,
    pub(crate) high: f64,
    pub(crate) low: f64,
    pub(crate) close: f64,
    pub(crate) gain: f64,
    pub(crate) buy_sell_gain: f64,
    #[serde(default)]
    pub(crate) ntps: i64,
    pub(crate) mrc: MrcBands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CacheSettings {
    pub(crate) enabled: bool,
    pub(crate) clear: bool,
    pub(crate) refresh: bool,
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            clear: false,
            refresh: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheStatus {
    Disabled,
    Rebuilt,
    Refreshed,
    Hit,
}

impl fmt::Display for CacheStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Disabled => "bypassed",
            Self::Rebuilt => "rebuilt",
            Self::Refreshed => "refreshed",
            Self::Hit => "hit",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheDocument {
    version: u32,
    symbol_key: String,
    timeframe: u64,
    indicator_name: Option<String>,
    rows: Vec<CandleRow>,
}

pub(crate) fn f64_or(obj: &Value, primary: &str, fallback: &str) -> f64 {
    fn parse(value: &Value) -> Option<f64> {
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|value| value.parse::<f64>().ok()))
            .or_else(|| value.get("Value").and_then(parse))
            .or_else(|| value.get("value").and_then(parse))
    }
    obj.get(primary)
        .and_then(parse)
        .or_else(|| obj.get(fallback).and_then(parse))
        .unwrap_or_default()
}

fn encode_symbol_key(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            _ => out.push_str(&format!("%{:02X}", ch as u32)),
        }
    }
    out
}

pub(crate) fn cache_file_path(
    directory: &Path,
    symbol_key: &str,
    timeframe: u64,
    indicator_name: Option<&str>,
) -> PathBuf {
    let indicator = indicator_name.unwrap_or("all");
    directory.join(format!(
        "{}-{}-{}.json",
        encode_symbol_key(symbol_key),
        timeframe,
        encode_symbol_key(indicator)
    ))
}

pub(crate) fn merge_rows(
    existing: &[CandleRow],
    incoming: impl IntoIterator<Item = CandleRow>,
) -> Vec<CandleRow> {
    let mut merged = BTreeMap::<i64, CandleRow>::new();
    for row in existing.iter().cloned().chain(incoming) {
        // New API data wins so a still-forming candle is refreshed on every run.
        merged.insert(row.time_ms, row);
    }
    merged.into_values().collect()
}

pub(crate) fn trim_rows_to_window(mut rows: Vec<CandleRow>, timeframe: u64) -> Vec<CandleRow> {
    rows.sort_by_key(|row| row.time_ms);
    let Some(latest) = rows.last().map(|row| row.time_ms) else {
        return rows;
    };
    let window_start = latest.saturating_sub(ROLLING_WINDOW_MS);
    rows.retain(|row| row.time_ms >= window_start);
    let max_rows = (ROLLING_WINDOW_MS / (timeframe.max(1) as i64 * 1_000) + 1) as usize;
    if rows.len() > max_rows {
        rows.drain(..rows.len() - max_rows);
    }
    rows
}

fn rows_have_gap(rows: &[CandleRow], timeframe: u64) -> bool {
    let interval_ms = timeframe.max(1) as i64 * 1_000;
    rows.windows(2)
        .any(|pair| pair[1].time_ms.saturating_sub(pair[0].time_ms) > interval_ms)
}

fn cache_document_is_usable(
    document: CacheDocument,
    symbol_key: &str,
    timeframe: u64,
    indicator_name: Option<&str>,
    allow_gaps: bool,
) -> Option<Vec<CandleRow>> {
    if document.version != CACHE_VERSION
        || document.symbol_key != symbol_key
        || document.timeframe != timeframe
        || document.indicator_name.as_deref() != indicator_name
    {
        return None;
    }
    let rows = trim_rows_to_window(document.rows, timeframe);
    (!rows.is_empty() && (allow_gaps || !rows_have_gap(&rows, timeframe))).then_some(rows)
}

fn read_cache(
    path: &Path,
    symbol_key: &str,
    timeframe: u64,
    indicator_name: Option<&str>,
    allow_gaps: bool,
) -> Option<Vec<CandleRow>> {
    let contents = fs::read_to_string(path).ok()?;
    let document = serde_json::from_str::<CacheDocument>(&contents).ok()?;
    cache_document_is_usable(document, symbol_key, timeframe, indicator_name, allow_gaps)
}

fn write_cache(
    path: &Path,
    symbol_key: &str,
    timeframe: u64,
    indicator_name: Option<&str>,
    rows: &[CandleRow],
) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let document = CacheDocument {
        version: CACHE_VERSION,
        symbol_key: symbol_key.to_string(),
        timeframe,
        indicator_name: indicator_name.map(str::to_string),
        rows: rows.to_vec(),
    };
    let encoded = serde_json::to_vec_pretty(&document)?;
    let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
    fs::write(&temporary, encoded)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

fn fetch_history_uncached(
    symbol_key: &str,
    timeframe: u64,
    limit: usize,
    indicator_name: Option<&str>,
) -> Result<Vec<CandleRow>, Box<dyn Error>> {
    let started = Instant::now();
    eprintln!(
        "[mrc-research][fetch] start symbol={} timeframe={} limit={} indicator={:?}",
        symbol_key, timeframe, limit, indicator_name
    );
    let mut merged = BTreeMap::<i64, CandleRow>::new();
    let interval_ms = (timeframe.max(1) as i64).saturating_mul(1_000);
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64;
    let mut end_ms = now_ms / interval_ms * interval_ms;
    while merged.len() < limit {
        let page_limit = (limit - merged.len()).min(API_PAGE_LIMIT);
        let start_ms = end_ms.saturating_sub(interval_ms.saturating_mul(page_limit as i64));
        eprintln!(
            "[mrc-research][fetch] request symbol={} timeframe={} start={} end={} page_limit={}",
            symbol_key, timeframe, start_ms, end_ms, page_limit
        );
        let page = fetch_history_page(
            symbol_key,
            timeframe,
            page_limit,
            start_ms,
            end_ms,
            indicator_name,
        )?;
        if page.is_empty() {
            eprintln!(
                "[mrc-research][fetch] empty page symbol={} timeframe={} start={} end={} elapsed_ms={}",
                symbol_key,
                timeframe,
                start_ms,
                end_ms,
                started.elapsed().as_millis()
            );
            break;
        }

        let page_len = page.len();
        let page_oldest = page.first().map(|row| row.time_ms).unwrap_or(end_ms);
        let previous_len = merged.len();
        for row in page {
            merged.insert(row.time_ms, row);
        }

        eprintln!(
            "[mrc-research][fetch] page symbol={} timeframe={} oldest={} rows={} total_rows={} elapsed_ms={}",
            symbol_key,
            timeframe,
            page_oldest,
            page_len,
            merged.len(),
            started.elapsed().as_millis()
        );

        if merged.len() == previous_len || page_oldest >= end_ms {
            break;
        }
        end_ms = page_oldest;
    }

    let mut rows = merged.into_values().collect::<Vec<_>>();
    rows.sort_by_key(|row| row.time_ms);
    if rows.len() > limit {
        rows.drain(..rows.len() - limit);
    }
    eprintln!(
        "[mrc-research][fetch] complete symbol={} timeframe={} rows={} elapsed_ms={}",
        symbol_key,
        timeframe,
        rows.len(),
        started.elapsed().as_millis()
    );
    Ok(rows)
}

fn refresh_cached_rows(
    symbol_key: &str,
    timeframe: u64,
    limit: usize,
    indicator_name: Option<&str>,
    cached: &[CandleRow],
) -> Result<Vec<CandleRow>, Box<dyn Error>> {
    let started = Instant::now();
    eprintln!(
        "[mrc-research][cache-refresh] start symbol={} timeframe={} limit={} cached_rows={} ",
        symbol_key,
        timeframe,
        limit,
        cached.len()
    );
    let incoming = fetch_history_uncached(symbol_key, timeframe, limit, indicator_name)?;
    let rows = merge_rows(cached, incoming);
    eprintln!(
        "[mrc-research][cache-refresh] complete symbol={} timeframe={} rows={} elapsed_ms={}",
        symbol_key,
        timeframe,
        rows.len(),
        started.elapsed().as_millis()
    );
    Ok(rows)
}

fn value_at<'a>(item: &'a Value, index: usize, names: &[&str]) -> Option<&'a Value> {
    item.as_array()
        .and_then(|values| values.get(index))
        .or_else(|| names.iter().find_map(|name| item.get(*name)))
}

fn value_as_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn parse_candle(item: &Value) -> Option<CandleRow> {
    let time_ms = value_at(item, 0, &["time_ms", "timestamp", "time", "ts"]).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })?;
    let open = value_at(item, 1, &["open", "Open", "o"]).and_then(value_as_f64)?;
    let high = value_at(item, 2, &["high", "High", "h"]).and_then(value_as_f64)?;
    let low = value_at(item, 3, &["low", "Low", "l"]).and_then(value_as_f64)?;
    let close = value_at(item, 4, &["close", "Close", "c"]).and_then(value_as_f64)?;
    let qty_buy = item
        .get("qty_buy")
        .and_then(value_as_f64)
        .unwrap_or_default();
    let qty_sell = item
        .get("qty_sell")
        .and_then(value_as_f64)
        .unwrap_or_default();
    let asset_buy = item.get("asset_buy").and_then(value_as_f64);
    let asset_sell = item.get("asset_sell").and_then(value_as_f64);
    let volume = value_at(item, 5, &["volume", "vol", "base_volume", "volume_base"])
        .and_then(value_as_f64)
        .unwrap_or(qty_buy + qty_sell);
    let quote_volume = item
        .as_array()
        .and_then(|values| values.get(7))
        .or_else(|| {
            [
                "quote_volume",
                "quoteVolume",
                "volume_quote",
                "volCcyQuote",
                "turnover",
            ]
            .iter()
            .find_map(|name| item.get(*name))
        })
        .and_then(value_as_f64)
        .or_else(|| Some(asset_buy.unwrap_or_default() + asset_sell.unwrap_or_default()))
        .filter(|value| *value != 0.0)
        .unwrap_or(volume);
    let buy_sell_gain = match (asset_buy, asset_sell) {
        (Some(buy), Some(sell)) => buy - sell,
        _ => 0.0,
    };
    let ntps = item
        .get("ntps")
        .or_else(|| item.get("NTPS"))
        .and_then(value_as_f64)
        .unwrap_or_default()
        .round() as i64;

    Some(CandleRow {
        time_ms,
        open,
        high,
        low,
        close,
        gain: quote_volume,
        buy_sell_gain,
        ntps,
        mrc: MrcBands::default(),
    })
}

#[derive(Clone, Copy)]
struct TaCandle {
    high: f64,
    low: f64,
    close: f64,
}

impl High for TaCandle {
    fn high(&self) -> f64 {
        self.high
    }
}

impl Low for TaCandle {
    fn low(&self) -> f64 {
        self.low
    }
}

impl Close for TaCandle {
    fn close(&self) -> f64 {
        self.close
    }
}

pub(crate) fn compute_mrc(rows: &mut [CandleRow]) {
    let mut channel = MeanReversionChannel::new(MRC_PERIOD);
    let mut previous_middle = None;
    for row in rows {
        let output = channel.next(&TaCandle {
            high: row.high,
            low: row.low,
            close: row.close,
        });
        row.mrc = MrcBands {
            down_big: output.down_big,
            down_outer: output.down_outer,
            down_small: output.down_small,
            down_inner: output.down_inner,
            mean_line: output.middle,
            up_inner: output.up_inner,
            up_small: output.up_small,
            up_outer: output.up_outer,
            up_big: output.up_big,
            current_cross: 0.0,
            mean_line_slope: previous_middle
                .map(|previous: f64| output.middle - previous)
                .unwrap_or_default(),
        };
        previous_middle = Some(output.middle);
    }
}

fn fetch_history_page(
    symbol_key: &str,
    timeframe: u64,
    limit: usize,
    start_ms: i64,
    end_ms: i64,
    indicator_name: Option<&str>,
) -> Result<Vec<CandleRow>, Box<dyn Error>> {
    let _ = indicator_name;
    let url = format!(
        "{}?symbol_key={}&timeframe={}&time_start={}&time_end={}&limit={}",
        BASE_URL,
        encode_symbol_key(&symbol_key.to_ascii_lowercase()),
        timeframe,
        start_ms,
        end_ms,
        limit.min(API_PAGE_LIMIT),
    );
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--max-time", "180"]);
    let output = cmd.arg(&url).output()?;
    if !output.status.success() {
        return Err(format!("curl fetch failed for {symbol_key} tf={timeframe}").into());
    }
    let payload: Value = serde_json::from_slice(&output.stdout)?;
    let data = payload
        .as_array()
        .or_else(|| payload.get("data").and_then(Value::as_array))
        .or_else(|| payload.get("klines").and_then(Value::as_array));
    let Some(data) = data else {
        return Err(format!("klines response has no data array for {symbol_key}").into());
    };
    let mut rows = data
        .iter()
        .filter_map(parse_candle)
        .filter(|row| row.time_ms >= start_ms && row.time_ms < end_ms)
        .collect::<Vec<_>>();
    if rows.is_empty() && !data.is_empty() {
        return Err(format!(
            "klines response contained no parseable rows in [{start_ms}, {end_ms}) for {symbol_key}"
        )
        .into());
    }
    rows.sort_by_key(|row| row.time_ms);
    Ok(rows)
}

pub(crate) fn fetch_history_for_timeframe_uncached(
    symbol_key: &str,
    timeframe: u64,
    limit: usize,
    indicator_name: Option<&str>,
) -> Result<Vec<CandleRow>, Box<dyn Error>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    fetch_history_uncached(symbol_key, timeframe, limit, indicator_name)
}

pub(crate) fn fetch_history_for_timeframe_cached(
    symbol_key: &str,
    timeframe: u64,
    limit: usize,
    indicator_name: Option<&str>,
    settings: CacheSettings,
) -> Result<(Vec<CandleRow>, CacheStatus), Box<dyn Error>> {
    if limit == 0 {
        return Ok((Vec::new(), CacheStatus::Disabled));
    }
    if !settings.enabled {
        return Ok((
            fetch_history_uncached(symbol_key, timeframe, limit, indicator_name)?,
            CacheStatus::Disabled,
        ));
    }

    let cache_directory = std::env::var_os("LIVTORGEX_RESEARCH_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(CACHE_DIRECTORY));
    let path = cache_file_path(&cache_directory, symbol_key, timeframe, indicator_name);
    eprintln!(
        "[mrc-research][cache] lookup symbol={} timeframe={} limit={} path={} enabled={} clear={} refresh={}",
        symbol_key,
        timeframe,
        limit,
        path.display(),
        settings.enabled,
        settings.clear,
        settings.refresh
    );
    if settings.clear {
        let _ = fs::remove_file(&path);
        eprintln!(
            "[mrc-research][cache] cleared symbol={} timeframe={} path={}",
            symbol_key,
            timeframe,
            path.display()
        );
    }

    let Some(cached) = read_cache(
        &path,
        symbol_key,
        timeframe,
        indicator_name,
        !settings.refresh,
    ) else {
        eprintln!(
            "[mrc-research][cache] miss symbol={} timeframe={} path={}",
            symbol_key,
            timeframe,
            path.display()
        );
        let rows = trim_rows_to_window(
            fetch_history_uncached(symbol_key, timeframe, limit, indicator_name)?,
            timeframe,
        );
        write_cache(&path, symbol_key, timeframe, indicator_name, &rows)?;
        eprintln!(
            "[mrc-research][cache] rebuilt symbol={} timeframe={} rows={} path={}",
            symbol_key,
            timeframe,
            rows.len(),
            path.display()
        );
        return Ok((rows, CacheStatus::Rebuilt));
    };
    eprintln!(
        "[mrc-research][cache] hit-before-refresh symbol={} timeframe={} rows={} latest={}",
        symbol_key,
        timeframe,
        cached.len(),
        cached.last().map(|row| row.time_ms).unwrap_or_default()
    );
    if !settings.refresh {
        let rows = cached.into_iter().rev().take(limit).collect::<Vec<_>>();
        let rows = rows.into_iter().rev().collect::<Vec<_>>();
        eprintln!(
            "[mrc-research][cache] complete symbol={} timeframe={} status={} rows={} path={} api_refresh=false",
            symbol_key,
            timeframe,
            CacheStatus::Hit,
            rows.len(),
            path.display()
        );
        return Ok((rows, CacheStatus::Hit));
    }

    let (mut rows, mut status) =
        match refresh_cached_rows(symbol_key, timeframe, limit, indicator_name, &cached) {
            Ok(rows) => (rows, None),
            Err(_) => (
                fetch_history_uncached(symbol_key, timeframe, limit, indicator_name)?,
                Some(CacheStatus::Rebuilt),
            ),
        };
    let status = status.take().unwrap_or_else(|| {
        if rows == cached {
            CacheStatus::Hit
        } else {
            CacheStatus::Refreshed
        }
    });

    // A larger --history request may need older rows than the cache currently
    // contains. Reuse the refreshed cache first, then fill the missing range
    // through the existing paginated API path.
    if rows.len() < limit {
        rows = merge_rows(
            &rows,
            fetch_history_uncached(symbol_key, timeframe, limit, indicator_name)?,
        );
    }
    rows = trim_rows_to_window(rows, timeframe);
    write_cache(&path, symbol_key, timeframe, indicator_name, &rows)?;

    let rows = rows.into_iter().rev().take(limit).collect::<Vec<_>>();
    let rows = rows.into_iter().rev().collect::<Vec<_>>();
    eprintln!(
        "[mrc-research][cache] complete symbol={} timeframe={} status={} rows={} path={}",
        symbol_key,
        timeframe,
        status,
        rows.len(),
        path.display()
    );
    Ok((rows, status))
}

pub(crate) fn fetch_history_for_timeframe(
    symbol_key: &str,
    timeframe: u64,
    limit: usize,
    indicator_name: Option<&str>,
) -> Result<Vec<CandleRow>, Box<dyn Error>> {
    fetch_history_for_timeframe_uncached(symbol_key, timeframe, limit, indicator_name)
}

pub(crate) fn fetch_history(
    symbol_key: &str,
    limit: usize,
) -> Result<Vec<CandleRow>, Box<dyn Error>> {
    fetch_history_for_timeframe(symbol_key, 60, limit, None)
}

pub(crate) fn fetch_candle_history(
    symbol_key: &str,
    limit: usize,
) -> Result<Vec<CandleRow>, Box<dyn Error>> {
    fetch_history_for_timeframe(symbol_key, 60, limit, Some("candle"))
}

pub(crate) fn fetch_full_history(
    symbol_key: &str,
    seed_limit: usize,
) -> Result<Vec<CandleRow>, Box<dyn Error>> {
    fetch_history(symbol_key, seed_limit.max(5_000))
}
