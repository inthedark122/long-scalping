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
        "t={} p={:.4} 5m={:?} 15m={} 1h={:?} risk={} cap_init={:.2}% budget={:.4} pos={:?} vml={}/{} exec={} reason={}",
        state.ticks,
        snapshot.tf1m.candle.close,
        regime.primary,
        if regime.fallback_used { "fb" } else { "na" },
        regime.context_1h,
        if risk.risk_high { "hi" } else { "lo" },
        state.cfg.cap_init,
        state.trade_budget,
        state.position.side,
        vt.long_side.count,
        vt.short_side.count,
        execution_note,
        decision.reason,
    )
}
