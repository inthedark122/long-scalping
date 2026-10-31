//! Order-construction helpers: build the ABI open/close/limit/TP payloads.

use lte_strategy_bridge::abi::{
    Direction, ModuleClosePosition, ModuleOpenPosition, ModuleOrderSide, ModuleOrderType,
    ModulePlaceOrder, ModuleUpdatePosition,
};

pub(crate) fn entry_order_mark(direction: Direction) -> &'static str {
    match direction {
        Direction::Long => "structural-scalping-entry-long",
        Direction::Short => "structural-scalping-entry-short",
        Direction::Both => "structural-scalping-entry-both",
    }
}

pub(crate) fn entry_order_marks() -> Vec<String> {
    [Direction::Long, Direction::Short]
        .into_iter()
        .map(|direction| entry_order_mark(direction).to_string())
        .collect()
}

pub(crate) fn close(direction: Direction, reason: &str) -> ModuleClosePosition {
    ModuleClosePosition {
        direction,
        reason: reason.to_string(),
    }
}

pub(crate) fn tp_mark(direction: Direction) -> String {
    match direction {
        Direction::Long => "tp-long".to_string(),
        Direction::Short => "tp-short".to_string(),
        Direction::Both => "tp-both".to_string(),
    }
}

#[allow(dead_code)]
pub(crate) fn limit_order(
    direction: Direction,
    amount: f64,
    price: f64,
    take_profit: Option<f64>,
    stop_loss: Option<f64>,
    mark: &str,
    note: &str,
) -> ModulePlaceOrder {
    ModulePlaceOrder {
        direction,
        amount: Some(amount),
        qty: None,
        enter_price: price,
        take_profit,
        stop_loss,
        note: Some(note.to_string()),
        mark: mark.to_string(),
        order_side: match direction {
            Direction::Long => ModuleOrderSide::Buy,
            Direction::Short => ModuleOrderSide::Sell,
            _ => ModuleOrderSide::Buy,
        },
        reduce_only: false,
    }
}

/// Immediate open at market. The engine uses this for forced-entry modes so we
/// do not wait for a resting maker fill before the position exists.
pub(crate) fn market_open(
    direction: Direction,
    amount: f64,
    enter: f64,
    take_profit: Option<f64>,
    stop_loss: Option<f64>,
    note: &str,
) -> ModuleOpenPosition {
    ModuleOpenPosition {
        direction,
        amount: Some(amount),
        qty: None,
        enter_price: Some(enter),
        order_type: ModuleOrderType::Market,
        take_profit,
        stop_loss,
        note: note.to_string(),
    }
}

#[allow(dead_code)]
pub(crate) fn tp_limit_order(
    direction: Direction,
    qty: f64,
    price: f64,
    mark: &str,
    note: Option<&str>,
) -> ModulePlaceOrder {
    ModulePlaceOrder {
        direction,
        amount: None,
        qty: Some(qty),
        enter_price: price,
        take_profit: None,
        stop_loss: None,
        note: note.map(|value| value.to_string()),
        mark: mark.to_string(),
        order_side: match direction {
            Direction::Long => ModuleOrderSide::Sell,
            Direction::Short => ModuleOrderSide::Buy,
            _ => ModuleOrderSide::Buy,
        },
        reduce_only: true,
    }
}

pub(crate) fn update_position(
    direction: Direction,
    enter_price: Option<f64>,
) -> ModuleUpdatePosition {
    ModuleUpdatePosition {
        direction,
        enter_price,
        take_profit: None,
        stop_loss: None,
    }
}

pub(crate) fn update_position_stop_loss(
    direction: Direction,
    stop_loss: f64,
) -> ModuleUpdatePosition {
    ModuleUpdatePosition {
        direction,
        enter_price: None,
        take_profit: None,
        stop_loss: Some(stop_loss),
    }
}
