use crate::model::{ExitKind, ExitMemory};
use crate::state::EngineState;
use lte_strategy_bridge::abi::{Direction, ModuleInput, ModuleOrderStatus, OrderRole};

pub(super) fn record_exit_memory(
    input: &ModuleInput,
    state: &mut EngineState,
    direction: Direction,
    role: &OrderRole,
    fill_price: f64,
) {
    if !matches!(role, OrderRole::TakeProfit | OrderRole::StopLoss) {
        return;
    }

    state.exit_memory = ExitMemory {
        side: crate::state::dir_to_position(direction),
        kind: Some(match role {
            OrderRole::TakeProfit => ExitKind::TakeProfit,
            OrderRole::StopLoss => ExitKind::StopLoss,
            OrderRole::Entry | OrderRole::Other => ExitKind::TakeProfit,
        }),
        price: if fill_price > 0.0 {
            fill_price
        } else {
            input.price
        },
        take_profit: state.position.take_profit,
        stop_loss: state.position.stop_loss,
        reliable: state.position.exit_tp_reliable,
        tick: state.ticks,
    };
}

pub(super) fn debug_line(
    direction: Direction,
    status: ModuleOrderStatus,
    role: OrderRole,
    mark: &Option<String>,
) -> String {
    format!("order {:?} {:?} {:?} {:?}", direction, status, role, mark)
}
