//! Fast-candle tracker: detect short directional bursts within the tick stream.

use crate::consts::*;
use crate::state::*;

// ── Fast candle tracker ──────────────────────────────────────────────────────
//
// Runs on every SugInfo tick using real-time price delta.
// Groups consecutive fast-moving ticks (same direction) into one "event".
// A fast tick = price moved > 0.2% of MRC range in one tick.
// BSG/Gain from the last closed 1m candle enriches peak stats.
// Pause tolerance: 5 slow ticks before ending the group (SugInfo is noisy).
// All statistics use EMA(α=0.20).
pub(crate) fn update_fast_candles(state: &mut State, price: f64) {
    let tick = state.ticks;
    let range = state.mrc_range;
    if range <= 0.0 {
        state.fc_tick_prev_price = price;
        return;
    }

    let prev = state.fc_tick_prev_price;
    // On first call or after a reset, just store price
    if prev == 0.0 {
        state.fc_tick_prev_price = price;
        return;
    }

    let delta = price - prev;
    state.fc_tick_prev_price = price;

    // A tick is "fast" when price moved > 0.2% of MRC range in one SugInfo step
    let fast_threshold = range * 0.002;
    let fast_bull = delta > fast_threshold;
    let fast_bear = delta < -fast_threshold;
    let is_fast = fast_bull || fast_bear;
    let dir_now: i8 = if fast_bull { 1 } else { -1 };

    // Pre-event signal: BSG-based (uses last closed 1m candle — intentionally stale).
    // High volume + ambiguous direction → often precedes a directional spike.
    let gain_range = (state.vol_gain_max - state.vol_gain_min).max(GAIN_RANGE_FLOOR);
    let bsg_range = (state.vol_bsg_max - state.vol_bsg_min).max(BSG_RANGE_FLOOR);
    let gain_pct = (state.vol_gain - state.vol_gain_min) / gain_range;
    let bsg_pct = (state.vol_bsg - state.vol_bsg_min) / bsg_range;
    let prefire_bull = gain_pct > 0.55 && (0.40..0.65).contains(&bsg_pct) && state.vol_bsg > 0.0;
    let prefire_bear = gain_pct > 0.55 && (0.35..0.60).contains(&bsg_pct) && state.vol_bsg < 0.0;
    let fc = &mut state.fc;
    if prefire_bull {
        fc.prefire = 1;
        fc.prefire_candle = tick;
    } else if prefire_bear {
        fc.prefire = -1;
        fc.prefire_candle = tick;
    } else if tick > fc.prefire_candle + 30 {
        fc.prefire = 0;
    } // expire after ~30 ticks

    const ALPHA: f64 = 0.20;
    let ema = |cur: f64, val: f64| -> f64 {
        if cur == 0.0 {
            val
        } else {
            cur * (1.0 - ALPHA) + val * ALPHA
        }
    };

    if fc.active {
        if is_fast && dir_now == fc.dir {
            // Enrich with BSG/Gain from last candle when available
            fc.peak_gain = fc.peak_gain.max(state.vol_gain.abs());
            fc.peak_bsg = fc.peak_bsg.max(state.vol_bsg.abs());
            fc.pause = 0;
        } else if !is_fast && fc.pause < 5 {
            fc.pause += 1; // tolerate brief pauses
        } else {
            // Event ends — update rolling stats (all inputs clamped to prevent outlier drift)
            let pct = ((price - fc.start_price).abs() / fc.start_price.max(f64::EPSILON) * 100.0)
                .min(5.0); // cap at 5%
            let dur = tick.saturating_sub(fc.start_candle).min(60) as f64; // cap at 60 ticks
            let calm = fc.start_candle.saturating_sub(fc.calm_since).min(300) as f64; // cap at 300 ticks (~5 min quiet)
            let gain = fc.peak_gain.min(10.0); // cap gain at 10
            fc.avg_pct = ema(fc.avg_pct, pct);
            fc.avg_gain = ema(fc.avg_gain, gain);
            fc.avg_dur = ema(fc.avg_dur, dur);
            fc.avg_calm = ema(fc.avg_calm, calm.max(1.0));
            fc.count += 1;
            fc.momentum = (fc.momentum * 0.85 + fc.dir as f64).clamp(-10.0, 10.0);
            fc.active = false;
            fc.last_end_dir = fc.dir;
            fc.last_end_tick = tick;
            fc.calm_since = tick;
            // Sliding window — keep last 8
            fc.history.push(crate::state::FcEvent {
                dir: fc.dir,
                pct,
                dur: dur as u64,
            });
            if fc.history.len() > 8 {
                fc.history.remove(0);
            }
            fc.pause = 0;
        }
    } else if is_fast {
        fc.active = true;
        fc.dir = dir_now;
        fc.start_price = price;
        fc.start_candle = tick;
        fc.peak_gain = state.vol_gain.abs();
        fc.peak_bsg = state.vol_bsg.abs();
        fc.pause = 0;
    } else if tick % 50 == 0 {
        fc.momentum *= 0.95; // slow decay during calm periods
    }
}

