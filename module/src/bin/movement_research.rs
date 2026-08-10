#![allow(dead_code)]

use std::error::Error;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use long_scalping::detection::cascade::{self, CascadeSignal, MovementPhase};
use long_scalping::state::CascadeState;

mod research;

use research::data::{
    f64_or, fetch_candle_history, fetch_full_history, fetch_history, get_i64, CandleRow,
};
use research::movement::{
    block_pct, candle_pct, cascade_window, movement_signal, pct_range, to_candle,
};
use research::virtual_trade::{
    setup_from_candle, TradeEvent, VirtualTradeResult, VirtualTradeSimulator, VirtualTradeSummary,
};

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
struct CascadeChartPoint {
    candle: CandleRow,
    dir: i8,
    phase: MovementPhase,
    signal: &'static str,
    movement_low: f64,
    movement_high: f64,
    average_size: f64,
    strength: f64,
    previous_dist3_pct: Option<f64>,
    previous_dist5_pct: Option<f64>,
    previous_dist8_pct: Option<f64>,
}

#[derive(Debug, Clone)]
struct ChartBreakLevel {
    price: f64,
    mid_index: usize,
}

#[derive(Debug, Clone)]
struct ChartCascadeBlock {
    start_index: usize,
    end_index: usize,
    high: f64,
    low: f64,
    dir: i8,
    break_levels: Vec<ChartBreakLevel>,
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

fn chart_points(candles: &[CandleRow]) -> Vec<CascadeChartPoint> {
    let mut state = CascadeState::default();
    candles
        .iter()
        .map(|candle| {
            let signal = cascade::update_from_candle(&mut state, &to_candle(candle));
            CascadeChartPoint {
                candle: candle.clone(),
                dir: signal.dir,
                phase: signal.phase,
                signal: movement_signal(&signal),
                movement_low: state.cas_low,
                movement_high: state.cas_high,
                average_size: signal.average_size,
                strength: signal.strength,
                previous_dist3_pct: cascade::recent_signed_total_size_percent(
                    &state,
                    3,
                    candle.close,
                ),
                previous_dist5_pct: cascade::recent_signed_total_size_percent(
                    &state,
                    5,
                    candle.close,
                ),
                previous_dist8_pct: cascade::recent_signed_total_size_percent(
                    &state,
                    8,
                    candle.close,
                ),
            }
        })
        .collect()
}

fn chart_cascade_blocks(candles: &[CandleRow]) -> Vec<ChartCascadeBlock> {
    let mut raw = Vec::<ChartCascadeBlock>::new();
    for (idx, candle) in candles.iter().enumerate() {
        let dir = candle_dir(candle);
        if dir == 0 {
            continue;
        }
        if let Some(current) = raw.last_mut() {
            if current.dir == dir {
                current.high = current.high.max(candle.high);
                current.low = current.low.min(candle.low);
                current.end_index = idx;
                continue;
            }
        }
        raw.push(ChartCascadeBlock {
            start_index: idx,
            end_index: idx,
            high: candle.high,
            low: candle.low,
            dir,
            break_levels: Vec::new(),
        });
    }

    if raw.len() < 3 {
        return raw;
    }

    let avg_raw = raw.iter().map(|block| block.high - block.low).sum::<f64>() / raw.len() as f64;
    let mut working = raw;
    let mut changed = true;
    while changed {
        changed = false;
        let mut next: Vec<ChartCascadeBlock> = Vec::with_capacity(working.len());
        let mut idx = 0;
        while idx < working.len() {
            let prev = next.last().cloned();
            let cur = working[idx].clone();
            let following = working.get(idx + 1).cloned();
            let enough_samples = idx >= 10;
            if let (Some(prev), Some(following)) = (prev, following) {
                if prev.dir == following.dir && cur.dir != prev.dir && enough_samples {
                    let counter_size = cur.high - cur.low;
                    let neighbour_avg =
                        ((prev.high - prev.low) + (following.high - following.low)) / 2.0;
                    let threshold = neighbour_avg.min(avg_raw) * 0.40;
                    if counter_size <= threshold {
                        let break_price = if cur.dir < 0 { cur.low } else { cur.high };
                        let mut merged = prev;
                        merged.end_index = following.end_index;
                        merged.high = merged.high.max(cur.high).max(following.high);
                        merged.low = merged.low.min(cur.low).min(following.low);
                        merged.break_levels.push(ChartBreakLevel {
                            price: break_price,
                            mid_index: (cur.start_index + cur.end_index) / 2,
                        });
                        merged.break_levels.extend(following.break_levels);
                        next.pop();
                        next.push(merged);
                        idx += 2;
                        changed = true;
                        continue;
                    }
                }
            }
            next.push(cur);
            idx += 1;
        }
        working = next;
    }
    working
}

fn chart_cascade_blocks_from_core(candles: &[CandleRow]) -> Vec<ChartCascadeBlock> {
    let mut state = CascadeState::default();
    let mut blocks = Vec::new();
    let mut active_start = None;

    for (idx, candle) in candles.iter().enumerate() {
        let before = state;
        let signal = cascade::update_from_candle(&mut state, &to_candle(candle));
        if active_start.is_none() && state.cas_dir != 0 {
            active_start = Some(idx);
        }
        if signal.phase == MovementPhase::Reversal {
            if let Some(start_index) = active_start {
                let break_levels = (0..before.break_n)
                    .map(|break_index| ChartBreakLevel {
                        price: before.breaks[break_index],
                        mid_index: (start_index + idx.saturating_sub(1)) / 2,
                    })
                    .collect();
                blocks.push(ChartCascadeBlock {
                    start_index,
                    end_index: idx.saturating_sub(1),
                    high: before.cas_high,
                    low: before.cas_low,
                    dir: before.cas_dir,
                    break_levels,
                });
            }
            active_start = Some(idx);
        }
    }

    if let Some(start_index) = active_start {
        let break_levels = (0..state.break_n)
            .map(|break_index| ChartBreakLevel {
                price: state.breaks[break_index],
                mid_index: (start_index + candles.len().saturating_sub(1)) / 2,
            })
            .collect();
        blocks.push(ChartCascadeBlock {
            start_index,
            end_index: candles.len().saturating_sub(1),
            high: state.cas_high,
            low: state.cas_low,
            dir: state.cas_dir,
            break_levels,
        });
    }
    blocks
}

fn svg_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn chart_previous_average_percent(
    blocks: &[ChartCascadeBlock],
    current_index: usize,
    count: usize,
) -> Option<f64> {
    if count == 0 || current_index < count {
        return None;
    }
    let start = current_index - count;
    let total = blocks[start..current_index]
        .iter()
        .filter(|block| block.low > 0.0)
        .map(|block| (block.high - block.low) / block.low * 100.0)
        .sum::<f64>();
    Some(total / count as f64)
}

fn render_cascade_svg(
    symbol_key: &str,
    points: &[CascadeChartPoint],
    output: &str,
) -> Result<(), Box<dyn Error>> {
    if points.is_empty() {
        return Err("cannot render an empty cascade chart".into());
    }

    let width = 1600.0;
    let height = 900.0;
    let left = 76.0;
    let right = 28.0;
    let top = 68.0;
    let price_bottom = 610.0;
    let distance_top = 690.0;
    let distance_bottom = 845.0;
    let plot_width = width - left - right;
    let price_min = points
        .iter()
        .map(|p| p.candle.low)
        .fold(f64::INFINITY, f64::min);
    let price_max = points
        .iter()
        .map(|p| p.candle.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let price_pad = ((price_max - price_min) * 0.06).max(price_max * 0.0005);
    let min_price = price_min - price_pad;
    let max_price = price_max + price_pad;
    let x = |idx: usize| left + idx as f64 * plot_width / points.len().max(1) as f64;
    let y_price = |price: f64| {
        price_bottom - (price - min_price) / (max_price - min_price) * (price_bottom - top)
    };
    let candle_width = (plot_width / points.len() as f64 * 0.68).max(1.0);
    let candles: Vec<CandleRow> = points.iter().map(|point| point.candle.clone()).collect();
    let blocks = chart_cascade_blocks_from_core(&candles);

    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">\n<style>text{{font-family:monospace;font-size:13px;fill:#c9d1d9}} .grid{{stroke:#30363d;stroke-width:1}} .wick{{stroke-width:1}} .label{{font-size:15px;font-weight:bold}} </style>\n<rect width=\"100%\" height=\"100%\" fill=\"#0d1117\"/>\n<text x=\"{left}\" y=\"30\" class=\"label\">Cascade chart: {symbol}</text>\n<text x=\"{left}\" y=\"50\">last {count} candles · 1m · production cascade state replay</text>\n",
        symbol = svg_escape(symbol_key),
        count = points.len(),
    );

    for step in 0..=5 {
        let price = min_price + (max_price - min_price) * step as f64 / 5.0;
        let y = y_price(price);
        svg.push_str(&format!("<line x1=\"{left}\" y1=\"{y:.2}\" x2=\"{}\" y2=\"{y:.2}\" class=\"grid\"/><text x=\"8\" y=\"{:.2}\">{price:.4}</text>\n", width - right, y + 4.0));
    }
    for (idx, point) in points.iter().enumerate() {
        let cx = x(idx) + plot_width / points.len() as f64 / 2.0;
        let open = y_price(point.candle.open);
        let close = y_price(point.candle.close);
        let high = y_price(point.candle.high);
        let low = y_price(point.candle.low);
        let color = if point.candle.close >= point.candle.open {
            "#3fb950"
        } else {
            "#f85149"
        };
        let rect_y = open.min(close);
        let rect_h = (open - close).abs().max(1.0);
        svg.push_str(&format!("<line x1=\"{cx:.2}\" y1=\"{high:.2}\" x2=\"{cx:.2}\" y2=\"{low:.2}\" stroke=\"{color}\" class=\"wick\"/><rect x=\"{:.2}\" y=\"{rect_y:.2}\" width=\"{candle_width:.2}\" height=\"{rect_h:.2}\" fill=\"{color}\"/>\n", cx - candle_width / 2.0));
    }

    let historical = blocks.len().saturating_sub(2);
    let avg_size = if historical > 0 {
        blocks[..historical]
            .iter()
            .map(|block| block.high - block.low)
            .sum::<f64>()
            / historical as f64
    } else {
        0.0
    };
    for (block_index, block) in blocks.iter().enumerate() {
        let significant = block_index + 2 >= blocks.len()
            || (avg_size > 0.0 && block.high - block.low >= avg_size * 0.6);
        if !significant {
            continue;
        }
        let x_start = x(block.start_index);
        let x_end = x(block.end_index + 1);
        let y_top = y_price(block.high);
        let y_bottom = y_price(block.low);
        let height = (y_bottom - y_top).abs();
        let is_long = block.dir > 0;
        let fill = if is_long { "#00c850" } else { "#dc3232" };
        let border = if is_long { "#00e664" } else { "#ff5050" };
        svg.push_str(&format!(
            "<rect x=\"{x_start:.2}\" y=\"{y_top:.2}\" width=\"{:.2}\" height=\"{height:.2}\" fill=\"{fill}\" fill-opacity=\"0.20\" stroke=\"{border}\" stroke-opacity=\"0.70\"/>\n",
            (x_end - x_start).max(candle_width)
        ));
        for break_level in &block.break_levels {
            let y = y_price(break_level.price);
            if y >= y_top && y <= y_bottom {
                svg.push_str(&format!(
                    "<line x1=\"{x_start:.2}\" y1=\"{y:.2}\" x2=\"{x_end:.2}\" y2=\"{y:.2}\" stroke=\"{border}\" stroke-width=\"1\" stroke-dasharray=\"4 3\"/><circle cx=\"{:.2}\" cy=\"{y:.2}\" r=\"2\" fill=\"{border}\"/>\n",
                    x(break_level.mid_index) + candle_width / 2.0
                ));
            }
        }
        let pct = (block.high - block.low) / block.low * 100.0;
        let label = if block.break_levels.is_empty() {
            format!("{pct:.1}%")
        } else {
            format!("{pct:.1}% ×{}", block.break_levels.len() + 1)
        };
        let point = points.get(block.start_index);
        let recent3 = point
            .and_then(|point| point.previous_dist3_pct)
            .map(|value| format!("3dist={value:.1}%"))
            .unwrap_or_else(|| "3dist=na".to_string());
        let recent5 = point
            .and_then(|point| point.previous_dist5_pct)
            .map(|value| format!("5dist={value:.1}%"))
            .unwrap_or_else(|| "5dist=na".to_string());
        let recent8 = point
            .and_then(|point| point.previous_dist8_pct)
            .map(|value| format!("8dist={value:.1}%"))
            .unwrap_or_else(|| "8dist=na".to_string());
        let (label_y, recent3_y, recent5_y, recent8_y) = if is_long {
            (y_top - 5.0, y_top - 20.0, y_top - 35.0, y_top - 50.0)
        } else {
            (
                y_bottom + 14.0,
                y_bottom + 29.0,
                y_bottom + 44.0,
                y_bottom + 59.0,
            )
        };
        svg.push_str(&format!(
            "<text x=\"{:.2}\" y=\"{label_y:.2}\" text-anchor=\"middle\" fill=\"{border}\" font-weight=\"bold\">{label}</text><text x=\"{:.2}\" y=\"{recent3_y:.2}\" text-anchor=\"middle\" fill=\"{border}\" font-size=\"10\">{recent3}</text><text x=\"{:.2}\" y=\"{recent5_y:.2}\" text-anchor=\"middle\" fill=\"{border}\" font-size=\"10\">{recent5}</text><text x=\"{:.2}\" y=\"{recent8_y:.2}\" text-anchor=\"middle\" fill=\"{border}\" font-size=\"10\">{recent8}</text>\n",
            (x_start + x_end) / 2.0,
            (x_start + x_end) / 2.0,
            (x_start + x_end) / 2.0,
            (x_start + x_end) / 2.0,
        ));
    }
    let distance_values = [
        (
            "3dist",
            "#58a6ff",
            points
                .iter()
                .map(|point| point.previous_dist3_pct)
                .collect::<Vec<_>>(),
        ),
        (
            "5dist",
            "#d29922",
            points
                .iter()
                .map(|point| point.previous_dist5_pct)
                .collect::<Vec<_>>(),
        ),
        (
            "8dist",
            "#bc8cff",
            points
                .iter()
                .map(|point| point.previous_dist8_pct)
                .collect::<Vec<_>>(),
        ),
    ];
    let max_distance = distance_values
        .iter()
        .flat_map(|(_, _, values)| values.iter().flatten())
        .map(|value| value.abs())
        .fold(0.1_f64, f64::max);
    let zero_y = (distance_top + distance_bottom) / 2.0;
    let y_distance = |value: f64| {
        zero_y
            - value.clamp(-max_distance, max_distance) / max_distance
                * (distance_bottom - distance_top)
                / 2.0
    };
    svg.push_str(&format!(
        "<line x1=\"{left}\" y1=\"{zero_y:.2}\" x2=\"{}\" y2=\"{zero_y:.2}\" class=\"grid\"/><text x=\"8\" y=\"{:.2}\">0.0%</text><text x=\"{left}\" y=\"{:.2}\" class=\"label\">Signed completed-move distance</text>\n",
        width - right,
        zero_y + 4.0,
        distance_top - 12.0,
    ));
    for (name, color, values) in distance_values {
        let mut path = String::new();
        let mut connected = false;
        for (idx, value) in values.iter().enumerate() {
            let Some(value) = value else {
                connected = false;
                continue;
            };
            let cx = x(idx) + plot_width / points.len() as f64 / 2.0;
            path.push_str(&format!(
                "{} {:.2},{:.2}",
                if connected { "L" } else { "M" },
                cx,
                y_distance(*value)
            ));
            connected = true;
        }
        if !path.is_empty() {
            let offset = match name {
                "3dist" => 0.0,
                "5dist" => 72.0,
                _ => 144.0,
            };
            svg.push_str(&format!(
                "<path d=\"{path}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"2\"/><text x=\"{}\" y=\"{:.2}\" fill=\"{color}\">{name}</text>\n",
                width - right - offset,
                distance_top - 12.0,
            ));
        }
    }
    svg.push_str(&format!("<line x1=\"{left}\" y1=\"{price_bottom}\" x2=\"{}\" y2=\"{price_bottom}\" class=\"grid\"/>\n", width - right));
    svg.push_str("<text x=\"80\" y=\"650\">green/red: cascade blocks · dashed lines: absorbed break levels · labels: block range and signed completed distance</text>\n</svg>\n");
    std::fs::write(output, svg)?;
    Ok(())
}

fn confirmed_direction(row: &CandleRow) -> i8 {
    row.zz_dir
}

fn fallback_break_price(row: &CandleRow) -> f64 {
    if row.zz_break_price > 0.0 {
        row.zz_break_price
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

fn simulate(
    candles: &[CandleRow],
    tp_mult: f64,
    sl_mult: f64,
) -> (
    Vec<VirtualTradeResult>,
    Vec<String>,
    Vec<TradeEvent>,
    Vec<SidewayProbe>,
) {
    let mut state = CascadeState::default();
    let mut traces = Vec::new();
    let mut trades = Vec::new();
    let mut events = Vec::new();
    let mut probes = Vec::new();
    let mut simulator = VirtualTradeSimulator::default();
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
        let last3_pct =
            cascade::recent_signed_total_size_percent(&state, 3, cur.close).unwrap_or(0.0);
        let last5_pct =
            cascade::recent_signed_total_size_percent(&state, 5, cur.close).unwrap_or(0.0);
        traces.push(format!(
            "{} candle={:.4}% dir={} phase={:?} signal={} size={:.4}% avg={:.4}% last3dist={:.4}% last5dist={:.4}% tp_p90={:.4}% samples={} window={} gain={:.4} ntps={}",
            fmt_time(cur.time_ms),
            candle_pct(cur),
            if signal.dir > 0 { "LONG" } else if signal.dir < 0 { "SHORT" } else { "FLAT" },
            phase,
            signal_kind,
            size_pct,
            avg_pct,
            last3_pct,
            last5_pct,
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
            "{} lock={} broken={} break_line={} pending={} zz={} break={:.4} confirmed={} reason={}",
            fmt_time(cur.time_ms),
            lock_label(&lock),
            lock_broken(&lock, cur),
            lock_break_label(&lock),
            pending_label(&pending),
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

        if simulator.active().is_none() {
            if let Some((side, reason)) = candidate_entry(&lock, &signal) {
                if let Some(p) = pending.as_ref() {
                    if p.side == side && cur.time_ms > p.armed_tick {
                        let tp_base =
                            cascade::cascade_take_profit_distance(&state).unwrap_or(state.avg_size);
                        if let Some(setup) =
                            setup_from_candle(cur, side, &reason, tp_base, tp_mult, sl_mult)
                        {
                            simulator.open(setup);
                        }
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

            if let Some(trade) = simulator.active() {
                events.push(TradeEvent {
                    kind: "enter",
                    side: if trade.side > 0 { "LONG" } else { "SHORT" },
                    time_ms: cur.time_ms,
                    reason: trade.reason.clone(),
                    pnl_pct: 0.0,
                });
            }
        }

        let completed_before = simulator.completed_count();
        simulator.update(cur);
        if simulator.completed_count() > completed_before {
            if let Some(result) = simulator.last_completed() {
                events.push(TradeEvent {
                    kind: "exit",
                    side: if result.side > 0 { "LONG" } else { "SHORT" },
                    time_ms: result.closed_at,
                    reason: result.exit_reason.clone(),
                    pnl_pct: result.pnl_pct,
                });
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

    simulator.finish(candles.last().unwrap());
    trades = simulator.completed();

    (trades, traces, events, probes)
}

fn print_replay(
    symbol_key: &str,
    candles: &[CandleRow],
    results: &[VirtualTradeResult],
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
    println!("{}", format_trade_summary(results));
}

fn format_trade_summary(results: &[VirtualTradeResult]) -> String {
    if results.is_empty() {
        return "no trades".to_string();
    }
    let summary = VirtualTradeSummary::from_trades(results);
    format!(
        "trades={} win_rate={:.1}% avg_win={:.3}% avg_loss={:.3}% avg_mfe={:.3}% avg_mae={:.3}%",
        summary.trades,
        summary.win_rate_pct,
        summary.average_win_pnl_pct,
        summary.average_loss_pnl_pct,
        summary.average_mfe_pct,
        summary.average_mae_pct,
    )
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
    let mut simulator = VirtualTradeSimulator::default();
    let mut last_row: Option<CandleRow> = None;
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
            zz_dir: 0,
            zz_break_price: 0.0,
            gain: f64_or(indicator, "ntps", "ntps"),
            buy_sell_gain: 0.0,
            ntps: get_i64(indicator, "ntps"),
            mrc: Default::default(),
        };
        let signal = cascade::update_from_candle(&mut state, &to_candle(&row));
        refresh_direction_lock(&mut lock, &row);
        let last3_pct =
            cascade::recent_signed_total_size_percent(&state, 3, row.close).unwrap_or(0.0);
        let last5_pct =
            cascade::recent_signed_total_size_percent(&state, 5, row.close).unwrap_or(0.0);

        println!(
                "{} dir={} phase={:?} signal={} candle={:.4}% avg={:.4}% last3dist={:.4}% last5dist={:.4}% tp_p90={:.4}% size={:.4}% samples={} lock={} broken={} break_line={}",
                fmt_time(row.time_ms),
                if signal.dir > 0 { "LONG" } else if signal.dir < 0 { "SHORT" } else { "FLAT" },
                signal.phase,
                movement_signal(&signal),
                candle_pct(&row),
                block_pct(signal.average_size, row.close),
                last3_pct,
                last5_pct,
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

        if simulator.active().is_none()
            && movement_signal(&signal) == "reverse-ready"
            && signal.dir != 0
        {
            let tp_base = cascade::cascade_take_profit_distance(&state).unwrap_or(state.avg_size);
            if let Some(setup) =
                setup_from_candle(&row, -signal.dir, "reverse-entry", tp_base, 1.0, 0.8)
            {
                simulator.open(setup);
            }
            if let Some(trade) = simulator.active() {
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

        let completed_before = simulator.completed_count();
        simulator.update(&row);
        if simulator.completed_count() > completed_before {
            if let Some(result) = simulator.last_completed() {
                events.push(TradeEvent {
                    kind: "exit",
                    side: if result.side > 0 { "LONG" } else { "SHORT" },
                    time_ms: result.closed_at,
                    reason: result.exit_reason.clone(),
                    pnl_pct: result.pnl_pct,
                });
                println!(
                    "{} EXIT {} pnl={:.3}% reason={}",
                    fmt_time(result.closed_at),
                    if result.side > 0 { "LONG" } else { "SHORT" },
                    result.pnl_pct,
                    result.exit_reason,
                );
            }
        }

        last_row = Some(row);
    }

    if simulator.active().is_some() {
        if let Some(last) = last_row {
            simulator.close_active(last.close, last.time_ms, "interrupt", "market");
            if let Some(result) = simulator.last_completed() {
                events.push(TradeEvent {
                    kind: "exit",
                    side: if result.side > 0 { "LONG" } else { "SHORT" },
                    time_ms: result.closed_at,
                    reason: result.exit_reason.clone(),
                    pnl_pct: result.pnl_pct,
                });
                println!(
                    "{} EXIT {} pnl={:.3}% reason=interrupt",
                    fmt_time(result.closed_at),
                    if result.side > 0 { "LONG" } else { "SHORT" },
                    result.pnl_pct
                );
            }
        }
    }

    if INTERRUPTED.load(Ordering::SeqCst) {
        println!("\nInterrupted.");
    }
    let trades = simulator.completed();
    if !trades.is_empty() {
        println!("{}", format_trade_summary(&trades));
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
        "movement_research --mode replay|live|both|chart [--symbol-key KEY] [--symbols A,B] [--history N] [--full-history] [--output FILE] [--ws-seconds N] [--tp-mult X] [--sl-mult X] [--csv FILE] [--raw]"
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
    let mut output = String::new();
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
            "--output" => {
                if let Some(val) = args.get(i + 1) {
                    output = val.clone();
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
                    result.exit_reason
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
            // Chart mode intentionally requests exactly the visible window. This keeps
            // the command fast and makes "last N candles" deterministic.
            let candles = fetch_candle_history(symbol_key, history)?;
            if candles.len() < 50 {
                eprintln!(
                    "{symbol_key}: need at least 50 candles, got {}",
                    candles.len()
                );
                continue;
            }
            let all_points = chart_points(&candles);
            let start = all_points.len().saturating_sub(history);
            let points = &all_points[start..];
            let chart_output = if output.is_empty() {
                let stem = symbol_key
                    .split('#')
                    .nth(1)
                    .unwrap_or(symbol_key)
                    .replace("-USDT-SWAP", "")
                    .replace('/', "_");
                format!("cascade-{stem}-{}.svg", points.len())
            } else {
                output.clone()
            };
            render_cascade_svg(symbol_key, points, &chart_output)?;
            println!("Cascade chart: {symbol_key}");
            println!("candles={} output={chart_output}", points.len());
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
