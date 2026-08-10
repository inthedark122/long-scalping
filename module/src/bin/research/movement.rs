use long_scalping::detection::cascade::{self, CascadeSignal, MovementPhase};
use long_scalping::model::Candle;
use long_scalping::state::CascadeState;

use super::data::CandleRow;

pub(crate) fn pct_range(lo: f64, hi: f64, price: f64) -> f64 {
    if price <= 0.0 {
        0.0
    } else {
        ((hi - lo).abs() / price) * 100.0
    }
}

pub(crate) fn candle_pct(candle: &CandleRow) -> f64 {
    pct_range(candle.low, candle.high, candle.close)
}

pub(crate) fn block_pct(size: f64, price: f64) -> f64 {
    if price <= 0.0 {
        0.0
    } else {
        (size.abs() / price) * 100.0
    }
}

pub(crate) fn cascade_window(state: &CascadeState, price: f64) -> String {
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

pub(crate) fn to_candle(row: &CandleRow) -> Candle {
    Candle {
        open: row.open,
        high: row.high,
        low: row.low,
        close: row.close,
    }
}

pub(crate) fn movement_signal(signal: &CascadeSignal) -> &'static str {
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
