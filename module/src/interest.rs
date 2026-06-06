//! Interest line, zone-streak tracking, and entry-scale computation.

use crate::consts::*;
use crate::state::*;

// ── Interest line ────────────────────────────────────────────────────────────
//
// Tracks which MRC cross zone is the current "key level" — the level price
// has been repeatedly testing without breaking. The more tests without a break,
// the stronger the signal in that direction.
//
// Logic:
//   - delta > 0 (cross moved up) and aligns with interest_dir → advance frontier
//   - delta > 0 but opposes interest_dir (e.g. brief bounce) → count as test
//   - penetration >1 zone opposite → interest flipped, new direction
pub(crate) fn update_interest_line(state: &mut State) {
    let cross = state.tf1.cross;
    let prev = state.tf1.cross_prev;

    if cross == prev {
        state.interest_age = state.interest_age.saturating_add(1);
        return;
    }

    let delta = (cross - prev).signum() as i8;

    if state.interest_dir == 0 {
        state.interest_cross = cross;
        state.interest_dir = delta;
        state.interest_age = 0;
        return;
    }

    if delta == state.interest_dir {
        // Continuing in the same direction: advance the frontier
        if (delta > 0 && cross > state.interest_cross)
            || (delta < 0 && cross < state.interest_cross)
        {
            state.interest_cross = cross;
            state.interest_tests = 0; // moved past old level
        }
        state.interest_age += 1;
    } else {
        // Moving opposite — measure how deep the counter-move is
        let depth: i8 = match state.interest_dir {
            1 => (state.interest_cross - cross).max(0),
            _ => (cross - state.interest_cross).max(0),
        };

        if depth <= 1 {
            // Shallow bounce: treat as a test of the interest level
            state.interest_tests = state.interest_tests.saturating_add(1);
            state.interest_age += 1;
        } else {
            // Penetrated 2+ zones: interest broken, flip direction
            state.interest_cross = cross;
            state.interest_dir = delta;
            state.interest_age = 0;
            state.interest_tests = 0;
        }
    }
}

// Maintain zone_streak: consecutive 1m candles price has spent in each cross zone.
// Called on every 1m candle close (after advance_tf updates cross/cross_prev).
pub(crate) fn update_zone_streak(state: &mut State) {
    let cross = state.tf1.cross;
    let prev = state.tf1.cross_prev;

    let cidx = (cross + 4) as usize;
    if cidx >= 9 {
        return;
    }

    if cross == prev {
        state.zone_streak[cidx] += 1;
    } else {
        // Reset old zone, start fresh in new zone
        let pidx = (prev + 4) as usize;
        if pidx < 9 {
            state.zone_streak[pidx] = 0;
        }
        state.zone_streak[cidx] = 1;
    }
}

// ── Entry scale from interest + regime + asymmetry ───────────────────────────
//
// Returns (long_scale, short_scale) multipliers applied to all entry amounts.
//
// Interest line boost: when price has tested a level N times without breaking,
// entries in the confirmed direction scale up proportionally.
//
// Strong-side asymmetry: one side breaks extreme bands while the other is stuck
// at a near-ML level. Example: price above UpBig for 5+ candles (zone_streak[8])
// while short side can't reach DownBig (DownBig never crossed, price drifting near
// DownInner) → market has a strong directional bias, entries follow it.
//
// Sideways dead zone: WIN regime near MeanLine (pzone 0.38-0.62) — cut entries
// opposing the SMC direction to avoid fading a ranging market.
pub(crate) fn compute_entry_scale(state: &State, pzone: f64) -> (f64, f64) {
    let mut lscale = 1.0_f64;
    let mut sscale = 1.0_f64;

    // ── Interest line boost ───────────────────────────────────────────────
    let itests = state.interest_tests;
    if itests >= 3 {
        let boost = ((itests - 2) as f64 * 0.20).min(0.80);
        match state.interest_dir {
            -1 => {
                sscale *= 1.0 + boost;
                lscale *= (1.0 - boost * 0.5).max(0.30);
            }
            1 => {
                lscale *= 1.0 + boost;
                sscale *= (1.0 - boost * 0.5).max(0.30);
            }
            _ => {}
        }
    }

    // ── Strong-side asymmetry ─────────────────────────────────────────────
    // zone_streak[8] = candles above UpBig; zone_streak[0] = candles below DownBig.
    // v0.56: extreme-line break age now from A's v_lines last_cross_tick (ticks, ≈15/candle) —
    // level_stats (B) removed. zone_streak (candle-based) still anchors this, bounding the drift.
    let now_tick = state.ticks;
    const RECENT_BREAK_TICKS: u64 = 30 * TICKS_PER_CANDLE; // ≈30 candles
    const STALE_BREAK_TICKS: u64 = 60 * TICKS_PER_CANDLE; // ≈60 candles
    let ub_streak = state.zone_streak[8]; // consecutive candles at cross=+4 (above UpBig)
    let db_streak = state.zone_streak[0]; // consecutive candles at cross=-4 (below DownBig)
    let ub_cross_age = if state.v_lines[8].last_cross_tick > 0 {
        now_tick.saturating_sub(state.v_lines[8].last_cross_tick)
    } else {
        u64::MAX
    };
    let db_cross_age = if state.v_lines[0].last_cross_tick > 0 {
        now_tick.saturating_sub(state.v_lines[0].last_cross_tick)
    } else {
        u64::MAX
    };

    // Strong long: price has been above UpBig (or recently broke through it upward)
    //              while DownBig was never / very long ago crossed downward
    let strong_long = (ub_streak >= 5
        || (ub_cross_age < RECENT_BREAK_TICKS && state.v_lines[8].last_cross_dir == 1))
        && db_cross_age > STALE_BREAK_TICKS;
    // Strong short: symmetric — price below DownBig while UpBig is stale
    let strong_short = (db_streak >= 5
        || (db_cross_age < RECENT_BREAK_TICKS && state.v_lines[0].last_cross_dir == -1))
        && ub_cross_age > STALE_BREAK_TICKS;

    if strong_long {
        lscale *= 1.40;
        sscale *= 0.50;
    } else if strong_short {
        sscale *= 1.40;
        lscale *= 0.50;
    }

    // ── Sideways dead zone ────────────────────────────────────────────────
    if state.tf1.regime == 1 && pzone > 0.38 && pzone < 0.62 {
        match state.smc_dir {
            1 => sscale *= 0.35,
            -1 => lscale *= 0.35,
            _ => {
                lscale *= 0.60;
                sscale *= 0.60;
            }
        }
    }

    (lscale.max(0.10), sscale.max(0.10))
}

