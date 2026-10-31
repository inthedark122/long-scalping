use crate::detection::mrc_crossing;
use crate::model::EntryReason;
use lte_strategy_bridge::abi::ModuleKline;
use std::collections::VecDeque;

pub const CANDLE_LOOKBACK: usize = 30;
pub const CLOSE_TO_CLOSE_CHANGES: usize = CANDLE_LOOKBACK - 1;
const MIN_RISING_CLOSES: usize = 20;
const MIN_NET_CHANGE_PCT: f64 = 0.4;
const ONE_MINUTE_MS: i64 = 60_000;
const MAX_CANDLE_AGE_MS: u64 = 120_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CandleTrend {
    pub direction: i8,
    pub start_time_ms: u64,
    pub end_time_ms: u64,
    pub start_close: f64,
    pub end_close: f64,
    pub rising_closes: u32,
    pub net_change_pct: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub trend: Option<CandleTrend>,
    pub entry: Option<EntryReason>,
    pub latest_closed_bar_end_ms: Option<u64>,
    pub summary: String,
}

pub fn evaluate(
    rows: Option<&VecDeque<ModuleKline>>,
    now_ms: u64,
    market_price: f64,
    lines: [f64; 9],
    tick_size: f64,
) -> Evaluation {
    let Some(rows) = rows else {
        return Evaluation {
            trend: None,
            entry: None,
            latest_closed_bar_end_ms: None,
            summary: format!(
                "history=0/{} signal=wait:missing-1m-klines",
                CANDLE_LOOKBACK
            ),
        };
    };
    let mut closed_rows = rows
        .iter()
        .filter(|row| row.time.saturating_add(ONE_MINUTE_MS).max(0) as u64 <= now_ms)
        .collect::<Vec<_>>();
    closed_rows.sort_unstable_by_key(|row| row.time);
    closed_rows.dedup_by_key(|row| row.time);
    let latest_closed_bar_end_ms = closed_rows
        .last()
        .map(|row| row.time.saturating_add(ONE_MINUTE_MS).max(0) as u64);
    if closed_rows.len() < CANDLE_LOOKBACK {
        return Evaluation {
            trend: None,
            entry: None,
            latest_closed_bar_end_ms,
            summary: format!(
                "closed_history={}/{} signal=wait:insufficient-1m-klines",
                closed_rows.len(),
                CANDLE_LOOKBACK
            ),
        };
    }

    let recent = closed_rows
        .iter()
        .rev()
        .take(CANDLE_LOOKBACK)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    let valid_closes = recent
        .iter()
        .all(|row| row.close.is_finite() && row.close > 0.0);
    let continuous = recent
        .windows(2)
        .all(|pair| pair[1].time.saturating_sub(pair[0].time) == ONE_MINUTE_MS);
    let rising_closes = recent
        .windows(2)
        .filter(|pair| pair[1].close > pair[0].close)
        .count();
    let falling_closes = recent
        .windows(2)
        .filter(|pair| pair[1].close < pair[0].close)
        .count();
    let first = recent[0];
    let last = recent[CANDLE_LOOKBACK - 1];
    let net_change_pct = (last.close - first.close) / first.close * 100.0;
    let candle_data_age_ms = now_ms.saturating_sub(latest_closed_bar_end_ms.unwrap_or(now_ms));

    if candle_data_age_ms > MAX_CANDLE_AGE_MS {
        return Evaluation {
            trend: None,
            entry: None,
            latest_closed_bar_end_ms,
            summary: format!(
                "history={}/{} candle_age_ms={} signal=wait:stale-1m-klines",
                recent.len(),
                CANDLE_LOOKBACK,
                candle_data_age_ms
            ),
        };
    }

    let direction = if rising_closes >= MIN_RISING_CLOSES && net_change_pct >= MIN_NET_CHANGE_PCT {
        1
    } else if falling_closes >= MIN_RISING_CLOSES && net_change_pct <= -MIN_NET_CHANGE_PCT {
        -1
    } else {
        0
    };
    if !valid_closes || !continuous || direction == 0 {
        return Evaluation {
            trend: None,
            entry: None,
            latest_closed_bar_end_ms,
            summary: format!(
                "history={}/{} higher_closes={}/{} lower_closes={}/{} change={:.4}% continuous={} signal=wait:no-sustained-direction",
                recent.len(),
                CANDLE_LOOKBACK,
                rising_closes,
                CLOSE_TO_CLOSE_CHANGES,
                falling_closes,
                CLOSE_TO_CLOSE_CHANGES,
                net_change_pct,
                continuous
            ),
        };
    }

    let trend = CandleTrend {
        direction,
        start_time_ms: first.time.max(0) as u64,
        end_time_ms: last.time.saturating_add(ONE_MINUTE_MS).max(0) as u64,
        start_close: first.close,
        end_close: last.close,
        rising_closes: rising_closes as u32,
        net_change_pct,
    };

    let entry = build_entry(trend, market_price, lines, tick_size);
    let summary = match entry.as_ref() {
        Some(EntryReason::CandleTrend {
            direction,
            target_line,
            stop_line,
            target_price,
            stop_price,
            ..
        }) => format!(
            "signal={} directional_closes={}/{} window=30m change={:.4}% target={}@{:.8} stop={}@{:.8} ready=true",
            if *direction > 0 { "up" } else { "down" },
            if *direction > 0 {
                trend.rising_closes
            } else {
                (CLOSE_TO_CLOSE_CHANGES as u32).saturating_sub(trend.rising_closes)
            },
            CLOSE_TO_CLOSE_CHANGES,
            trend.net_change_pct,
            mrc_crossing::LINE_LABELS[*target_line as usize],
            target_price,
            mrc_crossing::LINE_LABELS[*stop_line as usize],
            stop_price
        ),
        _ => format!(
            "signal={} directional_closes={}/{} window=30m change={:.4}% ready=false reason={}",
            if direction > 0 { "up" } else { "down" },
            if direction > 0 {
                trend.rising_closes
            } else {
                (CLOSE_TO_CLOSE_CHANGES as u32).saturating_sub(trend.rising_closes)
            },
            CLOSE_TO_CLOSE_CHANGES,
            trend.net_change_pct,
            if !market_price.is_finite() || market_price <= 0.0 {
                "invalid-market-price"
            } else {
                "no-valid-mrc-exit-levels"
            }
        ),
    };

    Evaluation {
        trend: Some(trend),
        entry,
        latest_closed_bar_end_ms,
        summary,
    }
}

fn build_entry(
    trend: CandleTrend,
    market_price: f64,
    lines: [f64; 9],
    tick_size: f64,
) -> Option<EntryReason> {
    if !market_price.is_finite()
        || market_price <= 0.0
        || !lines.iter().all(|line| line.is_finite() && *line > 0.0)
        || !lines.windows(2).all(|pair| pair[0] < pair[1])
    {
        return None;
    }

    let (stop_line, stop_price, target_line, target_price) = match trend.direction {
        1 => {
            let (stop_line, stop_price) = lines
                .iter()
                .copied()
                .enumerate()
                .filter(|(_, line)| *line < market_price)
                .max_by(|left, right| left.1.total_cmp(&right.1))?;
            let risk = market_price - stop_price;
            let (target_line, target_price) = lines
                .iter()
                .copied()
                .enumerate()
                .find(|(_, line)| *line > market_price && *line - market_price >= risk)?;
            (stop_line, stop_price, target_line, target_price)
        }
        -1 => {
            let (stop_line, stop_price) = lines
                .iter()
                .copied()
                .enumerate()
                .filter(|(_, line)| *line > market_price)
                .min_by(|left, right| left.1.total_cmp(&right.1))?;
            let risk = stop_price - market_price;
            let (target_line, target_price) = lines
                .iter()
                .copied()
                .enumerate()
                .rev()
                .find(|(_, line)| *line < market_price && market_price - *line >= risk)?;
            (stop_line, stop_price, target_line, target_price)
        }
        _ => return None,
    };

    let tick_size = (tick_size.is_finite() && tick_size > 0.0).then_some(tick_size);
    if !mrc_crossing::valid_price_levels_for_pattern(
        trend.direction,
        market_price,
        target_price,
        stop_price,
        tick_size,
        None,
    ) {
        return None;
    }

    Some(EntryReason::CandleTrend {
        direction: trend.direction,
        price: market_price,
        target_price,
        stop_price,
        target_line: target_line as u8,
        stop_line: stop_line as u8,
        rising_closes: trend.rising_closes,
        start_close: trend.start_close,
        end_close: trend.end_close,
        net_change_pct: trend.net_change_pct,
        start_time_ms: trend.start_time_ms,
        end_time_ms: trend.end_time_ms,
    })
}
