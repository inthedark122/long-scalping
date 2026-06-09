//! Trend calculator #1 — the MULTI-TIMEFRAME regime trend (slow, confirmed).
//!
//! Reads the multi-timeframe movement engine (movement.rs): `mkt_mode` is a hysteresis mode fused from
//! the 1m + 5m regime + cascade direction + SMC direction; `tf5.regime` is the 5m trend; `angle` is the
//! MRC mean-line slope. This is the "classic" trend — multi-TF and confirmed — as opposed to the fast,
//! price-only efficiency trend. (The mode is computed in movement::advance_market_mode / advance_tf_from_ind.)

use crate::state::State;

/// Direction this calculator votes for the combined `Trend`: +1 up · -1 down · 0 sideways.
/// Up = a confirmed up-mode, OR a 5m up-regime BACKED by a rising mean and a non-sideways 1m (the 0.77
/// "genuine uptrend" rule that rejects a lagging 5m flag). Mirror for down.
pub(crate) fn mtf_dir(state: &State) -> i8 {
    if state.mkt_mode == 2 || (state.tf5.regime == 2 && state.mkt_mode != 1 && state.angle > 0.0) {
        1
    } else if state.mkt_mode == -2
        || (state.tf5.regime == -2 && state.mkt_mode != 1 && state.angle < 0.0)
    {
        -1
    } else {
        0
    }
}
