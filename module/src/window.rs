//! Window-context detector — Layer 1 of the FC scalp.
//!
//! Answers the questions FC needs *before* it scalps a movement: **is it safe to
//! scalp right now, within what bounds, with which bias, at what scale** — WITHOUT
//! placing any order. FC (Layer 2) consumes this so it only scalps 1m movements
//! inside a safe post-volatility range, NATR-sized, and stands down the moment it
//! turns into a (down)trend (the failure mode that bag-held ALLO).
//!
//! Sources used (all already available to the module): the per-TF movement cascade
//! (`cas1`/`cas5` — direction, block size, rolling avg, breaks), the MRC band
//! envelope, the per-TF regimes, and the live order-book support wall. Richer range
//! inputs (ZigZag swing, NATR, SMC order-block, DP Zee) layer in once we confirm the
//! module actually receives them; the cascade's 1m avg block is already a good
//! NATR_1m proxy, so nothing here is blocked on that.

use crate::sizing::fcscalp_support;
use crate::state::State;
use lte_strategy_bridge::abi::ModuleInput;

/// Read-only "is it safe to scalp, and where" context handed to FC each tick.
#[derive(Debug, Clone, Default)]
pub struct WindowContext {
    /// Safe to scalp: ranging post-volatility, not a (down)trend, a real floor below,
    /// price not pinned at the top.
    pub safe: bool,
    /// Lower guard / stop reference. A decisive break below = the range failed → FC
    /// stands down (this is the guard against gridding a falling knife).
    pub floor: f64,
    /// Upper guard — don't chase / open new scalps above this.
    pub ceiling: f64,
    /// Long-only bias from the 5m read: +1 favorable, 0 neutral, -1 unfavorable.
    pub bias: i8,
    /// Typical 1m movement as a % of price — the scalp/TP size (cascade-1m avg ≈ NATR_1m).
    pub scale_pct: f64,
    /// Price position in [floor, ceiling]: 0.0 = on the floor, 1.0 = at the ceiling.
    pub pos_frac: f64,
}

/// A confirmed 5m down-block far above its average is a trend leg, not a range.
const TREND_STRETCH: f64 = 1.8;
/// Don't open new scalps once price is in the top fraction of the window.
const TOP_GUARD: f64 = 0.80;

pub(crate) fn detect_window(input: &ModuleInput, state: &State) -> WindowContext {
    let price = input.price;
    if price <= 0.0 {
        return WindowContext::default();
    }

    // ── Scale: the typical 1m movement = the scalp size. Prefer the 1m cascade's
    //    rolling avg block (a live NATR_1m proxy); fall back to a fraction of MRC range.
    let scale_pct = if state.cas1.avg_size > 0.0 {
        state.cas1.avg_size / price * 100.0
    } else if state.mrc_range > 0.0 {
        state.mrc_range / price * 100.0 * 0.25
    } else {
        0.0
    };

    // ── Bounds: nearest meaningful support below / resistance above price.
    let floor = window_floor(input, state, price);
    let ceiling = window_ceiling(state, price);
    let pos_frac = if ceiling > floor && floor > 0.0 {
        ((price - floor) / (ceiling - floor)).clamp(0.0, 1.0)
    } else {
        0.5
    };

    // ── Bias (long-only): the 5m direction. A 5m down-cascade / down-regime is
    //    unfavorable for opening longs; up / sideways is fine.
    let bias = if state.tf5.regime == -2 || state.cas5.cas_dir == -1 {
        -1
    } else if state.tf5.regime == 2 || state.cas5.cas_dir == 1 {
        1
    } else {
        0
    };

    // ── Safe-to-scalp gate ──────────────────────────────────────────────────────
    // Stand down ONLY on an ACTIVE down-leg — the live 5m cascade pointing DOWN with its
    // block stretched well past avg (a real dump in progress) — or a confirmed mkt-mode
    // downtrend. We deliberately do NOT use tf5.regime here: that MRC-cross flag stays at
    // -2 long after a dump has stopped, which over-blocked ALLO while it was oscillating.
    // An UP-leg is fine — we buy dips inside it. This cascade-based check both frees the
    // post-dump chop AND still catches the genuine dump (the -$17 ALLO scenario).
    let cas5_size = state.cas5.cas_high - state.cas5.cas_low;
    let active_dump = state.cas5.cas_dir == -1
        && state.cas5.avg_size > 0.0
        && cas5_size > state.cas5.avg_size * TREND_STRETCH;
    let has_floor = floor > 0.0 && floor < price;
    let not_at_top = pos_frac < TOP_GUARD;

    let safe = !active_dump && state.mkt_mode != -2 && has_floor && not_at_top;

    WindowContext {
        safe,
        floor,
        ceiling,
        bias,
        scale_pct,
        pos_frac,
    }
}

/// Nearest support below price: live order-book wall first, then the nearest MRC band
/// below, then the 5m cascade low.
fn window_floor(input: &ModuleInput, state: &State, price: f64) -> f64 {
    // Order book is an opt-in extension (default off) — window/cascade levels are primary.
    if state.cfg.fcscalp_use_ob {
        if let Some(s) = fcscalp_support(input, price) {
            if s > 0.0 && s < price {
                return s;
            }
        }
    }
    let mut best = 0.0_f64; // highest band still below price = nearest support
    for b in [
        state.mrc_di,
        state.mrc_ds,
        state.mrc_do,
        state.mrc_db,
        state.mrc_ml,
    ] {
        if b > 0.0 && b < price && b > best {
            best = b;
        }
    }
    if best > 0.0 {
        return best;
    }
    if state.cas5.cas_low > 0.0 && state.cas5.cas_low < price {
        return state.cas5.cas_low;
    }
    0.0
}

/// Nearest resistance above price: nearest MRC band above, then the 5m cascade high.
fn window_ceiling(state: &State, price: f64) -> f64 {
    let mut best = f64::MAX; // lowest band still above price = nearest resistance
    for b in [
        state.mrc_ui,
        state.mrc_us,
        state.mrc_uo,
        state.mrc_ub,
        state.mrc_ml,
    ] {
        if b > price && b < best {
            best = b;
        }
    }
    if best < f64::MAX {
        return best;
    }
    if state.cas5.cas_high > price {
        return state.cas5.cas_high;
    }
    price
}

/// Compact debug readout, e.g. "SAFE f1550.2 c1599.1 bi+1 scl0.18% pos0.42"
/// or "off(dn) ..." when not safe.
pub(crate) fn window_debug(w: &WindowContext) -> String {
    let tag = if w.safe { "SAFE" } else { "off" };
    format!(
        "{} f{:.4} c{:.4} bi{:+} scl{:.2}% pos{:.2}",
        tag, w.floor, w.ceiling, w.bias, w.scale_pct, w.pos_frac
    )
}
