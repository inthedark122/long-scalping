use crate::detection::cascade::CascadeSignal;
use crate::model::{IndicatorSnapshot, RegimeSnapshot, TradeDecision};
use crate::state::EngineState;

pub fn render(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    regime: &RegimeSnapshot,
    cascade_tf: &CascadeSignal,
    cascade_sug: &CascadeSignal,
    decision: &TradeDecision,
    execution_note: &str,
) -> String {
    let vt = &state.vlines[4];
    let cascade_tf_text = format_cascade(cascade_tf, snapshot.tf1m.candle.close);
    let cascade_sug_text = format_cascade(cascade_sug, snapshot.tf1m.candle.close);
    let entry_reason = match &decision.entry_reason {
        Some(crate::model::EntryReason::SugInfoMovement { price, .. }) => {
            format!("sug-info-movement price={:.4}", price)
        }
        None => "none".to_string(),
    };
    let entry = match &decision.entry {
        crate::model::EntryGate::Open { notes } => {
            if notes.is_empty() {
                "open".to_string()
            } else {
                format!("open:{}", notes.join("|"))
            }
        }
        crate::model::EntryGate::Wait(wait) => {
            let mut text = format!("wait:{}", wait.reason);
            if !wait.solutions.is_empty() {
                text.push_str(&format!(" sol={}", wait.solutions.len()));
            }
            if !wait.notes.is_empty() {
                text.push_str(&format!(" notes={}", wait.notes.join("|")));
            }
            text
        }
    };
    let (tp_src, sl_src, tp_struct, sl_struct) = debug_exit_sources(state);
    format!(
        "t={} p={:.4} 5m={:?} 15m={:?} 15m_fb={} 1h={:?} casTf={} casSug={} cap_init={:.2}% trading_amount={:.2}% budget={:.4} pos={:?} entry={} entry_reason={} mem={} tp_mode={} tp_mark={} tp={:.4} sl={:.4} tp_src={} tp_struct={:.4} sl_src={} sl_struct={:.4} vml={}/{} gain_1m={:.4} gain_zone={:?} gain_prog={:.2} gain_building={} gain_late={} exec={}",
        state.ticks,
        snapshot.tf1m.candle.close,
        regime.primary,
        regime.fallback,
        regime.fallback_used,
        regime.context_1h,
        cascade_tf_text,
        cascade_sug_text,
        state.cfg.cap_init,
        state.cfg.trading_amount,
        state.trade_budget,
        state.position.side,
        entry,
        entry_reason,
        state.exit_memory.summary(),
        state.position.exit_mode.as_str(),
        if state.position.tp_mark.is_empty() {
            "na"
        } else {
            &state.position.tp_mark
        },
        state.position.take_profit,
        state.position.stop_loss,
        tp_src,
        tp_struct,
        sl_src,
        sl_struct,
        vt.long_side.count,
        vt.short_side.count,
        snapshot.tf1m.volume_gain,
        snapshot.tf1m.gain_zone(),
        snapshot.tf1m.gain_progress_pct(),
        snapshot.tf1m.gain_is_building(),
        snapshot.tf1m.gain_is_late(),
        execution_note,
    )
}

fn debug_exit_sources(state: &EngineState) -> (&'static str, &'static str, f64, f64) {
    let tp_src = if state.position.exit_mode.as_str() == "trail-after-virtual-sl" {
        "trail"
    } else {
        "fast"
    };
    let sl_src = if state.position.exit_mode.as_str() == "trail-after-virtual-sl" {
        "trail"
    } else {
        "fast"
    };
    (tp_src, sl_src, state.position.take_profit, state.position.stop_loss)
}

fn format_cascade(cascade: &CascadeSignal, price: f64) -> String {
    let cascade_avg_pct = if price > 0.0 {
        cascade.average_size / price * 100.0
    } else {
        0.0
    };
    format!(
        "{:?}:{}:avg={:.2}%:s={:.2}:b={:.4}",
        cascade.phase,
        cascade.dir,
        cascade_avg_pct,
        cascade.strength,
        cascade.break_price,
    )
}
