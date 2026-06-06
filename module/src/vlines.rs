//! V-movement tracker: per-MRC-line resistance/support "V" counting, applied
//! each SugInfo tick and seeded from candle history at Init.

use crate::consts::*;
use crate::indicators::*;
use crate::state::*;
use lte_strategy_bridge::abi::Direction;

// ── V-movement tracker ───────────────────────────────────────────────────────
//
// Runs on every SugInfo tick. One VLineTracker per MRC line — each tracks its
// own V history permanently (no reset between Vs). This preserves V1/V2/V3
// power history even when price oscillates through adjacent lines between spikes.
pub(crate) fn update_v_lines(state: &mut State, price: f64) {
    if !state.has_mrc || state.mrc_range <= 0.0 {
        return;
    }
    let levels: [f64; 9] = [
        state.mrc_db,
        state.mrc_do,
        state.mrc_ds,
        state.mrc_di,
        state.mrc_ml,
        state.mrc_ui,
        state.mrc_us,
        state.mrc_uo,
        state.mrc_ub,
    ];
    let range = state.mrc_range;
    let tick = state.ticks; // sug tick for real-time crossing recency (Timing 0.2)
    apply_v_lines(state, &levels, range, price, tick);
}

// Core V-line update: process ONE price point against a given set of 9 line values on a given tick.
// Used both live (current bands, state.ticks) and for cold-start seeding (historical bands per
// candle, synthetic ticks). Maintains per-line peak/trough, completed-V power, and crossing recency.
pub(crate) fn apply_v_lines(state: &mut State, levels: &[f64; 9], range: f64, price: f64, tick: u64) {
    if range <= 0.0 {
        return;
    }
    for (i, &lvl) in levels.iter().enumerate() {
        if lvl <= 0.0 {
            continue;
        }
        let now_above = price >= lvl;
        let vl = &mut state.v_lines[i];

        match vl.was_above {
            None => {
                // First tick: initialise without closing any V
                vl.was_above = Some(now_above);
                if now_above {
                    vl.peak = Some(price);
                } else {
                    vl.trough = Some(price);
                }
            }
            Some(prev_above) if prev_above != now_above => {
                // Crossing — record real-time recency/direction (feeds entry freshness)
                vl.last_cross_tick = tick;
                vl.last_cross_dir = if now_above { 1 } else { -1 };
                if now_above {
                    // Crossed above: close the short V that was in progress
                    if let Some(tr) = vl.trough {
                        let power = (lvl - tr).abs() / range * 100.0;
                        let s = &mut vl.short_side;
                        s.prev_power = s.last_power;
                        s.last_power = power;
                        s.count = s.count.saturating_add(1);
                        s.exhausted = s.count >= 2 && s.last_power < s.prev_power * 0.70;
                        s.stair = s.count >= 2 && s.last_power > s.prev_power * 1.10;
                    }
                    vl.trough = None;
                    vl.peak = Some(price);
                } else {
                    // Crossed below: close the long V that was in progress
                    if let Some(pk) = vl.peak {
                        let power = (pk - lvl).abs() / range * 100.0;
                        let s = &mut vl.long_side;
                        s.prev_power = s.last_power;
                        s.last_power = power;
                        s.count = s.count.saturating_add(1);
                        s.exhausted = s.count >= 2 && s.last_power < s.prev_power * 0.70;
                        s.stair = s.count >= 2 && s.last_power > s.prev_power * 1.10;
                    }
                    vl.peak = None;
                    vl.trough = Some(price);
                }
                vl.was_above = Some(now_above);
            }
            Some(_) => {
                // Same side: update peak or trough
                if now_above {
                    vl.peak = Some(vl.peak.map_or(price, |p| p.max(price)));
                } else {
                    vl.trough = Some(vl.trough.map_or(price, |t| t.min(price)));
                }
            }
        }
    }
}

// v0.56 cold-start: seed the V-line tracker from the candle history available at Init so the
// freshness / stair / asymmetry signals aren't blind during the live warm-up. DEPTH-AGNOSTIC —
// it uses however many candles the feed provides (10 today, more later) by scanning until the MRC
// history runs out. Each candle's High/Low/Close is replayed against THAT candle's own MRC bands on
// a synthetic tick timeline (~TICKS_PER_CANDLE per candle); live ticks then continue on the same
// scale so the freshness windows age correctly. Candle wicks (High/Low) yield real penetration
// power, so with more candles the V counts / exhaustion populate fully.
pub(crate) fn seed_v_lines(state: &mut State, ind: &Indicators) {
    const MAX_SEED_CANDLES: usize = 512; // ready for far more than 10 when the feed sends them
    let mut depth = 0usize;
    for offset in 0..MAX_SEED_CANDLES {
        if read_mrc_at(ind, offset).is_none() {
            break;
        }
        depth = offset + 1;
    }
    if depth == 0 {
        return;
    }

    for k in 0..depth {
        let offset = depth - 1 - k; // oldest → newest
        let synthetic_tick = (k as u64 + 1) * TICKS_PER_CANDLE; // start at 1 candle so tick > 0
        let bands = match read_mrc_at(ind, offset) {
            Some(b) => b,
            None => continue,
        };
        let range = bands[8] - bands[0];
        if range <= 0.0 {
            continue;
        }
        let ml = bands[4];
        let high = read_float(ind, TF, offset, CANDLE_IND, "High").unwrap_or(ml);
        let low = read_float(ind, TF, offset, CANDLE_IND, "Low").unwrap_or(ml);
        let close = read_float(ind, TF, offset, CANDLE_IND, "Close").unwrap_or(ml);
        // Replay the candle's extremes in temporal order (by direction), then the close, so upper/
        // lower wick rejections register as real Vs.
        let path = match read_direction(ind, TF, offset, CANDLE_IND, "Direction") {
            Some(Direction::Long) => [low, high, close], // up candle: dipped then rallied
            _ => [high, low, close],                     // down/unknown: rallied then dropped
        };
        for &px in &path {
            apply_v_lines(state, &bands, range, px, synthetic_tick);
        }
    }
    // Hand off to live ticks on the same synthetic scale (newest seeded candle = depth·TPC).
    state.ticks = (depth as u64) * TICKS_PER_CANDLE;
}

