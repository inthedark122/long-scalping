use crate::exit::{ExitPlan, StopLoss, TakeProfit};
use crate::model::PositionState;

/// Enforces the configured minimum TP and SL distances on a completed plan.
pub fn enforce_minimum_exit_distances(
    plan: ExitPlan,
    entry_price: f64,
    side: PositionState,
    min_tp_percent: f64,
    min_sl_percent: f64,
) -> Option<ExitPlan> {
    if !entry_price.is_finite() || entry_price <= 0.0 {
        return None;
    }

    let min_tp_ratio = min_tp_percent.max(0.0) / 100.0;
    let min_sl_ratio = min_sl_percent.max(0.0) / 100.0;
    let (minimum_target, maximum_stop) = match side {
        PositionState::Long => (
            entry_price * (1.0 + min_tp_ratio),
            entry_price * (1.0 - min_sl_ratio),
        ),
        PositionState::Short => (
            entry_price * (1.0 - min_tp_ratio),
            entry_price * (1.0 + min_sl_ratio),
        ),
        PositionState::Flat => return None,
    };
    if !minimum_target.is_finite()
        || !maximum_stop.is_finite()
        || minimum_target <= 0.0
        || maximum_stop <= 0.0
    {
        return None;
    }

    let (target_price, stop_price) = match side {
        PositionState::Long => (
            plan.take_profit.price().max(minimum_target),
            plan.stop_loss.price().min(maximum_stop),
        ),
        PositionState::Short => (
            plan.take_profit.price().min(minimum_target),
            plan.stop_loss.price().max(maximum_stop),
        ),
        PositionState::Flat => return None,
    };
    if !target_price.is_finite()
        || !stop_price.is_finite()
        || target_price <= 0.0
        || stop_price <= 0.0
    {
        return None;
    }

    let take_profit = match plan.take_profit {
        TakeProfit::Structural { mut level } => {
            level.value = target_price;
            TakeProfit::Structural { level }
        }
        TakeProfit::Movement { .. } => TakeProfit::Movement {
            price: target_price,
        },
        TakeProfit::Sideway { .. } => TakeProfit::Sideway {
            price: target_price,
        },
        TakeProfit::Provided { .. } => TakeProfit::Provided {
            price: target_price,
        },
    };
    let stop_loss = match plan.stop_loss {
        StopLoss::Structural { mut level } => {
            level.value = stop_price;
            StopLoss::Structural { level }
        }
        StopLoss::Sideway { .. } => StopLoss::Sideway { price: stop_price },
        StopLoss::SupportBreak { .. } => StopLoss::SupportBreak { price: stop_price },
        StopLoss::Provided { .. } => StopLoss::Provided { price: stop_price },
    };

    match side {
        PositionState::Long if target_price > entry_price && stop_price < entry_price => {}
        PositionState::Short if target_price < entry_price && stop_price > entry_price => {}
        _ => return None,
    }

    Some(ExitPlan {
        take_profit,
        stop_loss,
        ..plan
    })
}
