use crate::model::{EntryGate, EntryWait, RegimeSnapshot, TradeDecision};

pub fn decide(regime: RegimeSnapshot) -> TradeDecision {
    let mut reason = if regime.fallback_used {
        "15m-fallback".to_string()
    } else {
        "5m-primary".to_string()
    };

    if regime.blocked_by_context {
        reason.push_str("|1h-block");
    }

    TradeDecision {
        market_state: regime.confirmed,
        entry: if regime.blocked_by_context {
            EntryGate::Wait(EntryWait {
                reason,
                solutions: vec![],
                notes: vec!["1h context blocks overconfident entry".to_string()],
            })
        } else {
            EntryGate::Open {
                notes: vec![reason],
            }
        },
        entry_reason: None,
    }
}
