#![allow(dead_code)]

#[path = "../src/bin/research/data.rs"]
mod data;

use data::{cache_file_path, merge_rows, trim_rows_to_window, CandleRow};
use long_scalping::model::MrcBands;
use std::path::Path;

const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

fn candle(time_ms: i64, close: f64) -> CandleRow {
    CandleRow {
        time_ms,
        open: close,
        high: close,
        low: close,
        close,
        gain: 0.0,
        buy_sell_gain: 0.0,
        mrc: MrcBands::default(),
    }
}

#[test]
fn merge_rows_replaces_revised_candles_and_sorts_them() {
    let existing = vec![candle(120_000, 12.0), candle(60_000, 6.0)];
    let merged = merge_rows(&existing, vec![candle(180_000, 18.0), candle(60_000, 60.0)]);

    assert_eq!(
        merged.iter().map(|row| row.time_ms).collect::<Vec<_>>(),
        vec![60_000, 120_000, 180_000]
    );
    assert_eq!(merged[0].close, 60.0);
}

#[test]
fn trim_rows_to_window_keeps_only_the_latest_seven_days() {
    let latest = 8 * DAY_MS;
    let rows = trim_rows_to_window(
        vec![
            candle(0, 0.0),
            candle(6 * DAY_MS, 6.0),
            candle(7 * DAY_MS, 7.0),
            candle(latest, 8.0),
        ],
        3_600,
    );

    assert_eq!(
        rows.iter().map(|row| row.time_ms).collect::<Vec<_>>(),
        vec![6 * DAY_MS, 7 * DAY_MS, latest]
    );
}

#[test]
fn trim_rows_to_window_caps_rows_to_timeframe_window_size() {
    let rows = (0..10).map(|index| candle(index * MINUTE_MS, index as f64));
    let trimmed = trim_rows_to_window(rows.collect(), 86_400);

    assert_eq!(trimmed.len(), 8);
    assert_eq!(trimmed.first().map(|row| row.time_ms), Some(2 * MINUTE_MS));
}

#[test]
fn cache_file_path_is_scoped_by_symbol_timeframe_and_indicator() {
    let path = cache_file_path(Path::new("./tmp/cache"), "OKX#ETH-USDT-SWAP#SWAP", 60, None);

    assert_eq!(
        path,
        Path::new("./tmp/cache/OKX%23ETH-USDT-SWAP%23SWAP-60-all.json")
    );
}

#[test]
fn candle_rows_round_trip_through_cache_json() {
    let original = candle(60_000, 101.25);
    let encoded = serde_json::to_string(&original).expect("serialize candle");
    let decoded: CandleRow = serde_json::from_str(&encoded).expect("deserialize candle");

    assert_eq!(decoded, original);
}
