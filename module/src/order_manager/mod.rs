mod martingale;
mod shared;
mod single;

use crate::model::TradeMode;
use crate::state::EngineState;
use lte_strategy_bridge::abi::{
    Direction, ModuleInput, ModuleOrderStatus, ModuleOutput, OrderRole,
};

#[derive(Debug, Clone)]
pub enum OrderManager {
    Single(single::SingleOrderManager),
    Martingale(martingale::MartingaleOrderManager),
}

impl OrderManager {
    pub fn for_mode(trade_mode: TradeMode) -> Self {
        match trade_mode {
            TradeMode::Single => Self::Single(single::SingleOrderManager::new()),
            TradeMode::Martingale => Self::Martingale(martingale::MartingaleOrderManager::new()),
        }
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
        match self {
            Self::Single(manager) => manager.handle_order_update(
                input, state, direction, status, role, fill_price, filled_qty, mark,
            ),
            Self::Martingale(manager) => manager.handle_order_update(
                input, state, direction, status, role, fill_price, filled_qty, mark,
            ),
        }
    }
}

pub fn handle_order_update(
    trade_mode: TradeMode,
    input: &ModuleInput,
    state: &mut EngineState,
    direction: Direction,
    status: ModuleOrderStatus,
    role: OrderRole,
    fill_price: f64,
    filled_qty: f64,
    mark: &Option<String>,
) -> ModuleOutput {
    OrderManager::for_mode(trade_mode).handle_order_update(
        input, state, direction, status, role, fill_price, filled_qty, mark,
    )
}
