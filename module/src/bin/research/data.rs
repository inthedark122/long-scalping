use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use long_scalping::model::MrcBands;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const BASE_URL: &str = "https://okx.tg.trading.sinvoid.me/api/indicators";
const BASE_HOST: &str = "okx.tg.trading.sinvoid.me";
const BASE_FALLBACK_IP: &str = "65.109.35.177";
const API_PAGE_LIMIT: usize = 200;
const CACHE_VERSION: u32 = 1;
const CACHE_DIRECTORY: &str = "./tmp/cache";
const ROLLING_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CandleRow {
    pub(crate) time_ms: i64,
    pub(crate) open: f64,
    pub(crate) high: f64,
    pub(crate) low: f64,
    pub(crate) close: f64,
    pub(crate) zz_dir: i8,
    pub(crate) zz_break_price: f64,
    pub(crate) gain: f64,
    pub(crate) buy_sell_gain: f64,
    pub(crate) ntps: i64,
    pub(crate) mrc: MrcBands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CacheSettings {
    pub(crate) enabled: bool,
    pub(crate) clear: bool,
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            clear: false,
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

fn dir_to_int(value: Option<&Value>) -> i8 {
    match value
        .and_then(Value::as_str)
        .map(|s| s.to_ascii_uppercase())
    {
        Some(s) if s == "LONG" => 1,
        Some(s) if s == "SHORT" => -1,
        _ => 0,
    }
}

pub(crate) fn f64_or(obj: &Value, primary: &str, fallback: &str) -> f64 {
    let parse = |v: &Value| {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
    };
    obj.get(primary)
        .and_then(parse)
        .or_else(|| obj.get(fallback).and_then(parse))
        .unwrap_or(0.0)
}

pub(crate) fn get_i64(map: &Value, key: &str) -> i64 {
    map.get(key)
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .unwrap_or(0)
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
) -> Option<Vec<CandleRow>> {
    if document.version != CACHE_VERSION
        || document.symbol_key != symbol_key
        || document.timeframe != timeframe
        || document.indicator_name.as_deref() != indicator_name
    {
        return None;
    }
    let rows = trim_rows_to_window(document.rows, timeframe);
    (!rows.is_empty() && !rows_have_gap(&rows, timeframe)).then_some(rows)
}

fn read_cache(
    path: &Path,
    symbol_key: &str,
    timeframe: u64,
    indicator_name: Option<&str>,
) -> Option<Vec<CandleRow>> {
    let contents = fs::read_to_string(path).ok()?;
    let document = serde_json::from_str::<CacheDocument>(&contents).ok()?;
    cache_document_is_usable(document, symbol_key, timeframe, indicator_name)
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
    let mut merged = BTreeMap::<i64, CandleRow>::new();
    let mut offset = 0usize;
    while merged.len() < limit {
        let page_limit = (limit - merged.len()).min(API_PAGE_LIMIT);
        let page = fetch_history_page(symbol_key, timeframe, page_limit, offset, indicator_name)?;
        if page.is_empty() {
            break;
        }

        let page_len = page.len();
        let previous_len = merged.len();
        for row in page {
            // The API is newest-first. The timestamp key makes retries or
            // overlapping page boundaries idempotent before chronological sort.
            merged.insert(row.time_ms, row);
        }
        offset = offset.saturating_add(page_len);

        if merged.len() == previous_len || page_len < page_limit {
            break;
        }
    }

    let mut rows = merged.into_values().collect::<Vec<_>>();
    rows.sort_by_key(|row| row.time_ms);
    if rows.len() > limit {
        rows.drain(..rows.len() - limit);
    }
    Ok(rows)
}

fn refresh_cached_rows(
    symbol_key: &str,
    timeframe: u64,
    indicator_name: Option<&str>,
    cached: &[CandleRow],
) -> Result<Vec<CandleRow>, Box<dyn Error>> {
    let Some(cached_latest) = cached.last().map(|row| row.time_ms) else {
        return fetch_history_uncached(symbol_key, timeframe, API_PAGE_LIMIT, indicator_name);
    };
    let interval_ms = (timeframe as i64).saturating_mul(1_000);
    let mut incoming = Vec::new();
    let mut offset = 0usize;

    loop {
        let page = fetch_history_page(
            symbol_key,
            timeframe,
            API_PAGE_LIMIT,
            offset,
            indicator_name,
        )?;
        if page.is_empty() {
            return Err("incremental candle refresh did not bridge the cached range".into());
        }

        let page_len = page.len();
        let bridged = page
            .first()
            .is_some_and(|row| row.time_ms <= cached_latest.saturating_add(interval_ms));
        incoming.extend(page);
        if bridged || page_len < API_PAGE_LIMIT {
            break;
        }
        offset = offset.saturating_add(page_len);
    }

    let rows = merge_rows(cached, incoming);
    if rows_have_gap(&rows, timeframe) {
        return Err("cached candle range contains a timestamp gap".into());
    }
    Ok(rows)
}

fn resolve_host_ip() -> Option<String> {
    let output = Command::new("getent")
        .args(["hosts", BASE_HOST])
        .output()
        .ok();
    let ip = output
        .and_then(|out| {
            if !out.status.success() {
                return None;
            }
            String::from_utf8(out.stdout).ok()
        })
        .and_then(|text| text.split_whitespace().next().map(str::to_string));
    Some(ip.unwrap_or_else(|| BASE_FALLBACK_IP.to_string()))
}

fn parse_mrc(ind: &Value) -> MrcBands {
    MrcBands {
        down_big: f64_or(ind, "DownBig", "down_big"),
        down_outer: f64_or(ind, "DownOuter", "down_outer"),
        down_small: f64_or(ind, "DownSmall", "down_small"),
        down_inner: f64_or(ind, "DownInner", "down_inner"),
        mean_line: f64_or(ind, "MeanLine", "mean_line"),
        up_inner: f64_or(ind, "UpInner", "up_inner"),
        up_small: f64_or(ind, "UpSmall", "up_small"),
        up_outer: f64_or(ind, "UpOuter", "up_outer"),
        up_big: f64_or(ind, "UpBig", "up_big"),
        current_cross: f64_or(ind, "CurrentCross", "current_cross"),
        mean_line_slope: 0.0,
    }
}

fn parse_candle(item: &Value) -> CandleRow {
    let ind = item.get("indicators").unwrap_or(&Value::Null);
    let candle = ind.get("candle").unwrap_or(ind);
    let mrc = ind.get("mrc_200").unwrap_or(&Value::Null);
    let zz = ind.get("zigzagtrend").unwrap_or(&Value::Null);
    let signal = ind.get("signal").unwrap_or(&Value::Null);
    let volume = ind.get("volume_50").unwrap_or(&Value::Null);

    CandleRow {
        time_ms: get_i64(item, "time"),
        open: f64_or(candle, "Open", "open"),
        high: f64_or(candle, "High", "high"),
        low: f64_or(candle, "Low", "low"),
        close: f64_or(candle, "Close", "close"),
        zz_dir: dir_to_int(zz.get("Direction")),
        zz_break_price: f64_or(zz, "BreakPrice", "break_price"),
        gain: f64_or(volume, "Gain", "gain"),
        buy_sell_gain: f64_or(volume, "BuySellGain", "buy_sell_gain"),
        ntps: get_i64(signal, "ntps"),
        mrc: parse_mrc(mrc),
    }
}

fn fetch_history_page(
    symbol_key: &str,
    timeframe: u64,
    limit: usize,
    offset: usize,
    indicator_name: Option<&str>,
) -> Result<Vec<CandleRow>, Box<dyn Error>> {
    let indicator_query = indicator_name
        .map(|name| format!("&indicator_name={name}"))
        .unwrap_or_default();
    let url = format!(
        "{}?symbol_key={}&timeframe={}&limit={}&offset={}{}",
        BASE_URL,
        encode_symbol_key(symbol_key),
        timeframe,
        limit.min(API_PAGE_LIMIT),
        offset,
        indicator_query
    );
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--max-time", "180"]);
    if let Some(ip) = resolve_host_ip() {
        cmd.args(["--resolve", &format!("{}:443:{}", BASE_HOST, ip)]);
    }
    let output = cmd.arg(&url).output()?;
    if !output.status.success() {
        return Err(format!("curl fetch failed for {symbol_key} tf={timeframe}").into());
    }
    let payload: Value = serde_json::from_slice(&output.stdout)?;
    let mut rows = payload
        .get("data")
        .and_then(Value::as_array)
        .map(|data| data.iter().map(parse_candle).collect::<Vec<_>>())
        .unwrap_or_default();
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

    let path = cache_file_path(
        Path::new(CACHE_DIRECTORY),
        symbol_key,
        timeframe,
        indicator_name,
    );
    if settings.clear {
        let _ = fs::remove_file(&path);
    }

    let Some(cached) = read_cache(&path, symbol_key, timeframe, indicator_name) else {
        let rows = trim_rows_to_window(
            fetch_history_uncached(symbol_key, timeframe, limit, indicator_name)?,
            timeframe,
        );
        write_cache(&path, symbol_key, timeframe, indicator_name, &rows)?;
        return Ok((rows, CacheStatus::Rebuilt));
    };

    let (mut rows, mut status) =
        match refresh_cached_rows(symbol_key, timeframe, indicator_name, &cached) {
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
    Ok((rows.into_iter().rev().collect(), status))
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
