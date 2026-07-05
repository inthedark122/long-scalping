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
use crate::state::MarketBias;
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

fn sig_dir(dp_dir: i8, zz_end_dir: i8) -> i8 {
    if zz_end_dir != 0 {
        zz_end_dir
    } else {
        dp_dir
    }
}

fn sig_conf(dp_dir: i8, zz_dir: i8, zz_end_dir: i8, fibo: f64) -> u8 {
    let mut conf = 0u8;
    if dp_dir != 0 {
        conf += 1;
    }
    if zz_dir != 0 {
        conf += 1;
    }
    if zz_end_dir != 0 {
        conf += 1;
    }
    if dp_dir != 0 && dp_dir == zz_end_dir {
        conf += 1;
    }
    if fibo > 0.0 {
        conf += 1;
    }
    conf.min(5)
}

pub(crate) fn update_market_bias(state: &mut State, price: f64) {
    let dir5 = sig_dir(state.sig5.dp_dir, state.sig5.zz_end_dir);
    let dir15 = sig_dir(state.sig15.dp_dir, state.sig15.zz_end_dir);
    let dir1h = sig_dir(state.sig1h.dp_dir, state.sig1h.zz_end_dir);
    let conf5 = sig_conf(
        state.sig5.dp_dir,
        state.sig5.zz_dir,
        state.sig5.zz_end_dir,
        state.sig5.zz_fibo,
    );
    let conf15 = sig_conf(
        state.sig15.dp_dir,
        state.sig15.zz_dir,
        state.sig15.zz_end_dir,
        state.sig15.zz_fibo,
    );
    let conf1h = sig_conf(
        state.sig1h.dp_dir,
        state.sig1h.zz_dir,
        state.sig1h.zz_end_dir,
        state.sig1h.zz_fibo,
    );
    let stretched_up = state.sig5.dp_zee1 > 0.0 && price > state.sig5.dp_zee1 * 1.03;
    let stretched_dn = state.sig5.dp_zee1 > 0.0 && price < state.sig5.dp_zee1 * 0.97;

    let (bias, conf) = if state.post_sl_active && state.ticks <= state.post_sl_until_tick {
        (MarketBias::Sideways, 0)
    } else if dir5 > 0
        && conf5 >= 2
        && !stretched_up
        && state.mkt_mode != -2
        && (dir1h >= 0 || conf1h == 0)
    {
        (MarketBias::Long, conf5.saturating_add(conf1h).min(5))
    } else if dir5 < 0
        && conf5 >= 2
        && !stretched_dn
        && state.mkt_mode != 2
        && (dir1h <= 0 || conf1h == 0)
    {
        (MarketBias::Short, conf5.saturating_add(conf1h).min(5))
    } else if state.tf5.regime == 1 || state.mkt_mode == 1 {
        (MarketBias::Sideways, conf15.max(conf5).min(5))
    } else if dir15 > 0 && conf15 >= 2 && dir1h >= 0 {
        (MarketBias::Long, conf15.min(5))
    } else if dir15 < 0 && conf15 >= 2 && dir1h <= 0 {
        (MarketBias::Short, conf15.min(5))
    } else {
        (MarketBias::Sideways, conf5.max(conf15).max(conf1h).min(5))
    };

    state.market_bias = bias;
    state.market_bias_conf = conf;
}
