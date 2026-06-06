//! Shared compile-time constants for the strategy module.

pub(crate) const TF: i64 = 60;
pub(crate) const TF_5M: i64 = 300;
pub(crate) const TF_15M: i64 = 900;
pub(crate) const TF_30M: i64 = 1800;
pub(crate) const MRC: &str = "mrc_200";
pub(crate) const SMC_IND: &str = "smc";
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

// trandm = which side is in control (-100 selling .. +100 buying). RIDE the controlling side,
// never fade it ("never buy the dip" — user 2026-05-29). |trandm| >= STRONG → strong control:
// tilt size hard AND block counter-trend auto-opens. |trandm| >= MILD → leaning: tilt size and
// hold the aligned position through a counter relief-bounce/dip (suppress the sudden-move cover).
pub(crate) const TRANDM_STRONG: f64 = 50.0;
pub(crate) const TRANDM_MILD: f64 = 25.0;
// asset_01 = $ (raw × 100) to move trandm 1% — power-of-trades/depth. Raw < ~50k ($5M) = thin/
// coiled (sudden move loaded); raw >= ~200k ($20M) = move spent (expect normal vol). Wired to
// state + debug this version; behavioral gate deferred until calibrated from live observation.

// Band names used to build grid levels — entry fills at a band block that band
pub(crate) const LONG_ENTRY_BANDS: &[&str] = &[
    "l-db", "l-db-do", "l-do", "l-do-ds", "l-ds", "l-ds-di", "l-di", "l-di-ml", "l-ml",
];
pub(crate) const LONG_TP_BANDS: &[&str] = &[
    "l-ml+", "l-ml-ui", "l-ui", "l-ui-us", "l-us", "l-us-uo", "l-uo", "l-uo-ub", "l-ub",
];
