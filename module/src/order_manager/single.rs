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
        ModuleOutput {
            debug: shared::debug_line(direction, status, role, mark),
            ..Default::default()
        }
    }
}
