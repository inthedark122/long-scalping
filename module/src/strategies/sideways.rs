use crate::detection::movement_window::{MovementWindow, PriceLocation};
use crate::model::EntryType;
use crate::strategies::{EntryDecision, EntryPlan, StrategyKind};
use lte_strategy_bridge::abi::Direction;

const MIN_RANGE_MOVEMENTS: usize = 5;
const MIN_RANGE_DIRECTION_CHANGES: usize = 4;

pub fn is_range(window: &MovementWindow) -> bool {
    window.movements >= MIN_RANGE_MOVEMENTS
        && window.direction_changes >= MIN_RANGE_DIRECTION_CHANGES
}

pub fn evaluate(
    window: MovementWindow,
    market_price: f64,
    tick_size: f64,
    cascade_direction: i8,
    movement_start_ms: u64,
    last_traded_movement_start_ms: Option<u64>,
    sideways_order_pending: bool,
) -> EntryDecision {
    if movement_start_ms == 0 {
        return wait("cascade-movement-start-unavailable");
    }
    if last_traded_movement_start_ms == Some(movement_start_ms) && !sideways_order_pending {
        return wait("cascade-movement-already-traded");
    }

    let width = window.high - window.low;
    let Some(stop_buffer) = stop_buffer(window, tick_size) else {
        return wait("invalid-stop-distance");
    };
    if !market_price.is_finite() || market_price <= 0.0 || !width.is_finite() || width <= 0.0 {
        return wait("invalid-movement-window");
    }

    let (direction, entry_price, take_profit, stop_loss) = match window.price_location {
        PriceLocation::Inside => {
            let midpoint = window.low + width / 2.0;
            match (market_price <= midpoint, cascade_direction) {
                (true, 1) => (1, window.low, window.high, window.low - stop_buffer),
                (true, -1) => return wait("waiting-for-range-low-rebound"),
                (false, -1) => (-1, window.high, window.low, window.high + stop_buffer),
                (false, 1) => return wait("waiting-for-range-high-reversal"),
                (_, _) => return wait("cascade-direction-unavailable"),
            }
        }
        PriceLocation::Above | PriceLocation::Below => {
            return wait("price-outside-range-window");
        }
    };
    if stop_loss <= 0.0
        || if direction > 0 {
            !(stop_loss < entry_price && take_profit > entry_price)
        } else {
            !(take_profit < entry_price && stop_loss > entry_price)
        }
    {
        return wait("invalid-range-rebound-levels");
    }

    let direction_name = if direction > 0 { "long" } else { "short" };
    let note = format!(
        "strategy=sideways entry=movement-window-boundary direction={} movement_start_ms={} entry={:.8} window_low={:.8} window_high={:.8} target_source=opposite-window-boundary target={:.8} stop_source=one-average-movement-outside-entry-boundary stop={:.8}",
        direction_name,
        movement_start_ms,
        entry_price,
        window.low,
        window.high,
        take_profit,
        stop_loss,
    );
    EntryDecision::Enter(EntryPlan {
        strategy: StrategyKind::Sideways,
        direction: if direction > 0 {
            Direction::Long
        } else {
            Direction::Short
        },
        entry_price,
        take_profit: Some(take_profit),
        stop_loss,
        reason: "movement-window-boundary",
        note,
        entry_type: Some(EntryType::Limit),
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeExitPlan {
    pub take_profit: f64,
    pub stop_loss: f64,
}

pub fn exit_plan(
    side: crate::model::PositionState,
    market_price: f64,
    tick_size: f64,
    window: MovementWindow,
) -> Option<RangeExitPlan> {
    let stop_buffer = stop_buffer(window, tick_size)?;
    let levels = match side {
        crate::model::PositionState::Long => RangeExitPlan {
            take_profit: window.high,
            stop_loss: window.low - stop_buffer,
        },
        crate::model::PositionState::Short => RangeExitPlan {
            take_profit: window.low,
            stop_loss: window.high + stop_buffer,
        },
        crate::model::PositionState::Flat => return None,
    };
    let valid = market_price.is_finite()
        && market_price > 0.0
        && window.low.is_finite()
        && window.high.is_finite()
        && window.low > 0.0
        && window.high > window.low
        && window.low < market_price
        && market_price < window.high
        && levels.take_profit.is_finite()
        && levels.stop_loss.is_finite()
        && levels.take_profit > 0.0
        && levels.stop_loss > 0.0;
    valid.then_some(levels)
}

fn stop_buffer(window: MovementWindow, tick_size: f64) -> Option<f64> {
    if !tick_size.is_finite() || tick_size <= 0.0 {
        return None;
    }
    if window.average_movement_size.is_finite() && window.average_movement_size > 0.0 {
        Some(window.average_movement_size.max(tick_size))
    } else {
        Some(tick_size)
    }
}

fn wait(reason: &'static str) -> EntryDecision {
    EntryDecision::Wait {
        strategy: StrategyKind::Sideways,
        reason,
    }
}
