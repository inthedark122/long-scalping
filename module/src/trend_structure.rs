//! Trend calculator #3 — SMC SWING-STRUCTURE trend.
//!
//! Reads the SMC running local low/high and how recently each printed a NEW extreme (smc_low_age /
//! smc_high_age, in candles — maintained in indicators::update_smc_levels). A fresh new LOW that's
//! fresher than the last new high = the market is stepping DOWN (lower-lows) = DownStairs; a fresh new
//! HIGH = stepping UP (higher-highs) = UpStairs; neither fresh = ranging. One input to the combined `Trend`.

use crate::state::State;

const STAIR_FRESH: u32 = 3; // candles within which a new extreme still counts as "fresh"

/// Direction this calculator votes for the combined `Trend`: +1 higher-highs · -1 lower-lows · 0 range.
pub(crate) fn structure_dir(state: &State) -> i8 {
    if state.smc_low_age <= STAIR_FRESH && state.smc_low_age <= state.smc_high_age {
        -1 // fresh lower-low → stepping down
    } else if state.smc_high_age <= STAIR_FRESH && state.smc_high_age < state.smc_low_age {
        1 // fresh higher-high → stepping up
    } else {
        0 // range
    }
}
