use super::{EntryDecision, EntryPlan};
use crate::model::{EntryType, PositionState};
use crate::orders::{entry_order_mark, entry_order_marks, limit_order, market_open};
use crate::risk;
use crate::state::EngineState;
use lte_strategy_bridge::abi::{Direction, ModuleInput, ModuleOpenPosition, ModulePlaceOrder};

#[derive(Debug, Default)]
pub struct Execution {
    pub open_position: Option<ModuleOpenPosition>,
    pub place_order: Option<ModulePlaceOrder>,
    pub cancel_orders: Vec<String>,
    pub status: String,
    pub constraint: Option<String>,
}

pub fn execute(input: &ModuleInput, state: &mut EngineState, decision: EntryDecision) -> Execution {
    if state.position.side != PositionState::Flat {
        return Execution {
            status: "position-open".to_string(),
            cancel_orders: entry_order_marks(),
            ..Default::default()
        };
    }

    match decision {
        EntryDecision::Enter(plan) => execute_plan(input, state, plan),
        EntryDecision::Wait { strategy, reason } => {
            state.pending_entry_bar = None;
            state.pending_strategy = None;
            state.pending_entry_plan = None;
            Execution {
                status: format!("{}:{reason}", strategy.as_str()),
                cancel_orders: entry_order_marks(),
                ..Default::default()
            }
        }
    }
}

fn execute_plan(input: &ModuleInput, state: &mut EngineState, plan: EntryPlan) -> Execution {
    if !direction_allowed(state.allowed_direction, plan.direction) {
        clear_pending(state);
        return Execution {
            status: "direction-not-allowed".to_string(),
            cancel_orders: entry_order_marks(),
            ..Default::default()
        };
    }

    let entry_type = plan.entry_type.unwrap_or_else(|| state.cfg.entry_type());
    let pending_order = state.pending_entry_bar.is_some() && state.pending_strategy.is_some();
    let same_pending_plan = pending_order
        && state.pending_strategy == Some(plan.strategy)
        && state.pending_entry_plan.is_some_and(|pending| {
            pending.take_profit == plan.take_profit.unwrap_or_default()
                && pending.stop_loss == plan.stop_loss
        });
    if entry_type == EntryType::Limit && same_pending_plan {
        return Execution {
            status: "limit-entry-pending".to_string(),
            ..Default::default()
        };
    }
    let replacing_pending_order = pending_order;
    if replacing_pending_order {
        clear_pending(state);
    }

    let side = if plan.direction == Direction::Long {
        PositionState::Long
    } else {
        PositionState::Short
    };
    let base_amount = risk::entry_budget(input, state);
    let sizing = match risk::size_entry(
        input,
        state,
        side,
        base_amount,
        plan.entry_price,
        Some(plan.stop_loss),
    ) {
        Ok(sizing) if sizing.amount > 0.0 => sizing,
        Ok(_) => {
            clear_pending(state);
            return Execution {
                status: "entry-size-is-zero".to_string(),
                constraint: Some("entry-size-is-zero".to_string()),
                cancel_orders: entry_order_marks(),
                ..Default::default()
            };
        }
        Err(reason) => {
            clear_pending(state);
            return Execution {
                status: reason.to_string(),
                constraint: Some(reason.to_string()),
                cancel_orders: entry_order_marks(),
                ..Default::default()
            };
        }
    };
    let min_notional = input.symbol_info.min_notional.filter(|value| *value > 0.0);
    if min_notional
        .is_some_and(|minimum| sizing.amount * (input.leverage as f64).max(1.0) < minimum)
    {
        clear_pending(state);
        return Execution {
            status: "below-min-notional".to_string(),
            constraint: Some("below-min-notional".to_string()),
            cancel_orders: entry_order_marks(),
            ..Default::default()
        };
    }

    match entry_type {
        EntryType::Market => {
            state.pending_entry_bar = None;
            state.pending_strategy = Some(plan.strategy);
            record_strategy_movement(state, plan.strategy);
            Execution {
                open_position: Some(market_open(
                    plan.direction,
                    sizing.amount,
                    plan.entry_price,
                    plan.take_profit,
                    Some(plan.stop_loss),
                    &plan.note,
                )),
                cancel_orders: entry_order_marks(),
                status: format!("market-entry:{}:{}", plan.strategy.as_str(), plan.reason),
                ..Default::default()
            }
        }
        EntryType::Limit => {
            state.pending_entry_bar = Some(state.bar_1m);
            state.pending_strategy = Some(plan.strategy);
            state.pending_entry_plan = Some(crate::state::PositionPlanSnapshot {
                take_profit: plan.take_profit.unwrap_or_default(),
                stop_loss: plan.stop_loss,
                tp_reliable: plan.take_profit.is_some(),
            });
            record_strategy_movement(state, plan.strategy);
            let opposite = match plan.direction {
                Direction::Long => Direction::Short,
                Direction::Short => Direction::Long,
                Direction::Both => Direction::Both,
            };
            Execution {
                place_order: Some(limit_order(
                    plan.direction,
                    sizing.amount,
                    plan.entry_price,
                    plan.take_profit,
                    Some(plan.stop_loss),
                    entry_order_mark(plan.direction),
                    &plan.note,
                )),
                cancel_orders: if replacing_pending_order {
                    entry_order_marks()
                } else {
                    vec![entry_order_mark(opposite).to_string()]
                },
                status: format!("limit-entry:{}:{}", plan.strategy.as_str(), plan.reason),
                ..Default::default()
            }
        }
    }
}

fn record_strategy_movement(state: &mut EngineState, strategy: super::StrategyKind) {
    if strategy == super::StrategyKind::Sideways {
        state
            .strategy_runtime
            .record_sideways_movement(state.cascade.movement_start_ms);
    }
}

fn clear_pending(state: &mut EngineState) {
    state.pending_entry_bar = None;
    state.pending_strategy = None;
    state.pending_entry_plan = None;
}

fn direction_allowed(allowed: Direction, requested: Direction) -> bool {
    allowed == Direction::Both || allowed == requested
}
