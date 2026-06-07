//! Debug string formatters: compact per-event diagnostic suffixes read back
//! from `list_bots` last_debug. Read-only over `State`.

use crate::sizing::uncertainty_damp;
use crate::state::{CascadeState, State, TfMovState};

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

// v0.57: runtime debug builder. Given a bitmask `sel`, returns the FULL-detail topic blocks for
// the selected presets, joined into one suffix. Driven by the persistent `debug_info` config —
// appended to every event line (init/ind/sug) so the API snapshot always carries the chosen view,
// readable straight from `list_bots` last_debug. Switching is a config push applied on the next
// tick (context accumulation), so a focused look is two switches: set preset → read → set 0.
// Reads only `state`, so it is identical on any tick/event — also ready to back a one-shot
// request action if/when a request→response channel to the bot exists. Empty when `sel == 0`.
//   1 V · 2 MRC · 4 BIG · 8 CAS · 16 DMP  (add to combine; 31 = ALL)
pub(crate) fn debug_builder(state: &State, sel: i64) -> String {
    if sel == 0 {
        return String::new();
    }
    let mut parts: Vec<String> = Vec::new();
    if sel & 1 != 0 {
        // V-tracker — same compact per-line table the v-tracker skill parses (`vt=` token).
        parts.push(format!("vt={}", vt_debug(&state.v_lines)));
    }
    if sel & 2 != 0 {
        // 1m MRC band price map (low→high) + 5m mean-line.
        parts.push(format!(
            "mrc=db{:.2} do{:.2} ds{:.2} di{:.2} ml{:.2} ui{:.2} us{:.2} uo{:.2} ub{:.2} | 5ml{:.2}",
            state.mrc_db, state.mrc_do, state.mrc_ds, state.mrc_di, state.mrc_ml,
            state.mrc_ui, state.mrc_us, state.mrc_uo, state.mrc_ub, state.mrc5_ml,
        ));
    }
    if sel & 4 != 0 {
        // Big-line first-reaction bracket + stuck-side exits.
        parts.push(format!("big=l{}", state.long_big_tested as u8,));
    }
    if sel & 8 != 0 {
        // Movement cascade / current run leg / stair-cross ladder / 5m hard-break.
        parts.push(format!(
            "cas=dir{}/run{}t lc{} b5={}",
            state.cas1.cas_dir, state.run_ticks, state.long_stair_cross, state.mrc5_break,
        ));
    }
    if sel & 16 != 0 {
        parts.push(format!(
            "dmp={:.2} dd={:.2}",
            uncertainty_damp(state),
            state.risk_drawdown
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" | sw[{}]", parts.join(" "))
    }
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
