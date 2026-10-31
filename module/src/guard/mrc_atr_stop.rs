use crate::model::PositionState;

pub const ATR_MULTIPLIER: f64 = 1.0;

pub fn widen_stop(
    side: PositionState,
    entry_price: f64,
    structural_stop: f64,
    atr: f64,
) -> Option<f64> {
    if !entry_price.is_finite()
        || entry_price <= 0.0
        || !structural_stop.is_finite()
        || structural_stop <= 0.0
        || !atr.is_finite()
        || atr <= 0.0
    {
        return None;
    }

    let buffer = atr * ATR_MULTIPLIER;
    let stop = match side {
        PositionState::Long => (structural_stop - buffer).min(entry_price - buffer),
        PositionState::Short => (structural_stop + buffer).max(entry_price + buffer),
        PositionState::Flat => return None,
    };

    (stop.is_finite() && stop > 0.0).then_some(stop)
}

pub fn ensure_atr_distance_from_entry(
    side: PositionState,
    entry_price: f64,
    planned_stop: f64,
    atr: f64,
) -> Option<f64> {
    if !entry_price.is_finite()
        || entry_price <= 0.0
        || !planned_stop.is_finite()
        || planned_stop <= 0.0
        || !atr.is_finite()
        || atr <= 0.0
    {
        return None;
    }

    let minimum_distance = atr * ATR_MULTIPLIER;
    let stop = match side {
        PositionState::Long => planned_stop.min(entry_price - minimum_distance),
        PositionState::Short => planned_stop.max(entry_price + minimum_distance),
        PositionState::Flat => return None,
    };

    (stop.is_finite() && stop > 0.0).then_some(stop)
}
