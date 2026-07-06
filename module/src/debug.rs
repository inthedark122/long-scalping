use crate::detection::cascade::CascadeSignal;
use crate::model::{IndicatorSnapshot, RegimeSnapshot, RiskSnapshot, TradeDecision};
use crate::state::EngineState;

pub fn render(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    regime: &RegimeSnapshot,
    cascade_1m: &CascadeSignal,
    cascade_sug: &CascadeSignal,
    _risk: &RiskSnapshot,
    decision: &TradeDecision,
    execution_note: &str,
) -> String {
    let vt = &state.vlines[4];
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
    format!(
        "t={} p={:.4} 5m={:?} 15m={:?} 15m_fb={} 1h={:?} cas1={:?} dir1={} start1={} est1={} sug_dir={} sug_ent={:.4} sug_exit={:.4} casSug={:?} dirSug={} startSug={} estSug={} cap_init={:.2}% trading_amount={:.2}% budget={:.4} pos={:?} entry={} entry_reason={} mem={} tp_mode={} tp_mark={} tp={:.4} sl={:.4} vml={}/{} gain_1m={:.4} gain_zone={:?} gain_prog={:.2} gain_building={} gain_late={} exec={}",
        state.ticks,
        snapshot.tf1m.candle.close,
        regime.primary,
        regime.fallback,
        regime.fallback_used,
        regime.context_1h,
        cascade_1m.phase,
        cascade_1m.dir,
        cascade_1m.starting,
        cascade_1m.established,
        snapshot.sug_direction,
        snapshot.sug_candle.map(|c| c.open).unwrap_or(0.0),
        snapshot.sug_candle.map(|c| c.close).unwrap_or(0.0),
        cascade_sug.phase,
        cascade_sug.dir,
        cascade_sug.starting,
        cascade_sug.established,
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
