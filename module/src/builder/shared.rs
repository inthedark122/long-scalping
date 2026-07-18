use super::policy::StrategyPolicy;
use crate::detection::{cascade::CascadeSignal, mrc, sideways};
use crate::model::{
    EntryGate, EntryReason, EntryWait, EntryWaitSolution, ExitKind, MarketState, RegimeSnapshot,
    TradeDecision,
};
use crate::prediction;
use crate::state::EngineState;
use lte_strategy_bridge::sug_info::SuggestionInfo;

pub(super) fn should_close_on_direction_change_impl(
    policy: StrategyPolicy,
    input: &lte_strategy_bridge::abi::ModuleInput,
    state: &EngineState,
) -> bool {
    let _ = (input, state);
    policy.direction_change_close
}

pub(super) fn refine_common(
    state: &mut EngineState,
    snapshot: &crate::model::IndicatorSnapshot,
    sug_snapshot: &crate::model::SugInfoSnapshot,
    sideway: &sideways::SidewaySignal,
    regime: RegimeSnapshot,
    cascade_tf: &CascadeSignal,
    cascade_sug: &CascadeSignal,
    sug_info: &SuggestionInfo,
    mut decision: TradeDecision,
    apply_direction_lock: bool,
) -> TradeDecision {
    if state.position.side == crate::model::PositionState::Flat
        && state.exit_memory.kind == Some(ExitKind::StopLoss)
    {
        if let Some(plan) = prediction::sideway_entry_plan(snapshot, sideway, cascade_sug) {
            if sideway.active {
                decision.market_state = plan.market_state;
                decision.entry = EntryGate::Open {
                    notes: plan.notes.clone(),
                };
                decision.entry_reason = Some(EntryReason::SidewayScalp {
                    price: plan.price,
                    block_low: sideway.block_low,
                    block_high: sideway.block_high,
                    started_at_ms: sug_info.system_time as u64,
                    source: format!("{:?}", sideway.source),
                });
            }
        }
    }

    if matches!(decision.entry, EntryGate::Open { .. })
        && decision.entry_reason.is_none()
        && sideway.active
    {
        if let Some(plan) = prediction::sideway_entry_plan(snapshot, sideway, cascade_sug) {
            decision.market_state = plan.market_state;
            decision.entry = EntryGate::Open {
                notes: plan.notes.clone(),
            };
            decision.entry_reason = Some(EntryReason::SidewayScalp {
                price: plan.price,
                block_low: sideway.block_low,
                block_high: sideway.block_high,
                started_at_ms: sug_info.system_time as u64,
                source: format!("{:?}", sideway.source),
            });
        }
    }

    if matches!(decision.entry, EntryGate::Open { .. }) && decision.entry_reason.is_none() {
        let entry_side = match decision.market_state {
            MarketState::Long => crate::model::PositionState::Long,
            MarketState::Short => crate::model::PositionState::Short,
            MarketState::Sideways => crate::model::PositionState::Flat,
        };
        if let Some(plan) = prediction::sug_entry_plan(
            snapshot,
            sug_snapshot,
            state,
            cascade_sug,
            entry_side,
            state.cfg.pullback_ratio() > 0.0,
        ) {
            let _ = (plan.target, plan.band);
            decision.market_state = plan.market_state;
            decision.entry = EntryGate::Open {
                notes: plan.notes.clone(),
            };
            decision.entry_reason = Some(EntryReason::SugInfoScalp {
                price: plan.entry,
                started_at_ms: sug_info.system_time as u64,
            });
        }
    }

    if matches!(decision.entry, EntryGate::Open { .. })
        && decision.entry_reason.is_none()
        && sug_snapshot.sug_candle.is_none()
    {
        if let Some(plan) = prediction::movement_entry_plan(snapshot, sug_snapshot, cascade_sug) {
            decision.market_state = plan.market_state;
            decision.entry = EntryGate::Open {
                notes: plan.notes.clone(),
            };
            decision.entry_reason = Some(EntryReason::SugInfoMovement {
                price: plan.price,
                started_at_ms: sug_info.system_time as u64,
            });
        }
    }

    let mrc1 = mrc::analyze(&snapshot.tf1m.mrc, snapshot.tf1m.candle.close);
    if decision.market_state == MarketState::Sideways
        && mrc1.bullish_structure
        && cascade_tf.dir > 0
    {
        decision = TradeDecision {
            market_state: MarketState::Long,
            entry: match decision.entry {
                EntryGate::Open { mut notes } => {
                    notes.push("1m-recovery".to_string());
                    EntryGate::Open { notes }
                }
                EntryGate::Wait(mut wait) => {
                    wait.notes.push("1m-recovery".to_string());
                    EntryGate::Wait(wait)
                }
            },
            entry_reason: decision.entry_reason.clone(),
        };
    }

    if apply_direction_lock {
        let locked_direction = state.direction_lock.direction;
        if locked_direction != MarketState::Sideways && decision.market_state != locked_direction {
            decision = TradeDecision {
                market_state: locked_direction,
                entry: EntryGate::Wait(EntryWait {
                    reason: "one-way-lock".to_string(),
                    solutions: vec![EntryWaitSolution::WaitForDirectionBreak {
                        source: "direction-lock".to_string(),
                        direction: locked_direction,
                        break_price: state.direction_lock.break_price,
                    }],
                    notes: vec![
                        format!("lock={}", state.direction_lock.label()),
                        format!("price={:.4}", snapshot.tf1m.candle.close),
                        "same-side-only".to_string(),
                    ],
                }),
                entry_reason: None,
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
