use crate::model::{RegimeSnapshot, RiskSnapshot, TradeDecision};

pub fn decide(regime: RegimeSnapshot, risk: RiskSnapshot) -> TradeDecision {
    let mut reason = if regime.fallback_used {
        "15m-fallback".to_string()
    } else {
        "5m-primary".to_string()
    };

    if regime.blocked_by_context {
        reason.push_str("|1h-block");
    }
    if risk.risk_high {
        reason.push_str("|risk-high");
    }

    TradeDecision {
        market_state: regime.confirmed,
        allow_entry: !risk.risk_high && !regime.blocked_by_context,
        reason,
    }
}
