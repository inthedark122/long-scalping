//! Indicator access: typed reads over the raw indicator map, MRC band
//! application, and volume-spike normalization.

use std::collections::{BTreeMap, HashMap};

use crate::consts::*;
use crate::state::State;
use lte_strategy_bridge::abi::{Direction, ModuleIndicatorValue, ModuleInput};
use lte_strategy_bridge::indicator_access::{
    IndicatorField, IndicatorFieldKey, IndicatorKey, TimeframeSec,
};

pub(crate) type Indicators =
    BTreeMap<i64, Vec<HashMap<String, HashMap<String, ModuleIndicatorValue>>>>;

pub(crate) fn apply_mrc_bands(state: &mut State, bands: &[f64; 9]) {
    // Read ML 5 candles ago from history — works on both Init and Indicators
    // (Indicators events only carry 1 candle, so we can't use input.indicators for lookback)
    let key_ml = IndicatorFieldKey {
        timeframe: TimeframeSec::Tf1m,
        indicator: IndicatorKey::Mrc200,
        field: IndicatorField::MeanLine,
    };
    let ml_prev = match state.history.get_prev(key_ml, 5) {
        Some(ModuleIndicatorValue::Float(f)) => *f,
        _ => 0.0,
    };

    state.has_mrc = true;
    state.mrc_db = bands[0];
    state.mrc_do = bands[1];
    state.mrc_ds = bands[2];
    state.mrc_di = bands[3];
    state.mrc_ml = bands[4];
    state.mrc_ui = bands[5];
    state.mrc_us = bands[6];
    state.mrc_uo = bands[7];
    state.mrc_ub = bands[8];
    state.mrc_range = bands[8] - bands[0];
    state.mrc_ml_prev = ml_prev;
    if ml_prev > 0.0 && state.mrc_range > 0.0 {
        state.angle = (bands[4] - ml_prev) / state.mrc_range;
    }
}

pub(crate) fn read_smc_dir(ind: &Indicators) -> i8 {
    match read_direction(ind, TF, 0, SMC_IND, "Direction") {
        Some(Direction::Long) => 1,
        Some(Direction::Short) => -1,
        _ => 0,
    }
}

/// Candles within which a fresh new low/high keeps us in a stair (else the stair has stalled → Normal).
const STAIR_FRESH: u32 = 3;

/// Update the SMC local low/high (the "stairs" anchor) and the 3-stage FC regime. Call on each 1m
/// close. SMC LowPrice/HighPrice are the running local extremes — they step to each new low/high the
/// moment it prints, then hold when the move stalls. fc_stage: -1 DownStairs · 0 Normal · +1 UpStairs.
pub(crate) fn update_smc_levels(ind: &Indicators, state: &mut State) {
    if let Some(low) = read_float(ind, TF, 0, SMC_IND, "LowPrice") {
        if low > 0.0 {
            if state.smc_low > 0.0 && low < state.smc_low - 1e-9 {
                state.smc_low_age = 0; // a NEW lower-low printed
            } else {
                state.smc_low_age = state.smc_low_age.saturating_add(1);
            }
            state.smc_low = low;
        }
    }
    if let Some(high) = read_float(ind, TF, 0, SMC_IND, "HighPrice") {
        if high > 0.0 {
            if state.smc_high > 0.0 && high > state.smc_high + 1e-9 {
                state.smc_high_age = 0; // a NEW higher-high printed
            } else {
                state.smc_high_age = state.smc_high_age.saturating_add(1);
            }
            state.smc_high = high;
        }
    }
    // DownStairs while a new low printed recently (and more recently than a new high); UpStairs the
    // mirror; else the stair has stalled → Normal.
    state.fc_stage = if state.smc_low_age <= STAIR_FRESH && state.smc_low_age <= state.smc_high_age {
        -1
    } else if state.smc_high_age <= STAIR_FRESH && state.smc_high_age < state.smc_low_age {
        1
    } else {
        0
    };
}

/// Read the 1m ZigZagTrend pivots and detect the LINE BREAK (call on each 1m close only — the
/// zigzag is static within a candle). The entry signal is a candle CLOSE that breaks above the
/// swing-high line (SellPrice). Crucially we compare to the PREVIOUS candle's SellPrice (the value
/// held in state before this update): the zigzag bumps SellPrice up the instant the line is broken,
/// so by the time we read the new value it has already run above price — only the previous value
/// shows the break. Direction is the "strong move" state (HH+HL); we just exclude a strong-down leg.
pub(crate) fn update_zigzag(ind: &Indicators, state: &mut State) {
    let new_dir = match read_direction(ind, TF, 0, ZZ_IND, "Direction") {
        Some(Direction::Long) => 1,
        Some(Direction::Short) => -1,
        _ => 0,
    };
    let close = read_float(ind, TF, 0, CANDLE_IND, "Close").unwrap_or(0.0);
    // LINE BREAK: this candle closed above the swing-high line as it stood at the PREVIOUS candle
    // (state.zz_sell, not yet overwritten), and it's a FRESH break (the prior close was at/below the
    // line — not a continuation already above it). Exclude a confirmed strong-down leg.
    state.zz_break_up = state.zz_sell > 0.0
        && state.zz_close_prev > 0.0
        && state.zz_close_prev <= state.zz_sell
        && close > state.zz_sell
        && new_dir != -1;

    if let Some(sell) = read_float(ind, TF, 0, ZZ_IND, "SellPrice") {
        state.zz_sell = sell;
    }
    if let Some(buy) = read_float(ind, TF, 0, ZZ_IND, "BuyPrice") {
        state.zz_buy = buy;
    }
    state.zz_dir = new_dir;
    state.zz_klines = read_float(ind, TF, 0, ZZ_IND, "KLines").unwrap_or(0.0);
    if close > 0.0 {
        state.zz_close_prev = close;
    }
}

/// Detect a SHARP down candle on each 1m close and arm the fast-return tracker. SHARP = a candle that
/// crosses >=2 MRC bands downward, OR breaches below DownBig (out of the channel). The move is measured
/// from the PREVIOUS candle's close (the pre-drop reference) down to THIS candle's low, counting how many
/// band lines lie between them with the current bands. We only trust band-crossings when the MRC channel
/// width is sane (mrc_valid) — degenerate bands on a thin-history symbol (e.g. SPCX at ~100% width) make
/// crossings meaningless. NOTE: there is no indicator for band-crossing — it is computed here from scratch.
pub(crate) fn detect_sharp_move(ind: &Indicators, state: &mut State, price: f64) {
    // Band-validity gate: channel set, ascending, and not absurdly wide vs price.
    let chan = state.mrc_ub - state.mrc_db;
    state.mrc_valid = chan > 0.0 && price > 0.0 && (chan / price) < state.cfg.mrc_max_width;

    let low = read_float(ind, TF, 0, CANDLE_IND, "Low").unwrap_or(0.0);
    let start = state.prev_close; // prior candle close = the pre-drop reference

    if state.cfg.fr_enable && state.mrc_valid && !state.sharp_active && start > 0.0 && low > 0.0 {
        let z_start = state.mrc_zone(start);
        let z_low = state.mrc_zone(low);
        let crossed = z_start - z_low; // > 0 = moved DOWN this many band lines this candle
        // SHARP = crossed >=2 bands, OR started inside the channel and the low pierced below DownBig.
        let breached_db = z_start >= 1 && z_low == 0;
        if crossed >= 2 || breached_db {
            state.sharp_active = true;
            state.sharp_tick = state.ticks;
            state.sharp_ref_px = start; // a "return" = price climbing back to this pre-drop level
            state.sharp_bands = crossed.max(1);
        }
    }

    // Roll the reference forward for the next candle.
    if price > 0.0 {
        state.prev_close = price;
    }
}

/// Resolve an armed sharp-down on each tick: a FAST RETURN = live price climbs back to the pre-drop
/// reference within fr_window candles (the wick reverted — thesis held, reset the fail streak). If the
/// window elapses with no return, it was a real decline (a non-return). fr_consec_fail_n non-returns in a
/// row → risk HIGH (stop new entries + allow the cuts). risk LOW → suppress the cuts, hold for the TP.
pub(crate) fn update_fast_return(price: f64, state: &mut State) {
    if !state.cfg.fr_enable {
        state.risk_high = false;
        state.sharp_active = false;
        return;
    }
    if state.sharp_active {
        let window_ticks = (state.cfg.fr_window.max(1.0) * TICKS_PER_CANDLE as f64) as u64;
        if price > 0.0 && price >= state.sharp_ref_px {
            state.fr_returns = state.fr_returns.saturating_add(1);
            state.fr_consec_fail_n = 0; // a return clears the streak → back to risk LOW
            state.fr_last_fail_px = 0.0;
            state.sharp_active = false;
        } else if state.ticks.saturating_sub(state.sharp_tick) > window_ticks {
            state.fr_fails = state.fr_fails.saturating_add(1);
            state.fr_consec_fail_n = state.fr_consec_fail_n.saturating_add(1);
            state.fr_last_fail_px = state.sharp_ref_px; // breakdown level to recover above
            state.sharp_active = false;
        }
    }
    // Recovery reset: price climbed back above the level where the dips were failing → the decline is
    // over, even if no new sharp-down has armed (calm recovery). Clears the streak → risk LOW.
    if state.fr_last_fail_px > 0.0 && price >= state.fr_last_fail_px {
        state.fr_consec_fail_n = 0;
        state.fr_last_fail_px = 0.0;
    }
    state.risk_high = state.fr_consec_fail_n >= (state.cfg.fr_consec_fail.max(1.0) as u32);
}

const VOL_HIST_N: usize = 10;

/// Classify the volatility REGIME on each 1m close (also Phase-2's stage detector). Two dimensions:
///   MAGNITUDE  = average per-candle move % over the last N closes.
///   DIRECTION  = efficiency ratio = |net move| / total path (≈1 one-way trend, ≈0 choppy flip).
/// Regimes: Quiet (low magnitude) · Chop (high magnitude, flipping) · TrendUp · TrendDown. Quiet & Chop
/// are scalp-friendly (dips revert) → HOLD for the TP; only TrendDown is a real one-way fall → allow cuts.
pub(crate) fn update_vol_regime(state: &mut State, price: f64) {
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

pub(crate) fn update_volume_spike(ind: &Indicators, state: &mut State) {
    let gain = read_float(ind, TF, 0, "volume_50", "Gain").unwrap_or(0.0);
    let gain_min = read_float(ind, TF, 0, "volume_50", "GainMin").unwrap_or(-1.0);
    let gain_max = read_float(ind, TF, 0, "volume_50", "GainMax").unwrap_or(1.0);
    let bsg = read_float(ind, TF, 0, "volume_50", "BuySellGain").unwrap_or(0.0);
    let bsg_min = read_float(ind, TF, 0, "volume_50", "BuySellGainMin").unwrap_or(-1.0);
    let bsg_max = read_float(ind, TF, 0, "volume_50", "BuySellGainMax").unwrap_or(1.0);

    state.vol_gain_prev = state.vol_gain;
    state.vol_gain = gain;
    state.vol_gain_max = gain_max;
    state.vol_gain_min = gain_min;
    state.vol_bsg = bsg;
    state.vol_bsg_max = bsg_max;
    state.vol_bsg_min = bsg_min;

    // Spike detection: normalize within 50-candle range to find outlier candles
    let gain_range = (gain_max - gain_min).max(GAIN_RANGE_FLOOR);
    let bsg_range = (bsg_max - bsg_min).max(BSG_RANGE_FLOOR);
    let gain_pct = (gain - gain_min) / gain_range; // 1.0 = biggest candle in 50c
    let bsg_pct = (bsg - bsg_min) / bsg_range; // 0.0 = max selling, 1.0 = max buying

    // Large candle (top half of 50c range) + strong directional pressure
    state.bearish_spike = gain_pct > 0.50 && bsg_pct < 0.30;
    state.bullish_spike = gain_pct > 0.50 && bsg_pct > 0.70;

    // Priority 1.1: track exhaustion candles (BSG > 2.0 = climax buy; BSG < -2.0 = climax sell)
    if state.bullish_spike && bsg > 2.0 {
        state.bsg_exhaustion_long_candle = state.tf1.candles;
    }
    if state.bearish_spike && bsg < -2.0 {
        state.bsg_exhaustion_short_candle = state.tf1.candles;
    }
}

pub(crate) fn read_float(
    ind: &Indicators,
    tf: i64,
    idx: usize,
    name: &str,
    field: &str,
) -> Option<f64> {
    ind.get(&tf)?
        .get(idx)?
        .get(name)?
        .get(field)
        .and_then(|v| match v {
            ModuleIndicatorValue::Float(f) => Some(*f),
            _ => None,
        })
}

pub(crate) fn read_direction(
    ind: &Indicators,
    tf: i64,
    idx: usize,
    name: &str,
    field: &str,
) -> Option<Direction> {
    ind.get(&tf)?
        .get(idx)?
        .get(name)?
        .get(field)
        .and_then(|v| match v {
            ModuleIndicatorValue::Direction(d) => Some(*d),
            // ModuleIndicatorValue is #[serde(untagged)] with String before Direction,
            // so "LONG"/"SHORT" strings get deserialized as String, not Direction.
            ModuleIndicatorValue::String(s) => match s.as_str() {
                "LONG" => Some(Direction::Long),
                "SHORT" => Some(Direction::Short),
                "BOTH" => Some(Direction::Both),
                _ => None,
            },
            _ => None,
        })
}

pub(crate) fn read_mrc(ind: &Indicators) -> Option<[f64; 9]> {
    read_mrc_at(ind, 0)
}

// Read the 9 MRC bands at a given history offset (0 = latest). Used by the cold-start seeder to
// replay each historical candle against the bands as they were at that candle.
pub(crate) fn read_mrc_at(ind: &Indicators, offset: usize) -> Option<[f64; 9]> {
    Some([
        read_float(ind, TF, offset, MRC, "DownBig")?,
        read_float(ind, TF, offset, MRC, "DownOuter")?,
        read_float(ind, TF, offset, MRC, "DownSmall")?,
        read_float(ind, TF, offset, MRC, "DownInner")?,
        read_float(ind, TF, offset, MRC, "MeanLine")?,
        read_float(ind, TF, offset, MRC, "UpInner")?,
        read_float(ind, TF, offset, MRC, "UpSmall")?,
        read_float(ind, TF, offset, MRC, "UpOuter")?,
        read_float(ind, TF, offset, MRC, "UpBig")?,
    ])
}

pub(crate) fn read_analysis(
    input: &ModuleInput,
) -> (
    i64,
    f64,
    f64,
    Option<lte_strategy_bridge::SuggestionInfoCandle>,
) {
    match &input.sug_info {
        Some(s) => (
            s.indicator.ntps,
            s.indicator.trandm,
            s.indicator.asset_01,
            s.fast_candle.clone(),
        ),
        None => (0, 0.0, 0.0, None), // trandm default neutral (0) for the -100..+100 scale
    }
}
