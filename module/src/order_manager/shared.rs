use crate::model::{ExitKind, ExitMemory};
use crate::state::{dir_to_position, EngineState};
use lte_strategy_bridge::abi::{Direction, ModuleInput, ModuleOrderStatus, OrderRole};

pub(super) fn record_exit_memory(
    input: &ModuleInput,
    state: &mut EngineState,
    direction: Direction,
    role: &OrderRole,
    fill_price: f64,
    filled_qty: f64,
    mark: &Option<String>,
) {
    if matches!(role, OrderRole::TakeProfit) {
        if let Some(mark) = mark.as_deref() {
            let prefix = match direction {
                Direction::Long => "tp-long-",
                Direction::Short => "tp-short-",
                Direction::Both => "",
            };
            let index = if mark == format!("{prefix}far") {
                Some(1)
            } else {
                mark.strip_prefix(prefix)
                    .and_then(|value| value.parse::<usize>().ok())
                    .and_then(|value| (1..=3).contains(&value).then_some(value - 1))
            };
            if let Some(index) = index {
                state.take_profit_allocation.record_fill(
                    dir_to_position(direction),
                    index,
                    filled_qty,
                );
            }
        }
    }
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
