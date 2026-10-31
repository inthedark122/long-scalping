use super::Forecast;
use crate::detection::mrc_crossing;
use crate::model::EntryReason;

pub fn extend_target_to_forecast(reason: &mut EntryReason, forecast: Forecast) -> bool {
    let direction = mrc_crossing::reason_direction(reason);
    if !forecast.supports(direction) {
        return false;
    }
    let move_pct = forecast.expected_return_pct * direction as f64;
    if !move_pct.is_finite() || move_pct <= 0.0 {
        return false;
    }

    match reason {
        EntryReason::MrcCrossing {
            price,
            target_price,
            stop_price,
            target_is_movement,
            favorable_move_pct,
            ..
        } => {
            let current_move = if *target_is_movement && favorable_move_pct.is_finite() {
                *favorable_move_pct
            } else {
                (*target_price - *price).abs() / price.max(f64::EPSILON) * 100.0
            };
            if move_pct <= current_move {
                return false;
            }
            let target = *price * (1.0 + direction as f64 * move_pct / 100.0);
            if !mrc_crossing::valid_movement_price_levels(
                direction,
                *price,
                target,
                *stop_price,
                None,
            ) {
                return false;
            }
            *target_price = target;
            *target_is_movement = true;
            *favorable_move_pct = move_pct;
            true
        }
        EntryReason::CandleTrend {
            direction,
            price,
            target_price,
            stop_price,
            ..
        } => {
            let current_move = (*target_price - *price).abs() / price.max(f64::EPSILON) * 100.0;
            if move_pct <= current_move {
                return false;
            }
            let target = *price * (1.0 + *direction as f64 * move_pct / 100.0);
            if !mrc_crossing::valid_price_levels_for_pattern(
                *direction,
                *price,
                target,
                *stop_price,
                None,
                None,
            ) {
                return false;
            }
            *target_price = target;
            true
        }
    }
}
