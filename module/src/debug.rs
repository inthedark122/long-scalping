//! Debug string formatters: compact per-event diagnostic suffixes read back
//! from `list_bots` last_debug. Read-only over `State`.

use crate::state::{CascadeState, TfMovState};

// ── Debug section toggles (debug_info bitmask) ──────────────────────────────
// The whole debug line is assembled from sections, each gated by a bit, so a
// focused look is one config push (set debug_info → read list_bots → set back).
// debug_info == 0 means "use the default compact view" (DBG_DEFAULT).
//   1 CORE · 2 CAS · 4 WIN · 8 RUN · 16 FC · 32 VT · 64 MRC · 128 DMP   (sum to combine)
pub(crate) const DBG_CORE: i64 = 1; // t, p, mode, ln, pnl, ord — the essentials
pub(crate) const DBG_CAS: i64 = 2; // cas1 / cas5 movement cascade
pub(crate) const DBG_WIN: i64 = 4; // window context
pub(crate) const DBG_RUN: i64 = 8; // run leg / sudden-move
pub(crate) const DBG_FC: i64 = 16; // fast-candle + history
pub(crate) const DBG_VT: i64 = 32; // V-line tracker table
pub(crate) const DBG_MRC: i64 = 64; // full MRC band price map
pub(crate) const DBG_DMP: i64 = 128; // uncertainty damp / drawdown
/// Default compact view when debug_info is left at 0.
pub(crate) const DBG_DEFAULT: i64 = DBG_CORE | DBG_CAS | DBG_WIN;

/// Resolve the active section mask: 0 → the default compact view.
pub(crate) fn dbg_sel(debug_info: i64) -> i64 {
    if debug_info == 0 {
        DBG_DEFAULT
    } else {
        debug_info
    }
}

/// Join the selected sections into one space-piped line, prefixed with `tag`.
pub(crate) fn dbg_join(tag: &str, parts: &[(i64, String)], sel: i64) -> String {
    let body: Vec<&str> = parts
        .iter()
        .filter(|(bit, _)| sel & bit != 0)
        .map(|(_, s)| s.as_str())
        .collect();
    format!("{} {}", tag, body.join(" | "))
}

/// Compact cascade readout: direction, current block size %, size÷avg ratio (how
/// stretched → exhaustion/window), candle count, avg block %, and break count.
/// e.g. "L 0.45%/1.2x 3c avg0.38% b2" — long block, 0.45% range, 1.2× the typical move.
pub(crate) fn cascade_debug(cas: &CascadeState, price: f64) -> String {
    let dir = match cas.cas_dir {
        1 => "L",
        -1 => "S",
        _ => "-",
    };
    let size = cas.cas_high - cas.cas_low;
    let size_pct = if price > 0.0 { size / price * 100.0 } else { 0.0 };
    let ratio = if cas.avg_size > 0.0 {
        size / cas.avg_size
    } else {
        0.0
    };
    let avg_pct = if price > 0.0 {
        cas.avg_size / price * 100.0
    } else {
        0.0
    };
    format!(
        "{} {:.2}%/{:.1}x {}c avg{:.2}% b{}",
        dir, size_pct, ratio, cas.cas_candles, avg_pct, cas.break_n
    )
}

pub(crate) fn fc_history_debug(history: &[crate::state::FcEvent]) -> String {
    if history.is_empty() {
        return "fc_hist=[]".to_string();
    }
    let entries: Vec<String> = history
        .iter()
        .map(|e| {
            let d = if e.dir > 0 { "^" } else { "v" };
            if e.pct >= 0.01 {
                format!("{}{:.1}%{}t", d, e.pct, e.dur)
            } else {
                format!("{}{}t", d, e.dur)
            }
        })
        .collect();
    format!("fc_hist=[{}]", entries.join(" "))
}

pub(crate) fn fast_candle_debug(fc: &Option<lte_strategy_bridge::SuggestionInfoCandle>) -> String {
    match fc {
        None => "fc=.".to_string(),
        Some(c) => {
            let dir = if c.exit >= c.enter { "+" } else { "-" };
            let pct = (c.exit - c.enter).abs() / c.enter.max(f64::EPSILON) * 100.0;
            let range = (c.max - c.min) / c.enter.max(f64::EPSILON) * 100.0;
            let net_qty = c.qty - c.qtym; // net USDT (positive = net buy)
            format!(
                "fc={}{:.2}%(rng={:.2}% net_qty={:.2})",
                dir, pct, range, net_qty
            )
        }
    }
}

pub(crate) fn vt_debug(v_lines: &[crate::state::VLineTracker; 9]) -> String {
    // Show 6 key lines: UB(8) US(6) UI(5) ML(4) DI(3) DB(0)
    let lines = [
        ("UB", &v_lines[8]),
        ("US", &v_lines[6]),
        ("UI", &v_lines[5]),
        ("ML", &v_lines[4]),
        ("DI", &v_lines[3]),
        ("DB", &v_lines[0]),
    ];
    lines
        .iter()
        .map(|(name, vl)| {
            let side = match vl.was_above {
                Some(true) => "^",
                Some(false) => "v",
                None => "-",
            };
            format!(
                "{}{}L{}({:.0}%{})S{}({:.0}%{})",
                name,
                side,
                vl.long_side.count,
                vl.long_side.last_power,
                if vl.long_side.exhausted { "!" } else { "" },
                vl.short_side.count,
                vl.short_side.last_power,
                if vl.short_side.exhausted { "!" } else { "" },
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn tf_debug(tf: &TfMovState) -> String {
    let r = match tf.regime {
        2 => "TUP",
        -2 => "TDN",
        1 => "SWY",
        _ => "UNK",
    };
    format!(
        "{}(m={:.0} c={} f={}/{} C={}/{} n={})",
        r, tf.mov, tf.cross, tf.floor, tf.floor_age, tf.ceiling, tf.ceiling_age, tf.candles
    )
}
