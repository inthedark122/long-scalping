//! Trend calculator #2 — the EFFICIENCY-RATIO trend, computed from RAW 1m closes only (no indicators).
//!
//! Two dimensions over the last N closes:
//!   MAGNITUDE  = average per-candle move % (how much it moves each candle).
//!   DIRECTION  = Kaufman efficiency ratio = |net move| / total path  (≈1 = one-way trend, ≈0 = choppy).
//! Regime: Quiet (low magnitude) · Chop (high magnitude but mean-reverting) · TrendUp · TrendDown.
//! Quiet & Chop are scalp-friendly (dips revert); only a one-way TrendDown is a real fall.
//! This is one of the inputs to the combined `Trend` (see trend.rs); it also drives the volatility gate
//! (hold-for-TP in Quiet/Chop) which is a MAGNITUDE concern, separate from the combined trend DIRECTION.

use crate::state::State;

const VOL_HIST_N: usize = 10;

/// Update the efficiency-ratio regime on each 1m close. Writes vol_pct (magnitude %), vol_er (efficiency
/// ratio 0..1), and vol_regime (0=Quiet · 1=Chop · 2=TrendUp · -2=TrendDown).
pub(crate) fn update_efficiency_trend(state: &mut State, price: f64) {
    if price <= 0.0 {
        return;
    }
    state.close_hist[state.close_hist_pos] = price;
    state.close_hist_pos = (state.close_hist_pos + 1) % VOL_HIST_N;
    if state.close_hist_len < VOL_HIST_N {
        state.close_hist_len += 1;
    }
    let n = state.close_hist_len;
    if n < 3 {
        state.vol_regime = 0; // not enough history → treat as Quiet
        return;
    }
    // Walk the ring oldest→newest; sum the path and capture first/last.
    let mut path = 0.0;
    let mut prev = 0.0;
    let mut first = 0.0;
    for i in 0..n {
        let idx = (state.close_hist_pos + VOL_HIST_N - n + i) % VOL_HIST_N;
        let c = state.close_hist[idx];
        if i == 0 {
            first = c;
        } else {
            path += (c - prev).abs();
        }
        prev = c;
    }
    let last = prev;
    let net = (last - first).abs();
    let avg_move = path / (n as f64 - 1.0);
    state.vol_pct = avg_move / price * 100.0; // % per candle
    state.vol_er = if path > 0.0 { net / path } else { 0.0 };
    state.vol_regime = if state.vol_pct < state.cfg.vol_lowvol_pct {
        0 // Quiet — too slow for a real move
    } else if state.vol_er >= state.cfg.vol_er_trend {
        if last < first {
            -2 // one-way TrendDown
        } else {
            2 // one-way TrendUp
        }
    } else {
        1 // Chop — high magnitude but mean-reverting (good scalp regime)
    };
}

/// Direction this calculator votes for the combined `Trend`: +1 up · -1 down · 0 neutral (Quiet/Chop).
pub(crate) fn efficiency_dir(state: &State) -> i8 {
    match state.vol_regime {
        2 => 1,
        -2 => -1,
        _ => 0,
    }
}
