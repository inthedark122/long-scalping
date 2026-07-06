use crate::model::{IndicatorSnapshot, RegimeSnapshot, RiskSnapshot, TradeDecision};
use crate::state::EngineState;

pub fn render(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    regime: &RegimeSnapshot,
    _risk: &RiskSnapshot,
    decision: &TradeDecision,
    execution_note: &str,
) -> String {
    let vt = &state.vlines[4];
    let fc = snapshot
        .fast_candle
        .map(|fc| format!("{:?} {:.4}->{:.4}", fc.phase(&snapshot.tf1m.candle), fc.candle.open, fc.candle.close))
        .unwrap_or_else(|| "na".to_string());
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
        "t={} p={:.4} 5m={:?} 15m={:?} 15m_fb={} 1h={:?} cap_init={:.2}% trading_amount={:.2}% budget={:.4} pos={:?} entry={} vml={}/{} fc={} gain_1m={:.4} gain_zone={:?} gain_prog={:.2} gain_building={} gain_late={} exec={}",
        state.ticks,
        snapshot.tf1m.candle.close,
        regime.primary,
        regime.fallback,
        regime.fallback_used,
        regime.context_1h,
        state.cfg.cap_init,
        state.cfg.trading_amount,
        state.trade_budget,
        state.position.side,
        entry,
        vt.long_side.count,
        vt.short_side.count,
        fc,
        snapshot.tf1m.volume_gain,
        snapshot.tf1m.gain_zone(),
        snapshot.tf1m.gain_progress_pct(),
        snapshot.tf1m.gain_is_building(),
        snapshot.tf1m.gain_is_late(),
        execution_note,
    )
}
