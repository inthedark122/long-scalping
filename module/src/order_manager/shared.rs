use crate::model::{EntryReason, ExitKind, ExitMemory, PositionState};
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
    input: &ModuleInput,
    state: &EngineState,
    direction: Direction,
    status: ModuleOrderStatus,
    role: OrderRole,
    fill_price: f64,
    filled_qty: f64,
    mark: &Option<String>,
) -> String {
    if matches!(&role, OrderRole::StopLoss)
        && matches!(
            status,
            ModuleOrderStatus::Filled | ModuleOrderStatus::PartiallyFilled
        )
    {
        let side = crate::state::dir_to_position(direction);
        let position = match side {
            PositionState::Long => input.positions.long.as_ref(),
            PositionState::Short => input.positions.short.as_ref(),
            PositionState::Flat => None,
        };
        let entry_price = if state.position.entry_price > 0.0 {
            state.position.entry_price
        } else {
            position
                .map(|position| position.entry_price)
                .filter(|price| *price > 0.0)
                .unwrap_or(state.position_context.entry_price)
        };
        let stop_loss = if state.exit_memory.stop_loss > 0.0 {
            state.exit_memory.stop_loss
        } else {
            state.position.stop_loss
        };
        let fill_price = if fill_price > 0.0 {
            fill_price
        } else {
            input.price
        };
        let fill_beyond_stop = match side {
            PositionState::Long => stop_loss - fill_price,
            PositionState::Short => fill_price - stop_loss,
            PositionState::Flat => f64::NAN,
        };
        let market_beyond_stop = match side {
            PositionState::Long => stop_loss - input.price,
            PositionState::Short => input.price - stop_loss,
            PositionState::Flat => f64::NAN,
        };
        let adverse_slippage = match side {
            PositionState::Long => input.price - fill_price,
            PositionState::Short => fill_price - input.price,
            PositionState::Flat => f64::NAN,
        };
        let entry_plan = state
            .position
            .entry_reason
            .as_ref()
            .or(state.position_context.entry_reason.as_ref())
            .map(|reason| match reason {
                EntryReason::MrcCrossing {
                    pattern_kind,
                    source_line,
                    stop_price,
                    ..
                } => format!(
                    "pattern={} entry_source_line={} planned_sl={:.8}",
                    pattern_kind, source_line, stop_price
                ),
                EntryReason::CandleTrend {
                    direction,
                    stop_line,
                    stop_price,
                    ..
                } => format!(
                    "pattern={} stop_line={} planned_sl={:.8}",
                    if *direction > 0 {
                        "candle-uptrend"
                    } else {
                        "candle-downtrend"
                    },
                    stop_line,
                    stop_price
                ),
            })
            .unwrap_or_else(|| "entry_plan=missing".to_string());

        return format!(
            "stop-loss-fill status={} side={:?} entry={:.8} active_sl={:.8} fill={:.8} market={:.8} fill_beyond_sl={:+.8} market_beyond_sl={:+.8} fill_crossed={} market_crossed={} adverse_slippage={:+.8} qty={:.8} source={} {} mark={:?}",
            status.as_str(),
            side,
            entry_price,
            stop_loss,
            fill_price,
            input.price,
            fill_beyond_stop,
            market_beyond_stop,
            fill_beyond_stop.is_finite() && fill_beyond_stop >= 0.0,
            market_beyond_stop.is_finite() && market_beyond_stop >= 0.0,
            adverse_slippage,
            filled_qty,
            state.position.stop_loss_reason,
            entry_plan,
            mark
        );
    }

    format!("order {:?} {:?} {:?} {:?}", direction, status, role, mark)
}
