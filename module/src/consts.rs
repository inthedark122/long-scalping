//! Shared compile-time constants for the strategy module.

pub(crate) const TF: i64 = 60;
pub(crate) const TF_5M: i64 = 300;
pub(crate) const TF_15M: i64 = 900;
pub(crate) const TF_30M: i64 = 1800;
pub(crate) const TF_1H: i64 = 3600;
pub(crate) const MRC: &str = "mrc_200";
pub(crate) const DP_IND: &str = "dpsignal";
pub(crate) const SMC_IND: &str = "smc";
pub(crate) const ZZ_IND: &str = "zigzagtrend";
pub(crate) const CANDLE_IND: &str = "candle";
// Approx sug ticks per 1m candle. Used to (a) seed the V-line tracker from candle history on a
// synthetic tick timeline at Init, and (b) express the freshness/staleness windows in candles.
pub(crate) const TICKS_PER_CANDLE: u64 = 15;

// Priority 0.0 — volume normalization floors.
// vol_gain_max/min and vol_bsg_max/min are 50-candle rolling extremes. After a long
// quiet stretch they collapse toward zero, so (max-min) → ~0 and any micro move
// normalizes to ~1.0 (a "full-scale" signal from noise). These floors keep the
// normalization denominator at a meaningful spread so a tiny absolute move stays small.
pub(crate) const GAIN_RANGE_FLOOR: f64 = 0.5; // min Gain spread (Gain is a ratio: +1.0 = 100% above baseline)
pub(crate) const BSG_RANGE_FLOOR: f64 = 1.0; // min BuySellGain spread

// Band names used to build grid levels — entry fills at a band block that band
pub(crate) const LONG_ENTRY_BANDS: &[&str] = &[
    "l-db", "l-db-do", "l-do", "l-do-ds", "l-ds", "l-ds-di", "l-di", "l-di-ml", "l-ml",
];
pub(crate) const LONG_TP_BANDS: &[&str] = &[
    "l-ml+", "l-ml-ui", "l-ui", "l-ui-us", "l-us", "l-us-uo", "l-uo", "l-uo-ub", "l-ub",
];
pub(crate) const SHORT_ENTRY_BANDS: &[&str] = &[
    "s-ub", "s-ub-uo", "s-uo", "s-uo-us", "s-us", "s-us-ui", "s-ui", "s-ui-ml", "s-ml",
];
pub(crate) const SHORT_TP_BANDS: &[&str] = &[
    "s-ml-", "s-ml-di", "s-di", "s-di-ds", "s-ds", "s-ds-do", "s-do", "s-do-db", "s-db",
];

// Position-owner ids (which entry strategy opened the position). Exits route per owner;
// see docs/strategy-arbitration.md. Base = AUTO/STAIR/TREND/REENTRY, FC = fast-candle scalp.
pub(crate) const OWN_NONE: u8 = 0;
pub(crate) const OWN_AUTO: u8 = 1;
pub(crate) const OWN_STAIR: u8 = 2;
pub(crate) const OWN_TREND: u8 = 3;
pub(crate) const OWN_REENTRY: u8 = 4;
pub(crate) const OWN_FC: u8 = 5;

/// Map an opening order's mark to its owning entry strategy.
pub(crate) fn owner_from_mark(mark: &str) -> u8 {
    if mark.starts_with("fcs") {
        OWN_FC
    } else if mark.starts_with("trend-") {
        OWN_TREND
    } else if mark.starts_with("re-") {
        OWN_REENTRY
    } else if mark.starts_with("stair") || mark.starts_with("dca-") {
        OWN_STAIR
    } else {
        OWN_AUTO // auto-l and anything else base
    }
}
