use crate::detection::cascade::cascade_take_profit_distance;
use crate::detection::levels;
use crate::model::{
    EntryReason, FastCandlePhase, IndicatorSnapshot, PositionState, SugInfoSnapshot,
};
use crate::state::CascadeState;

pub fn virtual_stop_loss_hit(side: PositionState, price: f64, stop_loss: f64) -> bool {
    match side {
        PositionState::Long => price <= stop_loss,
        PositionState::Short => price >= stop_loss,
        PositionState::Flat => false,
    }
}

pub fn breakeven_price(entry_price: f64, side: PositionState, tick_size: f64) -> f64 {
    if entry_price <= 0.0 {
        return 0.0;
    }

    match side {
        PositionState::Long => (entry_price + tick_size).max(entry_price),
        PositionState::Short => (entry_price - tick_size).max(tick_size),
        PositionState::Flat => entry_price,
    }
}

pub fn martingale_recovery_take_profit(
    current_price: f64,
    side: PositionState,
    tick_size: f64,
) -> f64 {
    if current_price <= 0.0 {
        return 0.0;
    }

    let move_ticks = 2.0 * tick_size.max(0.0);
    match side {
        PositionState::Long => current_price + move_ticks,
        PositionState::Short => (current_price - move_ticks).max(tick_size.max(f64::EPSILON)),
        PositionState::Flat => current_price,
    }
}

pub fn trailing_recovery_take_profit(
    current_price: f64,
    side: PositionState,
    tick_size: f64,
) -> Option<f64> {
    if !current_price.is_finite() || current_price <= 0.0 {
        return None;
    }
    if !tick_size.is_finite() || tick_size <= 0.0 {
        return None;
    }

    let price = match side {
        PositionState::Long => current_price + 2.0 * tick_size,
        PositionState::Short => current_price - 2.0 * tick_size,
        PositionState::Flat => return None,
    };
    (price.is_finite() && price > 0.0).then_some(price)
}

pub fn fast_candle_phase(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
) -> FastCandlePhase {
    sug_snapshot
        .fast_candle
        .as_ref()
        .map(|fc| fc.phase(&snapshot.tf1m.candle))
        .unwrap_or(FastCandlePhase::Absent)
}

pub fn projected_move_distance(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    cascade: &CascadeState,
) -> Option<f64> {
    if cascade.samples < 3 || cascade.avg_size <= 0.0 {
        return None;
    }

    let phase = fast_candle_phase(snapshot, sug_snapshot);
    let gain_building = snapshot.tf1m.gain_is_building() || snapshot.tf1m.gain_is_late();
    let active =
        matches!(phase, FastCandlePhase::Building | FastCandlePhase::Tiring) || gain_building;
    if !active {
        return None;
    }

    cascade_take_profit_distance(cascade).or(Some(cascade.avg_size))
}

pub fn support_break_level(
    snapshot: &IndicatorSnapshot,
    side: PositionState,
    entry_reason: Option<&EntryReason>,
) -> Option<f64> {
    if let Some(EntryReason::SidewayScalp {
        block_low,
        block_high,
        ..
    }) = entry_reason
    {
        return match side {
            PositionState::Long => Some(*block_low),
            PositionState::Short => Some(*block_high),
            PositionState::Flat => None,
        };
    }

    let book = levels::structural_levels(snapshot);
    let price = snapshot.tf1m.candle.close.max(f64::EPSILON);
    match side {
        PositionState::Long => book.nearest_below(price),
        PositionState::Short => book.nearest_above(price),
        PositionState::Flat => None,
    }
}
