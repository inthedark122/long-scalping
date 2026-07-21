use crate::detection::{cascade::CascadeSignal, sideways};
use crate::model::{
    EntryGate, EntryOpenReason, EntryReason, EntryWait, ExitKind, MarketState, RegimeSnapshot,
    TradeDecision,
};
use crate::prediction;
use crate::state::EngineState;
use lte_strategy_bridge::sug_info::SuggestionInfo;

pub(super) fn refine_common(
    state: &mut EngineState,
    snapshot: &crate::model::IndicatorSnapshot,
    sug_snapshot: &crate::model::SugInfoSnapshot,
    sideway: &sideways::SidewaySignal,
    regime: RegimeSnapshot,
    cascade: &CascadeSignal,
    _sug_info: &SuggestionInfo,
    mut decision: TradeDecision,
) -> TradeDecision {
    if state.position.side == crate::model::PositionState::Flat
        && state.exit_memory.kind == Some(ExitKind::StopLoss)
    {
        if let Some(plan) = prediction::sideway_entry_plan(snapshot, sideway, cascade) {
            if sideway.active {
                decision.market_state = plan.market_state;
                state.direction_source = "sideway-plan".to_string();
                state.direction_reason = format!(
                    "sideway-plan:market={:?}:source={:?}",
                    plan.market_state, sideway.source
                );
                decision.entry = EntryGate::Open {
                    notes: plan.notes.clone(),
                    reason: EntryOpenReason::Entry(EntryReason::SidewayScalp {
                        price: plan.price,
                        block_low: sideway.block_low,
                        block_high: sideway.block_high,
                        source: format!("{:?}", sideway.source),
                    }),
                };
            }
        }
    }

    if matches!(
        decision.entry,
        EntryGate::Open {
            reason: EntryOpenReason::Entry(EntryReason::Structural { .. }),
            ..
        }
    ) && sideway.active
    {
        if let Some(plan) = prediction::sideway_entry_plan(snapshot, sideway, cascade) {
            decision.market_state = plan.market_state;
            state.direction_source = "sideway-plan".to_string();
            state.direction_reason = format!(
                "sideway-plan:market={:?}:source={:?}",
                plan.market_state, sideway.source
            );
            decision.entry = EntryGate::Open {
                notes: plan.notes.clone(),
                reason: EntryOpenReason::Entry(EntryReason::SidewayScalp {
                    price: plan.price,
                    block_low: sideway.block_low,
                    block_high: sideway.block_high,
                    source: format!("{:?}", sideway.source),
                }),
            };
        }
    }

    if matches!(
        decision.entry,
        EntryGate::Open {
            reason: EntryOpenReason::Entry(EntryReason::Structural { .. }),
            ..
        }
    ) {
        let entry_side = match decision.market_state {
            MarketState::Long => crate::model::PositionState::Long,
            MarketState::Short => crate::model::PositionState::Short,
            MarketState::Sideways => crate::model::PositionState::Flat,
        };
        if let Some(plan) = prediction::sug_entry_plan(snapshot, sug_snapshot, cascade, entry_side)
        {
            let _ = (plan.target, plan.band);
            decision.market_state = plan.market_state;
            state.direction_source = "sug-info-plan".to_string();
            state.direction_reason = format!("sug-info-plan:market={:?}", plan.market_state);
            decision.entry = EntryGate::Open {
                notes: plan.notes.clone(),
                reason: EntryOpenReason::Entry(EntryReason::SugInfoScalp { price: plan.entry }),
            };
        }
    }

    if matches!(
        decision.entry,
        EntryGate::Open {
            reason: EntryOpenReason::Entry(EntryReason::Structural { .. }),
            ..
        }
    ) {
        if let Some(plan) = prediction::movement_entry_plan(
            snapshot,
            sug_snapshot,
            cascade,
            &state.cascade_tf,
            decision.market_state,
        ) {
            decision.market_state = plan.market_state;
            state.direction_source = "movement-plan".to_string();
            state.direction_reason = format!(
                "movement-plan:move_dir={}:market={:?}",
                plan.movement_direction, plan.market_state
            );
            decision.entry = EntryGate::Open {
                notes: plan.notes.clone(),
                reason: EntryOpenReason::Entry(EntryReason::SugInfoMovement { price: plan.price }),
            };
        }
    }

    if regime.blocked_by_context {
        decision.entry = EntryGate::Wait(EntryWait {
            reason: "1h-context-block".to_string(),
            solutions: vec![],
            notes: vec!["1h context blocks overconfident entry".to_string()],
        });
    }

    decision
}
