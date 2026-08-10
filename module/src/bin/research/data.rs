use std::collections::BTreeMap;
use std::error::Error;
use std::process::Command;

use long_scalping::model::MrcBands;
use serde_json::Value;

const BASE_URL: &str = "https://okx.tg.trading.sinvoid.me/api/indicators";
const BASE_HOST: &str = "okx.tg.trading.sinvoid.me";
const BASE_FALLBACK_IP: &str = "65.109.35.177";
const API_PAGE_LIMIT: usize = 200;

#[derive(Debug, Clone)]
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

pub(crate) fn fetch_history_for_timeframe(
    symbol_key: &str,
    timeframe: u64,
    limit: usize,
    indicator_name: Option<&str>,
) -> Result<Vec<CandleRow>, Box<dyn Error>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

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
            merged.entry(row.time_ms).or_insert(row);
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
