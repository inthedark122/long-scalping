use super::{catalog::PatternKind, CurrentPattern};
use lte_strategy_bridge::abi::ModuleKline;
use std::collections::VecDeque;
use ta::indicators::ExponentialMovingAverage;
use ta::Next;

pub const PATTERN_BARS: usize = 30;
pub const PREDICTION_HORIZON_BARS: usize = 15;
const ONE_MINUTE_MS: i64 = 60_000;
const PIVOT_RADIUS: usize = 2;
const EMA_PERIOD: usize = 20;
const MIN_TREND_CLOSES: usize = 20;
const MIN_TREND_RETURN_PCT: f64 = 0.4;
const MIN_MEAN_CROSSES: usize = 4;
const MAX_RANGE_DRIFT_RATIO: f64 = 0.25;
const SPIKE_RANGE_PCT: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PivotKind {
    High,
    Low,
}

#[derive(Debug, Clone, Copy)]
struct Pivot {
    kind: PivotKind,
    price: f64,
}

pub fn closed_rows<'a>(
    rows: Option<&'a VecDeque<ModuleKline>>,
    now_ms: u64,
) -> Vec<&'a ModuleKline> {
    let Some(rows) = rows else {
        return Vec::new();
    };
    let mut closed = rows
        .iter()
        .filter(|row| row.time >= 0 && row.time.saturating_add(ONE_MINUTE_MS) as u64 <= now_ms)
        .filter(|row| valid_ohlc(row))
        .collect::<Vec<_>>();
    closed.sort_unstable_by_key(|row| row.time);
    closed.dedup_by_key(|row| row.time);
    closed
}

pub fn current(rows: &[&ModuleKline]) -> Option<CurrentPattern> {
    if rows.len() < PATTERN_BARS {
        return None;
    }
    let window = &rows[rows.len() - PATTERN_BARS..];
    if !contiguous(window) {
        return None;
    }

    let (new_high, new_low, high_extension_pct, low_extension_pct) = breakout(window);
    let (spike_direction, mean_cross_direction, mean_cross_count) = mean_data(window);
    let kind = staircase(window)
        .or_else(|| range_pattern(window, mean_cross_count))
        .or_else(|| slow_trend(window));
    let kind = kind?;
    let direction = if kind == PatternKind::MeanRange {
        mean_cross_direction.unwrap_or_default()
    } else {
        kind.direction()
    };

    Some(CurrentPattern {
        kind,
        direction,
        confirmed_new_high: new_high,
        confirmed_new_low: new_low,
        high_extension_pct,
        low_extension_pct,
        spike_direction,
        mean_cross_direction,
    })
}

fn staircase(window: &[&ModuleKline]) -> Option<PatternKind> {
    let pivots = confirmed_pivots(window);
    let [first, second, third, fourth] = pivots.get(pivots.len().checked_sub(4)?..)? else {
        return None;
    };
    let up = first.kind == PivotKind::Low
        && second.kind == PivotKind::High
        && third.kind == PivotKind::Low
        && fourth.kind == PivotKind::High
        && third.price > first.price
        && fourth.price > second.price;
    if up {
        return Some(PatternKind::UpStaircase);
    }
    let down = first.kind == PivotKind::High
        && second.kind == PivotKind::Low
        && third.kind == PivotKind::High
        && fourth.kind == PivotKind::Low
        && third.price < first.price
        && fourth.price < second.price;
    down.then_some(PatternKind::DownStaircase)
}

fn confirmed_pivots(window: &[&ModuleKline]) -> Vec<Pivot> {
    let mut pivots: Vec<(usize, Pivot)> = Vec::new();
    for index in PIVOT_RADIUS..window.len().saturating_sub(PIVOT_RADIUS) {
        let row = window[index];
        let neighbours = &window[index - PIVOT_RADIUS..=index + PIVOT_RADIUS];
        let is_high = neighbours.iter().all(|other| row.high >= other.high)
            && neighbours.iter().any(|other| row.high > other.high);
        let is_low = neighbours.iter().all(|other| row.low <= other.low)
            && neighbours.iter().any(|other| row.low < other.low);
        let pivot = if is_high {
            Some(Pivot {
                kind: PivotKind::High,
                price: row.high,
            })
        } else if is_low {
            Some(Pivot {
                kind: PivotKind::Low,
                price: row.low,
            })
        } else {
            None
        };
        let Some(pivot) = pivot else {
            continue;
        };
        if let Some((_, previous)) = pivots.last_mut() {
            if previous.kind == pivot.kind {
                let more_extreme = match pivot.kind {
                    PivotKind::High => pivot.price > previous.price,
                    PivotKind::Low => pivot.price < previous.price,
                };
                if more_extreme {
                    *previous = pivot;
                }
                continue;
            }
        }
        pivots.push((index, pivot));
    }
    pivots.into_iter().map(|(_, pivot)| pivot).collect()
}

fn range_pattern(window: &[&ModuleKline], mean_cross_count: usize) -> Option<PatternKind> {
    if mean_cross_count < MIN_MEAN_CROSSES {
        return None;
    }
    let first = window.first()?.close;
    let last = window.last()?.close;
    let high = window
        .iter()
        .map(|row| row.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let low = window
        .iter()
        .map(|row| row.low)
        .fold(f64::INFINITY, f64::min);
    let range = high - low;
    (range > 0.0 && (last - first).abs() / range <= MAX_RANGE_DRIFT_RATIO)
        .then_some(PatternKind::MeanRange)
}

fn slow_trend(window: &[&ModuleKline]) -> Option<PatternKind> {
    let rising = window
        .windows(2)
        .filter(|pair| pair[1].close > pair[0].close)
        .count();
    let falling = window
        .windows(2)
        .filter(|pair| pair[1].close < pair[0].close)
        .count();
    let first = window.first()?.close;
    let last = window.last()?.close;
    let return_pct = (last - first) / first * 100.0;
    if rising >= MIN_TREND_CLOSES && return_pct >= MIN_TREND_RETURN_PCT {
        Some(PatternKind::SlowRise)
    } else if falling >= MIN_TREND_CLOSES && return_pct <= -MIN_TREND_RETURN_PCT {
        Some(PatternKind::SlowFall)
    } else {
        None
    }
}

fn breakout(window: &[&ModuleKline]) -> (bool, bool, f64, f64) {
    let Some(latest) = window.last() else {
        return (false, false, 0.0, 0.0);
    };
    let prior = &window[..window.len() - 1];
    let prior_high = prior
        .iter()
        .map(|row| row.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let prior_low = prior
        .iter()
        .map(|row| row.low)
        .fold(f64::INFINITY, f64::min);
    (
        latest.high > prior_high && latest.close > prior_high,
        latest.low < prior_low && latest.close < prior_low,
        ((latest.high - prior_high) / prior_high * 100.0).max(0.0),
        ((prior_low - latest.low) / prior_low * 100.0).max(0.0),
    )
}

fn mean_data(window: &[&ModuleKline]) -> (Option<i8>, Option<i8>, usize) {
    let Ok(mut ema) = ExponentialMovingAverage::new(EMA_PERIOD) else {
        return (None, None, 0);
    };
    let values = window
        .iter()
        .map(|row| (row.close, ema.next(row.close)))
        .collect::<Vec<_>>();
    let crossings = values
        .windows(2)
        .filter(|pair| (pair[0].0 >= pair[0].1) != (pair[1].0 >= pair[1].1))
        .count();
    let latest_cross = values.windows(2).rev().find_map(|pair| {
        let was_above = pair[0].0 >= pair[0].1;
        let is_above = pair[1].0 >= pair[1].1;
        if !was_above && is_above {
            Some(1)
        } else if was_above && !is_above {
            Some(-1)
        } else {
            None
        }
    });
    let spike_direction = window.last().and_then(|row| {
        let range_pct = (row.high - row.low) / row.close * 100.0;
        if range_pct >= SPIKE_RANGE_PCT && row.close > row.open {
            Some(1)
        } else if range_pct >= SPIKE_RANGE_PCT && row.close < row.open {
            Some(-1)
        } else {
            None
        }
    });
    (spike_direction, latest_cross, crossings)
}

fn contiguous(window: &[&ModuleKline]) -> bool {
    window
        .windows(2)
        .all(|pair| pair[1].time.saturating_sub(pair[0].time) == ONE_MINUTE_MS)
}

fn valid_ohlc(row: &ModuleKline) -> bool {
    [row.open, row.high, row.low, row.close]
        .iter()
        .all(|price| price.is_finite() && *price > 0.0)
        && row.high >= row.open.max(row.close)
        && row.low <= row.open.min(row.close)
}
