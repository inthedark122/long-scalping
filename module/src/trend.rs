//! The COMBINED trend — the single source of truth the strategy reads for "up / down / chop".
//!
//! It fuses three independent calculators, each from DIFFERENT inputs, by majority vote:
//!   trend_mtf        — multi-timeframe movement/cascade regime (slow, confirmed)
//!   trend_efficiency — Kaufman efficiency ratio on raw 1m closes (fast, price-only)
//!   trend_structure  — SMC swing structure (higher-highs vs lower-lows)
//! Combining is more robust than any single one (a lagging 5m flag can't call "uptrend" on a choppy tape
//! unless price action + structure agree). The zone gate, UpStairs and DownStairs all read THIS, so
//! "up/down" means exactly one thing everywhere. NOTE: the efficiency calculator separately drives the
//! volatility GATE (hold-for-TP in Quiet/Chop) — that's a MAGNITUDE concern, distinct from trend DIRECTION.

use crate::state::State;
use crate::trend_efficiency::efficiency_dir;
use crate::trend_mtf::mtf_dir;
use crate::trend_structure::structure_dir;

pub(crate) struct Trend {
    pub dir: i8,      // -1 down · 0 neutral · +1 up (majority of the 3 votes)
    pub strength: u8, // how many of the 3 calculators agree with dir (0..3)
}

impl Trend {
    pub fn is_up(&self) -> bool {
        self.dir == 1
    }
    pub fn is_down(&self) -> bool {
        self.dir == -1
    }
}

/// Combine the three trend calculators into one signal (majority vote + agreement strength).
pub(crate) fn compute_trend(state: &State) -> Trend {
    let votes = [mtf_dir(state), efficiency_dir(state), structure_dir(state)];
    let sum: i32 = votes.iter().map(|&v| v as i32).sum();
    let dir = sum.signum() as i8;
    let strength = if dir == 0 {
        0
    } else {
        votes.iter().filter(|&&v| v == dir).count() as u8
    };
    Trend { dir, strength }
}
