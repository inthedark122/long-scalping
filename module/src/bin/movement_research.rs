#![allow(dead_code)]

use std::error::Error;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

#[path = "../builder/mod.rs"]
mod builder;
#[path = "../detection/cascade.rs"]
mod cascade_impl;
#[path = "../martingale.rs"]
mod martingale;
#[path = "../model.rs"]
mod model;
#[path = "../detection/mrc.rs"]
mod mrc_impl;
#[path = "../orders.rs"]
mod orders;
#[path = "../observer/mod.rs"]
mod observer;
#[path = "../state.rs"]
mod state;
mod detection {
    pub mod mrc {
        pub use crate::mrc_impl::*;
    }
    pub mod cascade {
        pub use crate::cascade_impl::*;
    }
}

use detection::cascade::{self, CascadeSignal, MovementPhase};
use model::Candle;
use state::CascadeState;

const BASE_URL: &str = "https://okx.tg.trading.sinvoid.me/api/indicators";
const BASE_HOST: &str = "okx.tg.trading.sinvoid.me";
const BASE_FALLBACK_IP: &str = "65.109.35.177";
const DEFAULT_SYMBOL_KEY: &str = "OKX#ETH-USDT-SWAP#SWAP";
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn handle_sigint(_: i32) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
fn install_sigint_handler() {
    unsafe {
        extern "C" {
            fn signal(signum: i32, handler: extern "C" fn(i32)) -> extern "C" fn(i32);
        }
        const SIGINT: i32 = 2;
        let _ = signal(SIGINT, handle_sigint);
    }
}

#[cfg(not(unix))]
fn install_sigint_handler() {}
#[derive(Debug, Clone)]
struct CandleRow {
    time_ms: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    dp_dir: i8,
    zz_dir: i8,
    dp_zee: f64,
    dp_zee1: f64,
    zz_break_price: f64,
    gain: f64,
    ntps: i64,
}

#[derive(Debug, Clone)]
struct DirectionLock {
    dir: i8,
    break_price: f64,
    reason: &'static str,
}

#[derive(Debug, Clone)]
struct PendingEntry {
    side: i8,
    armed_tick: i64,
    reason: String,
}

#[derive(Debug, Clone)]
struct VirtualTrade {
    side: i8,
    entry_pct: f64,
    tp_pct: f64,
    sl_pct: f64,
    reason: String,
    max_favorable_pct: f64,
    max_adverse_pct: f64,
}

#[derive(Debug, Clone)]
struct TradeResult {
    side: i8,
    pnl_pct: f64,
    mfe_pct: f64,
    mae_pct: f64,
    reason: String,
}

#[derive(Debug, Clone)]
struct TradeEvent {
    kind: &'static str,
    side: &'static str,
    time_ms: i64,
    reason: String,
    pnl_pct: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidewaySource {
    OrderBlock,
    LineCross,
}

#[derive(Debug, Clone)]
struct SidewayEvent {
    time_ms: i64,
    source: SidewaySource,
    side: i8,
    entry_price: f64,
    block_low: f64,
    block_high: f64,
    reason: String,
}

#[derive(Debug, Clone)]
struct SidewayProbe {
    event: SidewayEvent,
    block_width_pct: f64,
    long_mfe_pct: f64,
    long_mae_pct: f64,
    short_mfe_pct: f64,
    short_mae_pct: f64,
    max_move_pct: f64,
    post_breakout_max_move_pct: f64,
    move_gt_1pct_after_breakout: bool,
    actual_dir: i8,
    direction_match: bool,
    sideway_persist_candles: usize,
    sideway_persist_minutes: f64,
    breakout_candles: usize,
    breakout_minutes: f64,
    breakout_dir: i8,
}

fn pct_range(lo: f64, hi: f64, price: f64) -> f64 {
    if price <= 0.0 {
        0.0
    } else {
        ((hi - lo).abs() / price) * 100.0
    }
}

fn fmt_time(ms: i64) -> String {
    if ms <= 0 {
        return "?".to_string();
    }
    let secs = ms / 1000;
    let days = secs / 86_400;
    let rem = secs % 86_400;
    format!(
        "d{} {:02}:{:02}:{:02}",
        days,
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
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

fn f64_or(obj: &Value, primary: &str, fallback: &str) -> f64 {
    let parse = |v: &Value| {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
    };
    obj.get(primary)
        .and_then(parse)
        .or_else(|| obj.get(fallback).and_then(parse))
        .unwrap_or(0.0)
}

fn get_i64(map: &Value, key: &str) -> i64 {
    map.get(key)
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .unwrap_or(0)
}

fn candle_pct(candle: &CandleRow) -> f64 {
    pct_range(candle.low, candle.high, candle.close)
}

fn block_pct(size: f64, price: f64) -> f64 {
    if price <= 0.0 {
        0.0
    } else {
        (size.abs() / price) * 100.0
    }
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
        .and_then(|text| text.split_whitespace().next().map(|ip| ip.to_string()));
    Some(ip.unwrap_or_else(|| BASE_FALLBACK_IP.to_string()))
}

fn candle_range(row: &CandleRow) -> f64 {
    (row.high - row.low).abs()
}

fn candle_body(row: &CandleRow) -> f64 {
    (row.close - row.open).abs()
}

fn candle_dir(row: &CandleRow) -> i8 {
    if row.close > row.open {
        1
    } else if row.close < row.open {
        -1
    } else {
        0
    }
}

fn candle_at(candles: &[CandleRow], idx: usize, offset: usize) -> Option<&CandleRow> {
    idx.checked_sub(offset).and_then(|pos| candles.get(pos))
}

fn average_range(candles: &[CandleRow], idx: usize, start_offset: usize, end_offset: usize) -> f64 {
    let mut total = 0.0;
    let mut count = 0.0;
    for offset in start_offset..=end_offset {
        if let Some(row) = candle_at(candles, idx, offset) {
            let range = candle_range(row);
            if range > 0.0 {
                total += range;
                count += 1.0;
            }
        }
    }
    if count > 0.0 {
        total / count
    } else {
        0.0
    }
}

fn detect_sideway(candles: &[CandleRow], idx: usize) -> Option<SidewayEvent> {
    let current = candles.get(idx)?;
    let current_price = current.close.max(f64::EPSILON);
    let avg_range = average_range(candles, idx, 1, 5);
    if avg_range <= 0.0 {
        return None;
    }

    for impulse_offset in 0..=4 {
        let impulse = candle_at(candles, idx, impulse_offset)?;
        let impulse_range = candle_range(impulse);
        let impulse_body = candle_body(impulse);
        let body_ratio = if impulse_range > 0.0 {
            impulse_body / impulse_range
        } else {
            0.0
        };
        if impulse_range < avg_range * 1.5 || body_ratio < 0.55 {
            continue;
        }
        let direction = candle_dir(impulse);
        if direction == 0 {
            continue;
        }

        let mut block = None;
        for older_offset in (impulse_offset + 1)..=((impulse_offset + 3).min(9)) {
            let older = candle_at(candles, idx, older_offset)?;
            let older_dir = candle_dir(older);
            if older_dir != 0 && older_dir != direction {
                block = Some(older);
                break;
            }
        }
        let Some(block) = block else {
            continue;
        };

        let block_low = block.low;
        let block_high = block.high;
        if block_high <= block_low {
            continue;
        }

        let tolerance = (current_price * 0.0015).max(avg_range * 0.10);
        let inside =
            current.close >= block_low - tolerance && current.close <= block_high + tolerance;
        let window_high = (0..=2)
            .filter_map(|o| candle_at(candles, idx, o).map(|row| row.high))
            .fold(f64::NEG_INFINITY, f64::max);
        let window_low = (0..=2)
            .filter_map(|o| candle_at(candles, idx, o).map(|row| row.low))
            .fold(f64::INFINITY, f64::min);
        let compressed = ((window_high - window_low).abs() / current_price) * 100.0 <= 0.75;

        if inside && compressed {
            return Some(SidewayEvent {
                time_ms: current.time_ms,
                source: SidewaySource::OrderBlock,
                side: if direction > 0 { 1 } else { -1 },
                entry_price: current.close,
                block_low,
                block_high,
                reason: format!(
                    "ob dir={} impulse={:.2}% body={:.2}% block={:.4}-{:.4} tol={:.4}",
                    direction,
                    (impulse_range / current_price) * 100.0,
                    body_ratio,
                    block_low,
                    block_high,
                    tolerance
                ),
            });
        }
    }

    let closes: Vec<f64> = (0..4)
        .filter_map(|o| candle_at(candles, idx, o).map(|row| row.close))
        .collect();
    let highs: Vec<f64> = (0..4)
        .filter_map(|o| candle_at(candles, idx, o).map(|row| row.high))
        .collect();
    let lows: Vec<f64> = (0..4)
        .filter_map(|o| candle_at(candles, idx, o).map(|row| row.low))
        .collect();
    if closes.len() < 3 {
        return None;
    }
    let mean = closes.iter().copied().sum::<f64>() / closes.len() as f64;
    let mut crosses = 0;
    for pair in closes.windows(2) {
        let a = pair[0] - mean;
        let b = pair[1] - mean;
        if a == 0.0 || b == 0.0 || a.signum() != b.signum() {
            crosses += 1;
        }
    }
    let window_high = highs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let window_low = lows.iter().copied().fold(f64::INFINITY, f64::min);
    let span_pct = ((window_high - window_low).abs() / current_price) * 100.0;
    let range_pct = candle_range(current) / current_price * 100.0;
    if crosses >= 2 && range_pct <= 0.75 && span_pct <= 1.50 {
        Some(SidewayEvent {
            time_ms: current.time_ms,
            source: SidewaySource::LineCross,
            side: if current.close >= mean { 1 } else { -1 },
            entry_price: current.close,
            block_low: window_low,
            block_high: window_high,
            reason: format!(
                "line crosses={} range={:.2}% span={:.2}% mean={:.4}",
                crosses, range_pct, span_pct, mean
            ),
        })
    } else {
        None
    }
}

fn summarize_sideway_probe(event: SidewayEvent, candles: &[CandleRow], idx: usize) -> SidewayProbe {
    let entry = event.entry_price.max(f64::EPSILON);
    let block_width_pct = pct_range(event.block_low, event.block_high, entry);
    let mut high = entry;
    let mut low = entry;
    let mut persist_candles = 0usize;
    let mut breakout_candles = 0usize;
    let mut breakout_dir = 0i8;
    let mut breakout_index = None;
    let tolerance = ((entry * 0.0015).max(0.0)).max(0.0);
    for (future_idx, future) in candles.iter().enumerate().skip(idx + 1).take(30) {
        breakout_candles += 1;
        if future.high > high {
            high = future.high;
        }
        if future.low < low {
            low = future.low;
        }
        if future.close >= event.block_low - tolerance
            && future.close <= event.block_high + tolerance
        {
            persist_candles += 1;
        } else {
            breakout_index = Some(future_idx);
            breakout_dir = if future.close > event.block_high {
                1
            } else if future.close < event.block_low {
                -1
            } else {
                0
            };
            break;
        }
    }
    let long_mfe_pct = ((high - entry) / entry) * 100.0;
    let long_mae_pct = ((low - entry) / entry) * 100.0;
    let short_mfe_pct = ((entry - low) / entry) * 100.0;
    let short_mae_pct = ((high - entry) / entry) * 100.0;
    let max_move_pct = long_mfe_pct.max(short_mfe_pct);
    let mut post_breakout_high = entry;
    let mut post_breakout_low = entry;
    if let Some(start_idx) = breakout_index {
        for future in candles.iter().skip(start_idx).take(30) {
            if future.high > post_breakout_high {
                post_breakout_high = future.high;
            }
            if future.low < post_breakout_low {
                post_breakout_low = future.low;
            }
        }
    }
    let post_breakout_long_mfe_pct = ((post_breakout_high - entry) / entry) * 100.0;
    let post_breakout_short_mfe_pct = ((entry - post_breakout_low) / entry) * 100.0;
    let post_breakout_max_move_pct = post_breakout_long_mfe_pct.max(post_breakout_short_mfe_pct);
    let move_gt_1pct_after_breakout = post_breakout_max_move_pct >= 1.0;
    let actual_dir = if long_mfe_pct > short_mfe_pct {
        1
    } else if short_mfe_pct > long_mfe_pct {
        -1
    } else {
        0
    };
    let direction_match = actual_dir != 0 && actual_dir == event.side;
    SidewayProbe {
        event,
        block_width_pct,
        long_mfe_pct,
        long_mae_pct,
        short_mfe_pct,
        short_mae_pct,
        max_move_pct,
        post_breakout_max_move_pct,
        move_gt_1pct_after_breakout,
        actual_dir,
        direction_match,
        sideway_persist_candles: persist_candles,
        sideway_persist_minutes: persist_candles as f64,
        breakout_candles,
        breakout_minutes: breakout_candles as f64,
        breakout_dir,
    }
}

fn percentile(mut values: Vec<f64>, pct: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let clamped = pct.clamp(0.0, 100.0);
    let rank = ((clamped / 100.0) * (values.len() as f64 - 1.0)).round() as usize;
    values[rank]
}

fn print_sideway_summary(label: &str, probes: &[SidewayProbe]) {
    if probes.is_empty() {
        println!("{label}: none");
        return;
    }
    let block_widths: Vec<f64> = probes.iter().map(|p| p.block_width_pct).collect();
    let max_moves: Vec<f64> = probes.iter().map(|p| p.max_move_pct).collect();
    let post_breakout_max_moves: Vec<f64> = probes
        .iter()
        .map(|p| p.post_breakout_max_move_pct)
        .collect();
    let persist_minutes: Vec<f64> = probes.iter().map(|p| p.sideway_persist_minutes).collect();
    let breakout_minutes: Vec<f64> = probes.iter().map(|p| p.breakout_minutes).collect();
    let avg_long_mfe = probes.iter().map(|p| p.long_mfe_pct).sum::<f64>() / probes.len() as f64;
    let avg_long_mae = probes.iter().map(|p| p.long_mae_pct).sum::<f64>() / probes.len() as f64;
    let avg_short_mfe = probes.iter().map(|p| p.short_mfe_pct).sum::<f64>() / probes.len() as f64;
    let avg_short_mae = probes.iter().map(|p| p.short_mae_pct).sum::<f64>() / probes.len() as f64;
    let over_1pct = max_moves
        .iter()
        .filter(|move_pct| **move_pct >= 1.0)
        .count();
    let over_1pct_after_breakout = probes
        .iter()
        .filter(|probe| probe.move_gt_1pct_after_breakout)
        .count();
    let match_count = probes.iter().filter(|p| p.direction_match).count();
    let breakout_up = probes.iter().filter(|p| p.breakout_dir > 0).count();
    let breakout_down = probes.iter().filter(|p| p.breakout_dir < 0).count();
    let avg_persist = persist_minutes.iter().sum::<f64>() / persist_minutes.len() as f64;
    let p90_persist = percentile(persist_minutes.clone(), 90.0);
    let max_persist = persist_minutes
        .iter()
        .copied()
        .fold(0.0_f64, |acc, value| acc.max(value));
    let avg_breakout = breakout_minutes.iter().sum::<f64>() / breakout_minutes.len() as f64;
    let p90_breakout = percentile(breakout_minutes.clone(), 90.0);
    let max_breakout = breakout_minutes
        .iter()
        .copied()
        .fold(0.0_f64, |acc, value| acc.max(value));
    let avg_block_width = block_widths.iter().sum::<f64>() / block_widths.len() as f64;
    let p90_block_width = percentile(block_widths.clone(), 90.0);
    let max_block_width = block_widths
        .iter()
        .copied()
        .fold(0.0_f64, |acc, value| acc.max(value));
    let avg_max_move = max_moves.iter().sum::<f64>() / max_moves.len() as f64;
    let p90_max_move = percentile(max_moves.clone(), 90.0);
    let max_move = max_moves
        .iter()
        .copied()
        .fold(0.0_f64, |acc, value| acc.max(value));
    let avg_post_breakout_max_move =
        post_breakout_max_moves.iter().sum::<f64>() / post_breakout_max_moves.len() as f64;
    let p90_post_breakout_max_move = percentile(post_breakout_max_moves.clone(), 90.0);
    let max_post_breakout_max_move = post_breakout_max_moves
        .iter()
        .copied()
        .fold(0.0_f64, |acc, value| acc.max(value));
    println!(
        "{label}: sideways={} move_gt_1pct={}/{} move_gt_1pct_after_breakout={}/{} direction_match={}/{} breakout_up={} breakout_down={} avg_block_width={:.3}% p90_block_width={:.3}% max_block_width={:.3}% inside_block_minutes_avg={:.1}m inside_block_minutes_p90={:.1}m inside_block_minutes_max={:.1}m minutes_to_breakout_avg={:.1}m minutes_to_breakout_p90={:.1}m minutes_to_breakout_max={:.1}m avg_max_move={:.3}% p90_max_move={:.3}% max_max_move={:.3}% avg_post_breakout_max_move={:.3}% p90_post_breakout_max_move={:.3}% max_post_breakout_max_move={:.3}% avg_long_mfe={:.3}% avg_long_mae={:.3}% avg_short_mfe={:.3}% avg_short_mae={:.3}%",
        probes.len(),
        over_1pct,
        probes.len(),
        over_1pct_after_breakout,
        probes.len(),
        match_count,
        probes.len(),
        breakout_up,
        breakout_down,
        avg_block_width,
        p90_block_width,
        max_block_width,
        avg_persist,
        p90_persist,
        max_persist,
        avg_breakout,
        p90_breakout,
        max_breakout,
        avg_max_move,
        p90_max_move,
        max_move,
        avg_post_breakout_max_move,
        p90_post_breakout_max_move,
        max_post_breakout_max_move,
        avg_long_mfe,
        avg_long_mae,
        avg_short_mfe,
        avg_short_mae
    );
}

fn cascade_window(state: &CascadeState, price: f64) -> String {
    let window = cascade::sample_window(state);
    if window.is_empty() {
        return "[]".to_string();
    }

    let values = window
        .into_iter()
        .map(|size| format!("{:.4}%", block_pct(size, price)))
        .collect::<Vec<_>>();
    format!("[{}]", values.join(", "))
}

fn to_candle(row: &CandleRow) -> Candle {
    Candle {
        open: row.open,
        high: row.high,
        low: row.low,
        close: row.close,
    }
}

fn movement_signal(signal: &CascadeSignal) -> &'static str {
    if signal.samples < 3 || signal.average_size <= 0.0 || signal.dir == 0 {
        return if signal.phase == MovementPhase::Starting {
            "start"
        } else {
            "building"
        };
    }
    match signal.phase {
        MovementPhase::Starting => "start",
        MovementPhase::Reversal => "reverse-ready",
        MovementPhase::Mature if signal.strength >= 1.3 => "reverse-ready",
        MovementPhase::CounterMove if signal.strength <= 0.8 => "safe-continuation",
        MovementPhase::CounterMove => "counter-move",
        MovementPhase::Building => "building",
        MovementPhase::Mature => "mature",
        MovementPhase::Unknown => "building",
    }
}

fn best_fill(candle: &CandleRow, side: i8) -> f64 {
    if side > 0 {
        candle.low
    } else if side < 0 {
        candle.high
    } else {
        candle.close
    }
}

fn open_virtual_trade(
    state: &CascadeState,
    candle: &CandleRow,
    side: i8,
    reason: &str,
    tp_mult: f64,
    sl_mult: f64,
) -> Option<VirtualTrade> {
    let tp_base = cascade::cascade_take_profit_distance(state).unwrap_or(state.avg_size);
    if side == 0 || tp_base <= 0.0 {
        return None;
    }
    let entry_pct = best_fill(candle, side);
    let (tp_pct, sl_pct) = if side > 0 {
        (entry_pct + tp_base * tp_mult, entry_pct - tp_base * sl_mult)
    } else {
        (entry_pct - tp_base * tp_mult, entry_pct + tp_base * sl_mult)
    };
    Some(VirtualTrade {
        side,
        entry_pct,
        tp_pct,
        sl_pct,
        reason: reason.to_string(),
        max_favorable_pct: 0.0,
        max_adverse_pct: 0.0,
    })
}

fn finalize_trade(
    trade: &VirtualTrade,
    exit_pct: f64,
    reason: &str,
    _exit_time_ms: i64,
) -> TradeResult {
    let (pnl_pct, mfe_pct, mae_pct) = if trade.side > 0 {
        (
            ((exit_pct - trade.entry_pct) / trade.entry_pct) * 100.0,
            ((trade.max_favorable_pct) / trade.entry_pct) * 100.0,
            ((trade.max_adverse_pct.abs()) / trade.entry_pct) * 100.0,
        )
    } else {
        (
            ((trade.entry_pct - exit_pct) / trade.entry_pct) * 100.0,
            ((trade.max_favorable_pct) / trade.entry_pct) * 100.0,
            ((trade.max_adverse_pct.abs()) / trade.entry_pct) * 100.0,
        )
    };
    TradeResult {
        side: trade.side,
        pnl_pct,
        mfe_pct,
        mae_pct,
        reason: reason.to_string(),
    }
}

fn summarize_results(results: &[TradeResult]) -> String {
    if results.is_empty() {
        return "no trades".to_string();
    }
    let wins: Vec<&TradeResult> = results.iter().filter(|r| r.pnl_pct > 0.0).collect();
    let losses: Vec<&TradeResult> = results.iter().filter(|r| r.pnl_pct <= 0.0).collect();
    let avg_win = if wins.is_empty() {
        0.0
    } else {
        wins.iter().map(|r| r.pnl_pct).sum::<f64>() / wins.len() as f64
    };
    let avg_loss = if losses.is_empty() {
        0.0
    } else {
        losses.iter().map(|r| r.pnl_pct).sum::<f64>() / losses.len() as f64
    };
    let avg_mfe = results.iter().map(|r| r.mfe_pct).sum::<f64>() / results.len() as f64;
    let avg_mae = results.iter().map(|r| r.mae_pct).sum::<f64>() / results.len() as f64;
    format!(
        "trades={} win_rate={:.1}% avg_win={:.3}% avg_loss={:.3}% avg_mfe={:.3}% avg_mae={:.3}%",
        results.len(),
        wins.len() as f64 / results.len() as f64 * 100.0,
        avg_win,
        avg_loss,
        avg_mfe,
        avg_mae
    )
}

fn parse_candle(item: &Value) -> CandleRow {
    let ind = item.get("indicators").unwrap_or(&Value::Null);
    let candle = ind.get("candle").unwrap_or(&Value::Null);
    let dp = ind.get("dpsignal").unwrap_or(&Value::Null);
    let zz = ind.get("zigzagtrend").unwrap_or(&Value::Null);
    let signal = ind.get("signal").unwrap_or(&Value::Null);
    let volume_50 = ind.get("volume_50").unwrap_or(&Value::Null);

    CandleRow {
        time_ms: get_i64(item, "time"),
        open: f64_or(candle, "Open", "open"),
        high: f64_or(candle, "High", "high"),
        low: f64_or(candle, "Low", "low"),
        close: f64_or(candle, "Close", "close"),
        dp_dir: dir_to_int(dp.get("Direction")),
        zz_dir: dir_to_int(zz.get("Direction")),
        dp_zee: f64_or(dp, "Zee", "zee"),
        dp_zee1: f64_or(dp, "Zee1", "zee1"),
        zz_break_price: f64_or(zz, "BreakPrice", "break_price"),
        gain: f64_or(volume_50, "Gain", "gain"),
        ntps: get_i64(signal, "ntps"),
    }
}

fn confirmed_direction(row: &CandleRow) -> i8 {
    if row.dp_dir != 0 && row.dp_dir == row.zz_dir {
        row.dp_dir
    } else {
        0
    }
}

fn fallback_break_price(row: &CandleRow) -> f64 {
    if row.zz_break_price > 0.0 {
        row.zz_break_price
    } else if row.dp_zee1 > 0.0 {
        row.dp_zee1
    } else if row.dp_zee > 0.0 {
        row.dp_zee
    } else {
        row.close
    }
}

fn refresh_direction_lock(lock: &mut DirectionLock, row: &CandleRow) {
    let confirmed = confirmed_direction(row);
    if confirmed == 0 {
        if lock.dir != 0 {
            lock.reason = "dpzz-freeze";
        }
        return;
    }

    let break_price = fallback_break_price(row);
    if lock.dir == 0 {
        lock.dir = confirmed;
        lock.break_price = break_price;
        lock.reason = "dpzz-lock";
        return;
    }

    if lock.dir == confirmed {
        lock.break_price = break_price;
        lock.reason = "dpzz-hold";
        return;
    }

    let broken = if lock.dir > 0 {
        row.close < lock.break_price || row.low < lock.break_price
    } else {
        row.close > lock.break_price || row.high > lock.break_price
    };

    if broken {
        lock.dir = confirmed;
        lock.break_price = break_price;
        lock.reason = "zigzag-break";
    }
}

fn lock_broken(lock: &DirectionLock, row: &CandleRow) -> bool {
    if lock.dir == 0 || lock.break_price <= 0.0 {
        return false;
    }
    if lock.dir > 0 {
        row.close < lock.break_price || row.low < lock.break_price
    } else {
        row.close > lock.break_price || row.high > lock.break_price
    }
}

fn lock_label(lock: &DirectionLock) -> String {
    if lock.dir == 0 {
        "na".to_string()
    } else {
        let side = if lock.dir > 0 { "LONG" } else { "SHORT" };
        format!("{}:{:.4}:{}", side, lock.break_price, lock.reason)
    }
}

fn lock_break_label(lock: &DirectionLock) -> String {
    if lock.dir == 0 || lock.break_price <= 0.0 {
        "na".to_string()
    } else {
        format!("{:.4}", lock.break_price)
    }
}

fn pending_label(pending: &Option<PendingEntry>) -> String {
    pending
        .as_ref()
        .map(|p| {
            format!(
                "{}:armed={}:{}",
                if p.side > 0 { "LONG" } else { "SHORT" },
                p.armed_tick,
                p.reason
            )
        })
        .unwrap_or_else(|| "na".to_string())
}

fn candidate_entry(lock: &DirectionLock, signal: &CascadeSignal) -> Option<(i8, String)> {
    let candidate = match movement_signal(signal) {
        "reverse-ready" if signal.dir != 0 => Some((-signal.dir, "reverse-entry".to_string())),
        "safe-continuation" if signal.dir != 0 => {
            Some((signal.dir, "continuation-entry".to_string()))
        }
        _ => None,
    }?;

    if lock.dir == 0 || lock.dir != candidate.0 {
        None
    } else {
        Some(candidate)
    }
}

fn fetch_history(symbol_key: &str, limit: usize) -> Result<Vec<CandleRow>, Box<dyn Error>> {
    let url = format!(
        "{}?symbol_key={}&timeframe=60&limit={}",
        BASE_URL,
        encode_symbol_key(symbol_key),
        limit
    );
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL"]);
    if let Some(ip) = resolve_host_ip() {
        cmd.args(["--resolve", &format!("{}:443:{}", BASE_HOST, ip)]);
    }
    let output = cmd.arg(&url).output()?;
    if !output.status.success() {
        return Err(format!("curl fetch failed for {symbol_key}").into());
    }
    let payload: Value = serde_json::from_slice(&output.stdout)?;
    let mut rows = Vec::new();
    if let Some(data) = payload.get("data").and_then(Value::as_array) {
        for item in data {
            rows.push(parse_candle(item));
        }
    }
    rows.sort_by_key(|row| row.time_ms);
    Ok(rows)
}

fn fetch_full_history(
    symbol_key: &str,
    seed_limit: usize,
) -> Result<Vec<CandleRow>, Box<dyn Error>> {
    let mut limit = seed_limit.max(200);
    let mut best = Vec::new();
    while limit <= 5_000 {
        let rows = fetch_history(symbol_key, limit)?;
        if rows.len() > best.len() {
            best = rows;
        }
        if best.len() < limit {
            break;
        }
        limit = (limit * 2).min(5_000);
    }
    Ok(best)
}

fn simulate(
    candles: &[CandleRow],
    tp_mult: f64,
    sl_mult: f64,
) -> (
    Vec<TradeResult>,
    Vec<String>,
    Vec<TradeEvent>,
    Vec<SidewayProbe>,
) {
    let mut state = CascadeState::default();
    let mut traces = Vec::new();
    let mut trades = Vec::new();
    let mut events = Vec::new();
    let mut probes = Vec::new();
    let mut active: Option<VirtualTrade> = None;
    let mut lock = DirectionLock {
        dir: 0,
        break_price: 0.0,
        reason: "na",
    };
    let mut pending: Option<PendingEntry> = None;
    let mut sideway_active_prev = false;

    if candles.len() < 3 {
        return (
            trades,
            vec!["not enough candles".to_string()],
            events,
            probes,
        );
    }

    for (idx, cur) in candles.iter().enumerate() {
        let prev_samples = state.samples;
        let prev_window = cascade_window(&state, cur.close);
        let signal = cascade::update_from_candle(&mut state, &to_candle(cur));
        let phase = signal.phase;
        let signal_kind = movement_signal(&signal);
        let size_pct = block_pct(signal.completed_size, cur.close);
        let avg_pct = block_pct(signal.average_size, cur.close);
        let tp_p90_pct = block_pct(
            cascade::cascade_take_profit_distance(&state).unwrap_or(state.avg_size),
            cur.close,
        );
        traces.push(format!(
            "{} candle={:.4}% dir={} phase={:?} signal={} size={:.4}% avg={:.4}% tp_p90={:.4}% samples={} window={} gain={:.4} ntps={}",
            fmt_time(cur.time_ms),
            candle_pct(cur),
            if signal.dir > 0 { "LONG" } else if signal.dir < 0 { "SHORT" } else { "FLAT" },
            phase,
            signal_kind,
            size_pct,
            avg_pct,
            tp_p90_pct,
            signal.samples,
            cascade_window(&state, cur.close),
            cur.gain,
            cur.ntps
        ));
        if let Some(signal) = detect_sideway(candles, idx) {
            if !sideway_active_prev {
                let probe = summarize_sideway_probe(signal.clone(), candles, idx);
                traces.push(format!(
                    "{} SIDEWAY source={:?} expected={} actual={} match={} persist={}m entry={:.4} block={:.4}-{:.4} long5m[mfe={:.3}% mae={:.3}%] short5m[mfe={:.3}% mae={:.3}%] reason={}",
                    fmt_time(signal.time_ms),
                    signal.source,
                    if signal.side > 0 { "LONG" } else { "SHORT" },
                    if probe.actual_dir > 0 {
                        "LONG"
                    } else if probe.actual_dir < 0 {
                        "SHORT"
                    } else {
                        "FLAT"
                    },
                    if probe.direction_match { "yes" } else { "no" },
                    probe.sideway_persist_candles,
                    signal.entry_price,
                    signal.block_low,
                    signal.block_high,
                    probe.long_mfe_pct,
                    probe.long_mae_pct,
                    probe.short_mfe_pct,
                    probe.short_mae_pct,
                    signal.reason
                ));
                probes.push(probe);
            }
            sideway_active_prev = true;
        } else {
            sideway_active_prev = false;
        }
        refresh_direction_lock(&mut lock, cur);
        traces.push(format!(
            "{} lock={} broken={} break_line={} pending={} dpzz={}/{} break={:.4} confirmed={} reason={}",
            fmt_time(cur.time_ms),
            lock_label(&lock),
            lock_broken(&lock, cur),
            lock_break_label(&lock),
            pending_label(&pending),
            cur.dp_dir,
            cur.zz_dir,
            lock.break_price,
            confirmed_direction(cur),
            lock.reason
        ));
        if state.samples != prev_samples {
            traces.push(format!(
                "{} commit samples={} avg={:.4}% window={} prev_window={} block={:.4}% dir={} phase={:?}",
                fmt_time(cur.time_ms),
                state.samples,
                block_pct(state.avg_size, cur.close),
                cascade_window(&state, cur.close),
                prev_window,
                size_pct,
                if signal.dir > 0 { "LONG" } else if signal.dir < 0 { "SHORT" } else { "FLAT" },
                phase
            ));
        }

        if active.is_none() {
            if let Some((side, reason)) = candidate_entry(&lock, &signal) {
                if let Some(p) = pending.as_ref() {
                    if p.side == side && cur.time_ms > p.armed_tick {
                        active = open_virtual_trade(&state, cur, side, &reason, tp_mult, sl_mult);
                        pending = None;
                    }
                } else {
                    pending = Some(PendingEntry {
                        side,
                        armed_tick: cur.time_ms,
                        reason,
                    });
                }
            } else {
                pending = None;
            }

            if let Some(trade) = active.as_ref() {
                events.push(TradeEvent {
                    kind: "enter",
                    side: if trade.side > 0 { "LONG" } else { "SHORT" },
                    time_ms: cur.time_ms,
                    reason: trade.reason.clone(),
                    pnl_pct: 0.0,
                });
            }
        }

        if let Some(trade) = active.as_mut() {
            if trade.side > 0 {
                trade.max_favorable_pct = trade.max_favorable_pct.max(cur.high - trade.entry_pct);
                trade.max_adverse_pct = trade.max_adverse_pct.min(cur.low - trade.entry_pct);
                if cur.low <= trade.sl_pct {
                    let result = finalize_trade(trade, trade.sl_pct, "sl", cur.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "LONG",
                        time_ms: cur.time_ms,
                        reason: "sl".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    trades.push(result);
                    active = None;
                } else if cur.high >= trade.tp_pct {
                    let result = finalize_trade(trade, trade.tp_pct, "tp", cur.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "LONG",
                        time_ms: cur.time_ms,
                        reason: "tp".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    trades.push(result);
                    active = None;
                }
            } else {
                trade.max_favorable_pct = trade.max_favorable_pct.max(trade.entry_pct - cur.low);
                trade.max_adverse_pct = trade.max_adverse_pct.min(trade.entry_pct - cur.high);
                if cur.high >= trade.sl_pct {
                    let result = finalize_trade(trade, trade.sl_pct, "sl", cur.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "SHORT",
                        time_ms: cur.time_ms,
                        reason: "sl".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    trades.push(result);
                    active = None;
                } else if cur.low <= trade.tp_pct {
                    let result = finalize_trade(trade, trade.tp_pct, "tp", cur.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "SHORT",
                        time_ms: cur.time_ms,
                        reason: "tp".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    trades.push(result);
                    active = None;
                }
            }
        }
        if idx == 0 {
            traces.push(format!(
                "seeded avg={:.4}% dir={}",
                avg_pct,
                if signal.dir > 0 {
                    "LONG"
                } else if signal.dir < 0 {
                    "SHORT"
                } else {
                    "FLAT"
                }
            ));
        }
    }

    if let Some(trade) = active {
        let last = candles.last().unwrap();
        trades.push(finalize_trade(&trade, last.close, "eod", last.time_ms));
    }

    (trades, traces, events, probes)
}

fn print_replay(
    symbol_key: &str,
    candles: &[CandleRow],
    results: &[TradeResult],
    traces: &[String],
    events: &[TradeEvent],
    sideway_probes: &[SidewayProbe],
) {
    println!("Movement replay: {symbol_key}");
    println!("candles={}", candles.len());
    for line in traces {
        println!("{line}");
    }
    for event in events {
        if event.kind == "enter" {
            println!(
                "{} ENTER {} reason={}",
                fmt_time(event.time_ms),
                event.side,
                event.reason
            );
        } else {
            println!(
                "{} EXIT {} pnl={:.3}% reason={}",
                fmt_time(event.time_ms),
                event.side,
                event.pnl_pct,
                event.reason
            );
        }
    }
    if !sideway_probes.is_empty() {
        print_sideway_summary("sideways", sideway_probes);
        let ob: Vec<SidewayProbe> = sideway_probes
            .iter()
            .filter(|p| matches!(p.event.source, SidewaySource::OrderBlock))
            .cloned()
            .collect();
        let lc: Vec<SidewayProbe> = sideway_probes
            .iter()
            .filter(|p| matches!(p.event.source, SidewaySource::LineCross))
            .cloned()
            .collect();
        print_sideway_summary("order_block", &ob);
        print_sideway_summary("line_cross", &lc);
    }
    println!("{}", summarize_results(results));
}

fn live_trace(symbol: &str, duration_s: u64, raw: bool) -> Result<(), Box<dyn Error>> {
    let mut child = Command::new("python3")
        .args([
            "../scripts/sug_ws.py",
            "--symbol",
            symbol,
            "--stream",
            "--raw",
        ])
        .stdout(Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().ok_or("missing python stdout")?;
    let reader = BufReader::new(stdout);
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(duration_s);
    let mut state = CascadeState::default();
    let mut lock = DirectionLock {
        dir: 0,
        break_price: 0.0,
        reason: "na",
    };
    let mut active: Option<VirtualTrade> = None;
    let mut last_row: Option<CandleRow> = None;
    let mut trades: Vec<TradeResult> = Vec::new();
    let mut events: Vec<TradeEvent> = Vec::new();

    while Instant::now() < deadline && !INTERRUPTED.load(Ordering::SeqCst) {
        let Ok(line) = rx.recv_timeout(Duration::from_secs(1)) else {
            continue;
        };
        let frame: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if frame.get("event").and_then(Value::as_str) != Some("Projection") {
            continue;
        }
        if raw {
            println!("{line}");
            continue;
        }

        let candle = frame.get("candle").unwrap_or(&Value::Null);
        let indicator = frame.get("indicator").unwrap_or(&Value::Null);
        let row = CandleRow {
            time_ms: get_i64(&frame, "time"),
            open: f64_or(candle, "enter", "open"),
            high: f64_or(candle, "max", "high"),
            low: f64_or(candle, "min", "low"),
            close: f64_or(candle, "exit", "close"),
            dp_dir: 0,
            zz_dir: 0,
            dp_zee: 0.0,
            dp_zee1: 0.0,
            zz_break_price: 0.0,
            gain: f64_or(indicator, "ntps", "ntps"),
            ntps: get_i64(indicator, "ntps"),
        };
        let signal = cascade::update_from_candle(&mut state, &to_candle(&row));
        refresh_direction_lock(&mut lock, &row);

        println!(
                "{} dir={} phase={:?} signal={} candle={:.4}% avg={:.4}% tp_p90={:.4}% size={:.4}% samples={} lock={} broken={} break_line={}",
                fmt_time(row.time_ms),
                if signal.dir > 0 { "LONG" } else if signal.dir < 0 { "SHORT" } else { "FLAT" },
                signal.phase,
                movement_signal(&signal),
                candle_pct(&row),
                block_pct(signal.average_size, row.close),
                block_pct(
                    cascade::cascade_take_profit_distance(&state).unwrap_or(state.avg_size),
                    row.close,
                ),
                block_pct(signal.completed_size, row.close),
                cascade_window(&state, row.close),
                lock_label(&lock),
                lock_broken(&lock, &row),
                lock_break_label(&lock)
            );

        if active.is_none() && movement_signal(&signal) == "reverse-ready" && signal.dir != 0 {
            active = open_virtual_trade(&state, &row, -signal.dir, "reverse-entry", 1.0, 0.8);
            if let Some(trade) = active.as_ref() {
                events.push(TradeEvent {
                    kind: "enter",
                    side: if trade.side > 0 { "LONG" } else { "SHORT" },
                    time_ms: row.time_ms,
                    reason: trade.reason.clone(),
                    pnl_pct: 0.0,
                });
                println!(
                    "{} ENTER {} reason={}",
                    fmt_time(row.time_ms),
                    if trade.side > 0 { "LONG" } else { "SHORT" },
                    trade.reason
                );
            }
        }

        if let Some(trade) = active.as_mut() {
            if trade.side > 0 {
                trade.max_favorable_pct = trade.max_favorable_pct.max(row.high - trade.entry_pct);
                trade.max_adverse_pct = trade.max_adverse_pct.min(row.low - trade.entry_pct);
                if row.low <= trade.sl_pct {
                    let result = finalize_trade(trade, trade.sl_pct, "sl", row.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "LONG",
                        time_ms: row.time_ms,
                        reason: "sl".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    println!(
                        "{} EXIT LONG pnl={:.3}% reason=sl",
                        fmt_time(row.time_ms),
                        result.pnl_pct
                    );
                    trades.push(result);
                    active = None;
                } else if row.high >= trade.tp_pct {
                    let result = finalize_trade(trade, trade.tp_pct, "tp", row.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "LONG",
                        time_ms: row.time_ms,
                        reason: "tp".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    println!(
                        "{} EXIT LONG pnl={:.3}% reason=tp",
                        fmt_time(row.time_ms),
                        result.pnl_pct
                    );
                    trades.push(result);
                    active = None;
                }
            } else {
                trade.max_favorable_pct = trade.max_favorable_pct.max(trade.entry_pct - row.low);
                trade.max_adverse_pct = trade.max_adverse_pct.min(trade.entry_pct - row.high);
                if row.high >= trade.sl_pct {
                    let result = finalize_trade(trade, trade.sl_pct, "sl", row.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "SHORT",
                        time_ms: row.time_ms,
                        reason: "sl".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    println!(
                        "{} EXIT SHORT pnl={:.3}% reason=sl",
                        fmt_time(row.time_ms),
                        result.pnl_pct
                    );
                    trades.push(result);
                    active = None;
                } else if row.low <= trade.tp_pct {
                    let result = finalize_trade(trade, trade.tp_pct, "tp", row.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "SHORT",
                        time_ms: row.time_ms,
                        reason: "tp".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    println!(
                        "{} EXIT SHORT pnl={:.3}% reason=tp",
                        fmt_time(row.time_ms),
                        result.pnl_pct
                    );
                    trades.push(result);
                    active = None;
                }
            }
        }

        last_row = Some(row);
    }

    if let Some(active_trade) = active {
        if let Some(last) = last_row {
            let result = finalize_trade(&active_trade, last.close, "interrupt", last.time_ms);
            events.push(TradeEvent {
                kind: "exit",
                side: if active_trade.side > 0 {
                    "LONG"
                } else {
                    "SHORT"
                },
                time_ms: last.time_ms,
                reason: "interrupt".to_string(),
                pnl_pct: result.pnl_pct,
            });
            println!(
                "{} EXIT {} pnl={:.3}% reason=interrupt",
                fmt_time(last.time_ms),
                if active_trade.side > 0 {
                    "LONG"
                } else {
                    "SHORT"
                },
                result.pnl_pct
            );
            trades.push(result);
        }
    }

    if INTERRUPTED.load(Ordering::SeqCst) {
        println!("\nInterrupted.");
    }
    if !trades.is_empty() {
        println!("{}", summarize_results(&trades));
    }
    if !events.is_empty() {
        println!("events:");
        for event in &events {
            if event.kind == "enter" {
                println!(
                    "{} ENTER {} reason={}",
                    fmt_time(event.time_ms),
                    event.side,
                    event.reason
                );
            } else {
                println!(
                    "{} EXIT {} pnl={:.3}% reason={}",
                    fmt_time(event.time_ms),
                    event.side,
                    event.pnl_pct,
                    event.reason
                );
            }
        }
    }

    let _ = child.kill();
    Ok(())
}

fn usage() {
    eprintln!(
        "movement_research --mode replay|live|both|chart [--symbol-key KEY] [--symbols A,B] [--history N] [--full-history] [--ws-seconds N] [--tp-mult X] [--sl-mult X] [--csv FILE] [--raw]"
    );
}

fn main() -> Result<(), Box<dyn Error>> {
    install_sigint_handler();

    let mut symbol_keys: Vec<String> = Vec::new();
    let mut mode = "replay".to_string();
    let mut history: usize = 60;
    let mut full_history = false;
    let mut ws_seconds: u64 = 300;
    let mut tp_mult = 1.0;
    let mut sl_mult = 0.8;
    let mut csv = String::new();
    let mut raw = false;
    let mut symbols = String::new();

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--symbol-key" => {
                if let Some(val) = args.get(i + 1) {
                    symbol_keys.push(val.clone());
                    i += 2;
                    continue;
                }
            }
            "--symbols" => {
                if let Some(val) = args.get(i + 1) {
                    symbols = val.clone();
                    i += 2;
                    continue;
                }
            }
            "--history" => {
                if let Some(val) = args.get(i + 1) {
                    history = val.parse().unwrap_or(history);
                    i += 2;
                    continue;
                }
            }
            "--full-history" => {
                full_history = true;
                i += 1;
                continue;
            }
            "--mode" => {
                if let Some(val) = args.get(i + 1) {
                    mode = val.clone();
                    i += 2;
                    continue;
                }
            }
            "--ws-seconds" => {
                if let Some(val) = args.get(i + 1) {
                    ws_seconds = val.parse().unwrap_or(ws_seconds);
                    i += 2;
                    continue;
                }
            }
            "--tp-mult" => {
                if let Some(val) = args.get(i + 1) {
                    tp_mult = val.parse().unwrap_or(tp_mult);
                    i += 2;
                    continue;
                }
            }
            "--sl-mult" => {
                if let Some(val) = args.get(i + 1) {
                    sl_mult = val.parse().unwrap_or(sl_mult);
                    i += 2;
                    continue;
                }
            }
            "--csv" => {
                if let Some(val) = args.get(i + 1) {
                    csv = val.clone();
                    i += 2;
                    continue;
                }
            }
            "--raw" => {
                raw = true;
                i += 1;
                continue;
            }
            "--help" | "-h" => {
                usage();
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    if !symbols.is_empty() {
        symbol_keys.extend(
            symbols
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        );
    }
    if symbol_keys.is_empty() {
        symbol_keys.push(DEFAULT_SYMBOL_KEY.to_string());
    }

    let mut csv_rows = Vec::new();

    if mode == "replay" || mode == "both" {
        for symbol_key in &symbol_keys {
            let candles = if full_history || mode == "chart" {
                fetch_full_history(symbol_key, history)?
            } else {
                fetch_history(symbol_key, history)?
            };
            if candles.len() < 50 {
                eprintln!(
                    "{symbol_key}: need at least 50 candles, got {}",
                    candles.len()
                );
                continue;
            }
            let (results, traces, events, sideway_probes) = simulate(&candles, tp_mult, sl_mult);
            print_replay(
                symbol_key,
                &candles,
                &results,
                &traces,
                &events,
                &sideway_probes,
            );
            for result in &results {
                csv_rows.push(format!(
                    "{},{},{:.6},{:.6},{:.6},{}",
                    symbol_key,
                    if result.side > 0 { "LONG" } else { "SHORT" },
                    result.pnl_pct,
                    result.mfe_pct,
                    result.mae_pct,
                    result.reason
                ));
            }
        }
        if !csv.is_empty() {
            std::fs::write(
                &csv,
                format!(
                    "symbol,side,pnl_pct,mfe_pct,mae_pct,reason\n{}\n",
                    csv_rows.join("\n")
                ),
            )?;
        }
        if mode == "replay" {
            return Ok(());
        }
    }

    if mode == "chart" {
        for symbol_key in &symbol_keys {
            let candles = fetch_full_history(symbol_key, history)?;
            if candles.len() < 50 {
                eprintln!(
                    "{symbol_key}: need at least 50 candles, got {}",
                    candles.len()
                );
                continue;
            }
            let (results, traces, events, sideway_probes) = simulate(&candles, tp_mult, sl_mult);
            println!("Chart history: {symbol_key}");
            println!("candles={}", candles.len());
            for line in traces {
                println!("{line}");
            }
            if !sideway_probes.is_empty() {
                print_sideway_summary("sideways", &sideway_probes);
            }
            println!("{}", summarize_results(&results));
            if !events.is_empty() {
                println!("events={}", events.len());
            }
        }
        return Ok(());
    }

    if mode == "live" || mode == "both" {
        for symbol_key in &symbol_keys {
            let symbol = symbol_key.split('#').nth(1).unwrap_or(symbol_key);
            live_trace(symbol, ws_seconds, raw)?;
        }
    }

    Ok(())
}
