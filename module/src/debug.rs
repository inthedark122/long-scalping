use crate::model::{IndicatorSnapshot, RegimeSnapshot, RiskSnapshot, TradeDecision};
use crate::state::EngineState;

pub fn render(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    regime: &RegimeSnapshot,
    risk: &RiskSnapshot,
    decision: &TradeDecision,
    execution_note: &str,
) -> String {
    let vt = &state.vlines[4];
    format!(
        "t={} p={:.4} 5m={:?} 15m={} 1h={:?} risk={} cd={} pos={:?} vml={}/{} exec={} reason={}",
        state.ticks,
        snapshot.tf1m.candle.close,
        regime.primary,
        if regime.fallback_used { "fb" } else { "na" },
        regime.context_1h,
        if risk.risk_high { "hi" } else { "lo" },
        state.cooldown_until.saturating_sub(state.ticks),
        state.position.side,
        vt.long_side.count,
        vt.short_side.count,
        execution_note,
        decision.reason,
    )
}
