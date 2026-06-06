//! Order-construction helpers: build the ABI open/close/limit/TP payloads.

use lte_strategy_bridge::abi::{
    Direction, ModuleClosePosition, ModuleOpenPosition, ModuleOrderSide, ModulePlaceOrder,
};

pub(crate) fn market_open(direction: Direction, amount: f64, note: &str) -> ModuleOpenPosition {
    ModuleOpenPosition {
        direction,
        amount: Some(amount),
        qty: None,
        enter_price: None,
        order_type: "Market".to_string(),
        take_profit: None,
        stop_loss: None,
        note: note.to_string(),
    }
}

pub(crate) fn close(direction: Direction, reason: &str) -> ModuleClosePosition {
    ModuleClosePosition {
        direction,
        reason: reason.to_string(),
    }
}

pub(crate) fn limit_order(
    direction: Direction,
    amount: f64,
    price: f64,
    mark: &str,
) -> ModulePlaceOrder {
    ModulePlaceOrder {
        direction,
        amount: Some(amount),
        qty: None,
        enter_price: price,
        take_profit: None,
        stop_loss: None,
        mark: mark.to_string(),
        order_side: match direction {
            Direction::Long => ModuleOrderSide::Buy,
            Direction::Short => ModuleOrderSide::Sell,
            _ => ModuleOrderSide::Buy,
        },
        reduce_only: false,
    }
}

pub(crate) fn tp_order_with_band(
    direction: Direction,
    amount: f64,
    price: f64,
    mark: &str,
    band: &str,
) -> ModulePlaceOrder {
    ModulePlaceOrder {
        direction,
        amount: Some(amount),
        qty: None,
        enter_price: price,
        take_profit: None,
        stop_loss: None,
        mark: format!("{}:{}", mark, band),
        order_side: match direction {
            Direction::Long => ModuleOrderSide::Sell,
            Direction::Short => ModuleOrderSide::Buy,
            _ => ModuleOrderSide::Buy,
        },
        reduce_only: true,
    }
}
