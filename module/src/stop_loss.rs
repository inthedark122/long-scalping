use crate::model::PositionState;

pub fn virtual_stop_loss_hit(side: PositionState, price: f64, stop_loss: f64) -> bool {
    match side {
        PositionState::Long => price <= stop_loss,
        PositionState::Short => price >= stop_loss,
        PositionState::Flat => false,
    }
}

pub fn trailing_recovery_take_profit_with_ticks(
    current_price: f64,
    side: PositionState,
    tick_size: f64,
    ticks: u32,
) -> Option<f64> {
    if !current_price.is_finite() || current_price <= 0.0 {
        return None;
    }
    if !tick_size.is_finite() || tick_size <= 0.0 {
        return None;
    }

    let price = match side {
        PositionState::Long => current_price + ticks.max(1) as f64 * tick_size,
        PositionState::Short => current_price - ticks.max(1) as f64 * tick_size,
        PositionState::Flat => return None,
    };
    (price.is_finite() && price > 0.0).then_some(price)
}
