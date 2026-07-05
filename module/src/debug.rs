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
    let fc = snapshot
        .fast_candle
        .map(|fc| format!("{:.4}->{:.4}", fc.candle.open, fc.candle.close))
        .unwrap_or_else(|| "na".to_string());
    format!(
        "t={} p={:.4} 5m={:?} 15m={} 1h={:?} risk={} cap_init={:.2}% trading_amount={:.2}% budget={:.4} pos={:?} vml={}/{} fc={} gain_1m={:.4} gain_zone={:?} gain_prog={:.2} gain_building={} gain_late={} exec={} reason={}",
        state.ticks,
        snapshot.tf1m.candle.close,
        regime.primary,
        if regime.fallback_used { "fb" } else { "na" },
        regime.context_1h,
        if risk.risk_high { "hi" } else { "lo" },
        state.cfg.cap_init,
        state.cfg.trading_amount,
        state.trade_budget,
        state.position.side,
        vt.long_side.count,
        vt.short_side.count,
        fc,
        snapshot.tf1m.volume_gain,
        snapshot.tf1m.gain_zone(),
        snapshot.tf1m.gain_progress_pct(),
        snapshot.tf1m.gain_is_building(),
        snapshot.tf1m.gain_is_late(),
        execution_note,
        decision.reason,
    )
}
