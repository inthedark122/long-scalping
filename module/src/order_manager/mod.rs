mod shared;
mod single;

use crate::state::EngineState;
use lte_strategy_bridge::abi::{
    Direction, ModuleInput, ModuleOrderStatus, ModuleOutput, OrderRole,
};

pub fn handle_order_update(
    input: &ModuleInput,
    state: &mut EngineState,
    direction: Direction,
    status: ModuleOrderStatus,
    role: OrderRole,
    fill_price: f64,
    filled_qty: f64,
    mark: &Option<String>,
) -> ModuleOutput {
    single::SingleOrderManager::new().handle_order_update(
        input, state, direction, status, role, fill_price, filled_qty, mark,
    )
}
