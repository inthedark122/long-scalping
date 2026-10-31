use super::shared;
use crate::state::EngineState;
use lte_strategy_bridge::abi::{
    Direction, ModuleInput, ModuleOrderStatus, ModuleOutput, OrderRole,
};

#[derive(Debug, Clone, Default)]
pub struct SingleOrderManager;

impl SingleOrderManager {
    pub fn new() -> Self {
        Self
    }

    pub fn handle_order_update(
        &self,
        input: &ModuleInput,
        state: &mut EngineState,
        direction: Direction,
        status: ModuleOrderStatus,
        role: OrderRole,
        fill_price: f64,
        filled_qty: f64,
        mark: &Option<String>,
    ) -> ModuleOutput {
        shared::record_exit_memory(input, state, direction, &role, fill_price, filled_qty, mark);
        let entry_order_ended = matches!(&role, OrderRole::Entry)
            && matches!(
                status,
                ModuleOrderStatus::Canceled | ModuleOrderStatus::Expired
            );
        let has_active_position = input
            .positions
            .long
            .as_ref()
            .into_iter()
            .chain(input.positions.short.as_ref())
            .any(|position| position.qty > 0.0 || position.notional > 0.0);
        if entry_order_ended
            && !has_active_position
            && state.position.side == crate::model::PositionState::Flat
            && state.position_context.side == crate::model::PositionState::Flat
            && mark.as_deref() == Some(crate::orders::entry_order_mark(direction))
        {
            state.pending_entry_bar = None;
            state.pending_entry_reason = None;
            state.pending_entry_plan = None;
            state.pending_strategy = None;
        }
        ModuleOutput {
            debug: shared::debug_line(
                input, state, direction, status, role, fill_price, filled_qty, mark,
            ),
            ..Default::default()
        }
    }
}
