//! Movement cascade, market-mode observer, risk ratchet, and per-timeframe
//! movement/regime tracking.

use crate::consts::*;
use crate::indicators::*;
use crate::interest::{update_interest_line, update_zone_streak};
use crate::state::*;
use lte_strategy_bridge::abi::Direction;

// ── Movement cascade ─────────────────────────────────────────────────────────
//
// Ports movementCascade.ts into an incremental per-candle algorithm.
// Direction is determined by vol_bsg sign (>= 0 = Long, < 0 = Short).
// Counter-moves whose high-low size is < 40% of the rolling avg cascade size
// are absorbed: their extreme price is recorded as a break level (support/
// resistance DCA zone) and the parent cascade block continues.

pub(crate) fn update_cascade(cas: &mut CascadeState, vol_bsg: f64, high: f64, low: f64) {
    if high <= 0.0 || low <= 0.0 {
        return;
    }
    let dir: i8 = if vol_bsg >= 0.0 { 1 } else { -1 };

    if cas.dir == 0 {
        cas.dir = dir;
        cas.high = high;
        cas.low = low;
        cas.candles = 1;
        cas.cas_dir = dir;
        cas.cas_high = high;
        cas.cas_low = low;
        cas.cas_candles = 1;
        return;
    }

    if dir == cas.dir {
        if high > cas.high {
            cas.high = high;
        }
        if low < cas.low {
            cas.low = low;
        }
        cas.candles += 1;
        if dir == cas.cas_dir {
            if high > cas.cas_high {
                cas.cas_high = high;
            }
            if low < cas.cas_low {
                cas.cas_low = low;
            }
            cas.cas_candles += 1;
        }
        return;
    }

    // Raw block just completed in direction cas.dir — evaluate for cascade absorption.
    let completed_size = cas.high - cas.low;
    let threshold = if cas.avg_size > 0.0 {
        cas.avg_size * 0.40
    } else {
        f64::MAX
    };
    let is_counter = cas.dir != cas.cas_dir;
    let absorb = is_counter && completed_size < threshold;

    if absorb {
        // Small counter-move: record break level and extend cascade range.
        // dir == -1 (short counter in long cascade) → break price = counter LOW (support)
        // dir == +1 (long counter in short cascade) → break price = counter HIGH (resistance)
        let break_price = if cas.dir == -1 { cas.low } else { cas.high };
        if cas.break_n >= 5 {
            cas.breaks.rotate_left(1);
            cas.breaks[4] = break_price;
        } else {
            cas.breaks[cas.break_n] = break_price;
            cas.break_n += 1;
        }
        if cas.high > cas.cas_high {
            cas.cas_high = cas.high;
        }
        if cas.low < cas.cas_low {
            cas.cas_low = cas.low;
        }
        // First candle of the resuming cascade direction also extends the cascade.
        if dir == cas.cas_dir {
            if high > cas.cas_high {
                cas.cas_high = high;
            }
            if low < cas.cas_low {
                cas.cas_low = low;
            }
            cas.cas_candles += 1;
        }
    } else {
        // Real reversal: commit current cascade to history, start a new one.
        let cas_size = cas.cas_high - cas.cas_low;
        if cas_size > 0.0 {
            cas.sizes[cas.sizes_pos] = cas_size;
            cas.sizes_pos = (cas.sizes_pos + 1) % 10;
            if cas.sizes_len < 10 {
                cas.sizes_len += 1;
            }
            cas.avg_size = cas.sizes[..cas.sizes_len].iter().sum::<f64>() / cas.sizes_len as f64;
        }
        cas.cas_dir = dir;
        cas.cas_high = high.max(cas.high);
        cas.cas_low = low.min(cas.low);
        cas.cas_candles = 1;
        cas.break_n = 0;
        cas.breaks = [0.0; 5];
    }

    cas.dir = dir;
    cas.high = high;
    cas.low = low;
    cas.candles = 1;
}

/// Rebuild a TF's movement cascade from the 10-candle Init history (oldest-first pass).
pub(crate) fn init_cascade(cas: &mut CascadeState, ind: &Indicators, tf_sec: i64) {
    for offset in (0..10usize).rev() {
        let bsg = read_float(ind, tf_sec, offset, "volume_50", "BuySellGain").unwrap_or(0.0);
        let high = read_float(ind, tf_sec, offset, CANDLE_IND, "High").unwrap_or(0.0);
        let low = read_float(ind, tf_sec, offset, CANDLE_IND, "Low").unwrap_or(0.0);
        update_cascade(cas, bsg, high, low);
    }
}

// ── Market mode observer ─────────────────────────────────────────────────────
//
// Classifies current state into Sideways (1), TrendLong (2), TrendShort (-2).
// Requires cascade block > 1.2× avg AND at least 2 of 3 TFs agreeing.
// Mode commits after 2 consecutive confirming candles (hysteresis).

pub(crate) fn classify_mode(state: &State) -> i8 {
    // v0.49.0 #3: strong multi-TF trend override. A slow grind never builds a big cascade,
    // so the size_ratio gate below kept returning Sideways while 1m+5m were both clearly
    // trending and SMC agreed (observed: a clean -1% downtrend read as "sideways"). If both
    // TF regimes point the same way AND SMC confirms, it IS a trend. tf*.regime are 0/UNK
    // until warmed up (>=10 candles), so this can't fire prematurely.
    if state.tf1.regime == -2 && state.tf5.regime == -2 && state.smc_dir == -1 {
        return -2;
    }
    if state.tf1.regime == 2 && state.tf5.regime == 2 && state.smc_dir == 1 {
        return 2;
    }

    if state.cas1.sizes_len < 3 || state.cas1.avg_size < 0.001 {
        return 1;
    }
    let cas_size = state.cas1.cas_high - state.cas1.cas_low;
    let size_ratio = cas_size / state.cas1.avg_size;
    if size_ratio < 1.2 {
        return 1;
    }

    let dir = state.cas1.cas_dir;
    let agrees = match dir {
        1 => {
            (state.tf1.cross >= 1) as u8
                + (state.tf5.regime == 2) as u8
                + (state.tf15.regime >= 1) as u8
        }
        -1 => {
            (state.tf1.cross <= -1) as u8
                + (state.tf5.regime == -2) as u8
                + (state.tf15.regime <= -1) as u8
        }
        _ => 0,
    };
    if agrees >= 2 {
        // Sanity check: tf1.cross is real-time price zone (-4..+4).
        // A bullish cascade can lag while price has already dropped to the lower range.
        let cross_contradicts =
            (dir == 1 && state.tf1.cross <= -2) || (dir == -1 && state.tf1.cross >= 2);
        // Priority 2.1: above-avg volume candle on opposing side confirms the trend is wrong.
        // Tighter than cross_contradicts: cross <= -1 (below ML) + real volume = genuine reversal.
        let volume_contradicts = (dir == 1 && state.tf1.cross <= -1 && state.vol_gain > 0.0)
            || (dir == -1 && state.tf1.cross >= 1 && state.vol_gain > 0.0);
        // Priority 2.3: extreme market activity with strong BSG opposing the proposed trend direction.
        let ntps_contradicts = state.ntps > 400
            && ((dir == 1 && state.vol_bsg < -1.5) || (dir == -1 && state.vol_bsg > 1.5));
        if cross_contradicts || volume_contradicts || ntps_contradicts {
            1
        } else {
            dir * 2
        }
    } else {
        1
    }
}

pub(crate) fn advance_market_mode(state: &mut State) {
    let candidate = classify_mode(state);
    if candidate == state.mkt_mode {
        state.mkt_mode_candles += 1;
        state.mkt_cand = candidate;
        state.mkt_cand_candles = 0;
    } else if candidate == state.mkt_cand {
        state.mkt_cand_candles += 1;
        if state.mkt_cand_candles >= 3 {
            state.mkt_mode = candidate;
            state.mkt_mode_candles = 0;
            state.mkt_cand_candles = 0;
            // Reset transition-close progress on every mode change.
            state.mode_close_l = 0;
            state.filled.retain(|k| !k.starts_with("tc-"));
        }
    } else {
        state.mkt_cand = candidate;
        state.mkt_cand_candles = 1;
    }
}

// ── Risk ratchet ─────────────────────────────────────────────────────────────
//
// Combines two signals:
//   peak_factor: how far auto_max_amount is below its all-time high (profit drawn down)
//   floor_factor: how far auto_max_amount is below the starting max_amount (net loss)
// Both scale entry amounts; the geometric mean softens the combined effect.

pub(crate) fn update_risk_ratchet(state: &mut State, max_amount: f64, auto_max_amount: f64) {
    if max_amount <= 0.0 {
        return;
    }
    let current = if auto_max_amount > 0.0 {
        auto_max_amount
    } else {
        max_amount
    };
    if current > state.risk_peak_auto {
        state.risk_peak_auto = current;
    }

    let peak_factor = if state.risk_peak_auto > 0.0 {
        (current / state.risk_peak_auto).clamp(0.10, 1.0)
    } else {
        1.0
    };

    // Penalise only when auto_max_amount has fallen below the starting budget.
    let floor_factor = if auto_max_amount > 0.0 && auto_max_amount < max_amount {
        (auto_max_amount / max_amount).max(0.10)
    } else {
        1.0
    };

    let post_sl_factor = if state.post_sl_active && state.ticks <= state.post_sl_until_tick {
        state.post_sl_factor.clamp(0.10, 1.0)
    } else {
        1.0
    };
    if state.post_sl_active && state.ticks > state.post_sl_until_tick {
        state.post_sl_active = false;
        state.post_sl_factor = 1.0;
    }

    state.risk_drawdown = (peak_factor * floor_factor).sqrt().clamp(0.10, 1.0) * post_sl_factor;
}


// ── Per-TF movement tracking ─────────────────────────────────────────────────

pub(crate) fn init_tf_from_ind(ind: &Indicators, state: &mut State, tf_sec: i64) {
    for offset in (0..10usize).rev() {
        if read_float(ind, tf_sec, offset, MRC, "MeanLine").is_none() {
            continue;
        }
        let cross = read_float(ind, tf_sec, offset, MRC, "CurrentCross")
            .map(|f| f.round() as i8)
            .unwrap_or(0);
        let candle_dir = match read_direction(ind, tf_sec, offset, CANDLE_IND, "Direction") {
            Some(Direction::Long) => 1i8,
            Some(Direction::Short) => -1i8,
            _ => 0i8,
        };
        {
            let tf_state = match tf_sec {
                TF => &mut state.tf1,
                TF_5M => &mut state.tf5,
                TF_15M => &mut state.tf15,
                TF_30M => &mut state.tf30,
                _ => return,
            };
            advance_tf(tf_state, cross, candle_dir, 0.0);
        }
        // Populate level stats, interest line, and zone streaks from history
        // so the first SugInfo can immediately evaluate stair/V-bounce conditions
        if tf_sec == TF {
            update_interest_line(state);
            update_zone_streak(state);
        }
    }
}

pub(crate) fn advance_tf_from_ind(ind: &Indicators, state: &mut State, tf_sec: i64) {
    let cross = match read_float(ind, tf_sec, 0, MRC, "CurrentCross") {
        Some(f) => f.round() as i8,
        None => return,
    };
    let candle_dir = match read_direction(ind, tf_sec, 0, CANDLE_IND, "Direction") {
        Some(Direction::Long) => 1i8,
        Some(Direction::Short) => -1i8,
        _ => 0i8,
    };
    {
        let tf_state = match tf_sec {
            TF => &mut state.tf1,
            TF_5M => &mut state.tf5,
            TF_15M => &mut state.tf15,
            TF_30M => &mut state.tf30,
            _ => return,
        };
        advance_tf(tf_state, cross, candle_dir, 0.0);
    }
    // After 1m candle: update interest line, per-level statistics, zone streaks
    if tf_sec == TF {
        update_interest_line(state);
        update_zone_streak(state);
    }
}

pub(crate) fn advance_tf(tf: &mut TfMovState, cross: i8, candle_dir: i8, ext: f64) {
    let prev_cross = tf.cross;
    tf.cross_prev = prev_cross;
    tf.candles += 1;
    tf.cross = cross;

    tf.history[tf.hist_pos] = cross;
    tf.hist_pos = (tf.hist_pos + 1) % 20;
    if tf.hist_len < 20 {
        tf.hist_len += 1;
    }

    // Floor/ceiling from full ring buffer
    let (mut mn, mut mx) = (cross, cross);
    for i in 0..tf.hist_len {
        let idx = (tf.hist_pos + 20 - 1 - i) % 20;
        let v = tf.history[idx];
        if v < mn {
            mn = v;
        }
        if v > mx {
            mx = v;
        }
    }
    let old_floor = tf.floor;
    let old_ceiling = tf.ceiling;
    tf.floor = mn;
    tf.ceiling = mx;
    tf.floor_age = if mn != old_floor || tf.candles == 1 {
        0
    } else {
        tf.floor_age + 1
    };
    tf.ceiling_age = if mx != old_ceiling || tf.candles == 1 {
        0
    } else {
        tf.ceiling_age + 1
    };

    // Angle sign: track consecutive cross movement direction
    let delta = cross.saturating_sub(prev_cross).signum();
    if delta != 0 {
        if delta == tf.angle_sign {
            tf.angle_sign_candles += 1;
        } else {
            tf.angle_sign = delta;
            tf.angle_sign_candles = 1;
        }
    }

    // Slow mov update: ±step per candle based on cross level, body/shadow, extension
    let dir_sign = cross.signum() as f64;
    let abs_cross = cross.abs() as f64;
    let body_factor: f64 =
        if candle_dir != 0 && ((candle_dir > 0 && cross > 0) || (candle_dir < 0 && cross < 0)) {
            1.5 // body agrees with cross = confirmed directional cross
        } else if candle_dir != 0 {
            0.7 // body opposes = shadow cross = weaker
        } else {
            1.0
        };
    let ext_bonus = ext.abs().min(3.0);
    let step = dir_sign * (1.0 + abs_cross * 0.8 + ext_bonus) * body_factor;
    let momentum = if (tf.mov > 20.0 && step > 0.0) || (tf.mov < -20.0 && step < 0.0) {
        1.3
    } else if (tf.mov > 0.0 && step < 0.0) || (tf.mov < 0.0 && step > 0.0) {
        0.5
    } else {
        1.0
    };
    tf.mov = (tf.mov + step * momentum).clamp(-100.0, 100.0);
    if cross == 0 {
        tf.mov *= 0.85;
    }

    tf.regime = classify_regime(tf);
    tf.conf = compute_conf(tf);
}

pub(crate) fn classify_regime(tf: &TfMovState) -> i8 {
    if tf.hist_len < 10 {
        return 0; // UNK — warming up
    }
    // floor=ceiling=non-zero: price locked at one cross level for hist_len candles → early trend
    if tf.floor == tf.ceiling && tf.floor != 0 {
        return if tf.floor > 0 { 2 } else { -2 };
    }
    // floor >= -1: never dips below DownInner → floor is high
    // ceiling <= 1: never pushes above UpInner → ceiling is low
    match (tf.floor >= -1, tf.ceiling <= 1) {
        (true, false) => 2,  // UP: high floor + pushes up
        (false, true) => -2, // DOWN: low ceiling + sinks down
        _ => 1,              // WIN: ranging (both sides active, or tight around ML)
    }
}

pub(crate) fn compute_conf(tf: &TfMovState) -> u8 {
    if tf.hist_len < 5 {
        return 0;
    }
    let hist_score = (tf.hist_len.min(20) as f64 / 20.0) * 40.0;
    let mov_score = tf.mov.abs().min(100.0) / 100.0 * 40.0;
    let stability = if tf.floor_age > 5 && tf.ceiling_age > 5 {
        20.0
    } else if tf.floor_age > 3 || tf.ceiling_age > 3 {
        10.0
    } else {
        0.0
    };
    ((hist_score + mov_score + stability) as u8).min(100)
}

