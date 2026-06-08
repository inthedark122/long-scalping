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
