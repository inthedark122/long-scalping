use crate::model::{PositionState, TimeframeSnapshot};
use crate::state::EngineState;
use lte_strategy_bridge::abi::ModuleIndicatorValue;
use lte_strategy_bridge::indicator_access::{
    IndicatorField, IndicatorFieldKey, IndicatorHistory, IndicatorKey, TimeframeSec,
};

const LOOKBACK: usize = 10;
const IMPULSE_MULTIPLE: f64 = 1.5;
const BODY_RATIO_MIN: f64 = 0.55;
const RETEST_TOLERANCE_PCT: f64 = 0.0015;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidewaySource {
    OrderBlock,
    LineCross,
}

impl Default for SidewaySource {
    fn default() -> Self {
        Self::OrderBlock
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SidewaySignal {
    pub active: bool,
    pub source: SidewaySource,
    pub side: PositionState,
    pub block_low: f64,
    pub block_high: f64,
    pub displacement_pct: f64,
    pub block_width_pct: f64,
    pub retest_pct: f64,
    pub price_bias: i8,
    pub reason: String,
}

fn read_float(
    history: &IndicatorHistory,
    timeframe: TimeframeSec,
    indicator: IndicatorKey,
    field: IndicatorField,
    offset: usize,
) -> f64 {
    match history.get_prev(
        IndicatorFieldKey {
            timeframe,
            indicator,
            field,
        },
        offset,
    ) {
        Some(ModuleIndicatorValue::Float(value)) => *value,
        Some(ModuleIndicatorValue::String(value)) => value.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn candle_open(history: &IndicatorHistory, offset: usize) -> f64 {
    read_float(
        history,
        TimeframeSec::Tf1m,
        IndicatorKey::Candle,
        IndicatorField::Open,
        offset,
    )
}

fn candle_high(history: &IndicatorHistory, offset: usize) -> f64 {
    read_float(
        history,
        TimeframeSec::Tf1m,
        IndicatorKey::Candle,
        IndicatorField::High,
        offset,
    )
}

fn candle_low(history: &IndicatorHistory, offset: usize) -> f64 {
    read_float(
        history,
        TimeframeSec::Tf1m,
        IndicatorKey::Candle,
        IndicatorField::Low,
        offset,
    )
}

fn candle_close(history: &IndicatorHistory, offset: usize) -> f64 {
    read_float(
        history,
        TimeframeSec::Tf1m,
        IndicatorKey::Candle,
        IndicatorField::Close,
        offset,
    )
}

fn candle_range(history: &IndicatorHistory, offset: usize) -> f64 {
    (candle_high(history, offset) - candle_low(history, offset)).abs()
}

fn candle_body(history: &IndicatorHistory, offset: usize) -> f64 {
    (candle_close(history, offset) - candle_open(history, offset)).abs()
}

fn candle_dir(history: &IndicatorHistory, offset: usize) -> i8 {
    let open = candle_open(history, offset);
    let close = candle_close(history, offset);
    if close > open {
        1
    } else if close < open {
        -1
    } else {
        0
    }
}

fn lookback_available(history: &IndicatorHistory) -> bool {
    candle_close(history, LOOKBACK - 1) > 0.0
}

fn average_range(history: &IndicatorHistory, start: usize, end: usize) -> f64 {
    let mut total = 0.0;
    let mut count = 0.0;
    for offset in start..=end {
        let range = candle_range(history, offset);
        if range > 0.0 {
            total += range;
            count += 1.0;
        }
    }
    if count <= 0.0 {
        0.0
    } else {
        total / count
    }
}

fn price_bias(price: f64, low: f64, high: f64) -> i8 {
    if high <= low || price <= 0.0 {
        return 0;
    }
    let mid = (low + high) / 2.0;
    if price < mid {
        -1
    } else if price > mid {
        1
    } else {
        0
    }
}

fn candle_mid(history: &IndicatorHistory, offset: usize) -> f64 {
    let high = candle_high(history, offset);
    let low = candle_low(history, offset);
    if high <= 0.0 || low <= 0.0 {
        0.0
    } else {
        (high + low) / 2.0
    }
}

fn detect_order_block(state: &EngineState, snapshot: &TimeframeSnapshot) -> SidewaySignal {
    let history = &state.history;
    if !lookback_available(history) {
        return SidewaySignal {
            reason: "ob-warmup".to_string(),
            ..Default::default()
        };
    }

    let current_price = snapshot.candle.close.max(f64::EPSILON);
    let avg_range = average_range(history, 1, 5);
    if avg_range <= 0.0 {
        return SidewaySignal {
            reason: "ob-no-average".to_string(),
            ..Default::default()
        };
    }

    let mut chosen = None;
    for impulse_offset in 0..=4 {
        let impulse_range = candle_range(history, impulse_offset);
        let impulse_body = candle_body(history, impulse_offset);
        let body_ratio = if impulse_range > 0.0 {
            impulse_body / impulse_range
        } else {
            0.0
        };
        if impulse_range < avg_range * IMPULSE_MULTIPLE || body_ratio < BODY_RATIO_MIN {
            continue;
        }

        let direction = candle_dir(history, impulse_offset);
        if direction == 0 {
            continue;
        }

        let mut block_offset = None;
        for older in (impulse_offset + 1)..=(impulse_offset + 3).min(LOOKBACK - 1) {
            let old_dir = candle_dir(history, older);
            if old_dir != 0 && old_dir != direction {
                block_offset = Some(older);
                break;
            }
        }
        let Some(block_offset) = block_offset else {
            continue;
        };

        let block_low = candle_low(history, block_offset);
        let block_high = candle_high(history, block_offset);
        if block_high <= block_low {
            continue;
        }

        let retest_tol = (current_price * RETEST_TOLERANCE_PCT).max(avg_range * 0.10);
        let touched =
            current_price >= block_low - retest_tol && current_price <= block_high + retest_tol;
        let retest_pct = if current_price > 0.0 {
            (((current_price - ((block_low + block_high) / 2.0)).abs()) / current_price) * 100.0
        } else {
            0.0
        };
        let block_width_pct = ((block_high - block_low).abs() / current_price) * 100.0;
        let displacement_pct = (impulse_range / current_price) * 100.0;

        if !touched {
            chosen = Some(SidewaySignal {
                active: false,
                source: SidewaySource::OrderBlock,
                side: if direction > 0 {
                    PositionState::Long
                } else {
                    PositionState::Short
                },
                block_low,
                block_high,
                displacement_pct,
                block_width_pct,
                retest_pct,
                price_bias: price_bias(current_price, block_low, block_high),
                reason: format!(
                    "ob-candidate dir={} impulse={:.2}% body={:.2}% block={:.4}-{:.4} touched=false",
                    direction, displacement_pct, body_ratio, block_low, block_high
                ),
            });
            continue;
        }

        let side = if direction > 0 {
            PositionState::Long
        } else {
            PositionState::Short
        };
        let signal = SidewaySignal {
            active: true,
            source: SidewaySource::OrderBlock,
            side,
            block_low,
            block_high,
            displacement_pct,
            block_width_pct,
            retest_pct,
            price_bias: price_bias(current_price, block_low, block_high),
            reason: format!(
                "ob-active dir={} impulse={:.2}% body={:.2}% block={:.4}-{:.4} retest={:.2}% avg_range={:.4}",
                direction,
                displacement_pct,
                body_ratio,
                block_low,
                block_high,
                retest_pct,
                avg_range
            ),
        };
        return signal;
    }

    chosen.unwrap_or(SidewaySignal {
        reason: "ob-not-found".to_string(),
        ..Default::default()
    })
}

fn detect_line_cross(state: &EngineState, snapshot: &TimeframeSnapshot) -> SidewaySignal {
    let history = &state.history;
    if !lookback_available(history) {
        return SidewaySignal {
            source: SidewaySource::LineCross,
            reason: "line-warmup".to_string(),
            ..Default::default()
        };
    }

    let current_price = snapshot.candle.close.max(f64::EPSILON);
    let mut closes = Vec::new();
    let mut highs = Vec::new();
    let mut lows = Vec::new();
    for offset in 0..4 {
        let close = candle_close(history, offset);
        let high = candle_high(history, offset);
        let low = candle_low(history, offset);
        if close > 0.0 && high > 0.0 && low > 0.0 {
            closes.push(close);
            highs.push(high);
            lows.push(low);
        }
    }

    if closes.len() < 3 {
        return SidewaySignal {
            source: SidewaySource::LineCross,
            reason: "line-no-closes".to_string(),
            ..Default::default()
        };
    }

    let mean = candle_mid(history, 0);
    let mut crosses = 0;
    for pair in closes.windows(2) {
        let a = pair[0] - mean;
        let b = pair[1] - mean;
        if a == 0.0 || b == 0.0 || a.signum() != b.signum() {
            crosses += 1;
        }
    }

    let window_low = lows.iter().copied().fold(f64::INFINITY, f64::min);
    let window_high = highs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let window_span_pct = if current_price > 0.0 {
        ((window_high - window_low).abs() / current_price) * 100.0
    } else {
        0.0
    };
    let current_range_pct = if current_price > 0.0 {
        ((snapshot.candle.high - snapshot.candle.low).abs() / current_price) * 100.0
    } else {
        0.0
    };
    let line_step_pct = if snapshot.mrc.up_inner > 0.0 && snapshot.mrc.down_inner > 0.0 {
        ((snapshot.mrc.up_inner - snapshot.mrc.down_inner).abs() / current_price) * 100.0
    } else {
        0.0
    };
    let compressed = current_range_pct <= (line_step_pct * 1.2).max(0.25);
    let active = crosses >= 2 && compressed && window_span_pct <= (line_step_pct * 2.2).max(0.50);

    SidewaySignal {
        active,
        source: SidewaySource::LineCross,
        side: if closes.last().copied().unwrap_or(current_price) >= mean {
            PositionState::Long
        } else {
            PositionState::Short
        },
        block_low: window_low,
        block_high: window_high,
        displacement_pct: 0.0,
        block_width_pct: window_span_pct,
        retest_pct: if current_price > 0.0 {
            (((current_price - ((window_low + window_high) / 2.0)).abs()) / current_price) * 100.0
        } else {
            0.0
        },
        price_bias: price_bias(current_price, window_low, window_high),
        reason: if active {
            format!(
                "line-active crosses={} range={:.2}% span={:.2}%",
                crosses, current_range_pct, window_span_pct
            )
        } else {
            format!(
                "line-wait crosses={} range={:.2}% span={:.2}%",
                crosses, current_range_pct, window_span_pct
            )
        },
    }
}

pub fn analyze(state: &EngineState, snapshot: &TimeframeSnapshot) -> SidewaySignal {
    let ob = detect_order_block(state, snapshot);
    if ob.active {
        return ob;
    }

    let line = detect_line_cross(state, snapshot);
    if line.active {
        return line;
    }

    line
}

pub fn reentry_ready(signal: &SidewaySignal, side: PositionState) -> bool {
    if !signal.active {
        return false;
    }

    match side {
        PositionState::Long => {
            signal.side == PositionState::Long
                && (signal.price_bias <= 0 || matches!(signal.source, SidewaySource::LineCross))
        }
        PositionState::Short => {
            signal.side == PositionState::Short
                && (signal.price_bias >= 0 || matches!(signal.source, SidewaySource::LineCross))
        }
        PositionState::Flat => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reentry_follows_bias() {
        let signal = SidewaySignal {
            active: true,
            side: PositionState::Long,
            price_bias: -1,
            ..Default::default()
        };
        assert!(reentry_ready(&signal, PositionState::Long));
        assert!(!reentry_ready(&signal, PositionState::Short));
    }
}
