use crate::detection::cascade::{CascadeSignal, MovementPhase};
use crate::detection::mrc_crossing::MrcPrediction;
use crate::detection::mrc_pattern_classification::MrcPatternKind;
use crate::detection::vlines::MrcCrossEvent;
use crate::state::{MrcCrossingState, MrcMovementSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierState {
    None,
    Approaching,
    ConfirmedBreak,
    Rejected,
}

impl BarrierState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Approaching => "approaching",
            Self::ConfirmedBreak => "confirmed-break",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MrcCrossAnalysis {
    pub line_idx: u8,
    pub line: f64,
    pub market_price: f64,
    pub mrc_min: f64,
    pub mrc_max: f64,
    pub direction: i8,
    pub now_ms: u64,
    pub barrier_line: Option<u8>,
    pub barrier_price: Option<f64>,
    pub barrier_state: BarrierState,
    pub target_line: Option<u8>,
    pub predicted_line_price: Option<f64>,
    pub trade_target_line: Option<u8>,
    /// The MRC line used as the actual take-profit level for the executed side.
    /// This is separate from `target_line`, which is the learned destination.
    pub take_profit_line: Option<u8>,
    /// The MRC line used as the actual stop-loss level for the executed side.
    pub stop_loss_line: Option<u8>,
    pub target_price: Option<f64>,
    pub stop_price: Option<f64>,
    pub predicted_direction: i8,
    pub trade_direction: i8,
    pub pattern_kind: Option<MrcPatternKind>,
    pub pattern_steps: u32,
    pub pattern_elapsed_ms: u64,
    pub probability: Option<f64>,
    pub estimated_time_ms: Option<f64>,
    pub favorable_move_pct: Option<f64>,
    pub adverse_move_pct: Option<f64>,
    pub effective_move_pct: Option<f64>,
    pub effective_entry_price: Option<f64>,
    pub predicted_end_price: Option<f64>,
    pub realized_duration_ms: Option<u64>,
    pub realized_favorable_move_pct: Option<f64>,
    pub realized_adverse_move_pct: Option<f64>,
    pub realized_price_high: Option<f64>,
    pub realized_price_low: Option<f64>,
    pub realized_target_low: Option<f64>,
    pub realized_target_high: Option<f64>,
    pub realized_mrc_low: Option<f64>,
    pub realized_mrc_high: Option<f64>,
    pub pattern: Vec<u8>,
    pub pattern_samples: u32,
    pub context_samples: u32,
    pub no_pullback_count: u32,
    pub reverted_line: Option<u8>,
    pub target_samples: u32,
    pub movement_samples: u32,
    pub next_line_crosses: u32,
    pub support_resistance_moves: u32,
    pub price_only_moves: u32,
    pub successful_moves: u32,
    pub failed_moves: u32,
    pub movement_success_rate: Option<f64>,
    pub expected_value_pct: Option<f64>,
    pub expected_price_low: Option<f64>,
    pub expected_price_high: Option<f64>,
    pub cascade_dir: i8,
    pub cascade_strength: f64,
    pub cascade_movement_position: f64,
    pub cascade_stretched: bool,
    pub cascade_phase: MovementPhase,
    pub cascade_move_pct: Option<f64>,
    pub volume_gain: f64,
    pub volume_gain_min: f64,
    pub volume_gain_max: f64,
    pub buy_sell_gain: f64,
}

impl MrcCrossAnalysis {
    pub fn tradeable(&self) -> bool {
        let barrier_allows_trade = match self.barrier_state {
            BarrierState::Approaching => false,
            BarrierState::Rejected => true,
            BarrierState::None | BarrierState::ConfirmedBreak => true,
        };
        barrier_allows_trade
            && matches!(self.trade_direction, 1 | -1)
            && self
                .effective_entry_price
                .zip(self.target_price)
                .zip(self.stop_price)
                .is_some_and(|((entry, target), stop)| {
                    if self.trade_direction > 0 {
                        target > entry && stop < entry
                    } else {
                        target < entry && stop > entry
                    }
                })
    }
}

pub fn movement_percentages(movement: MrcMovementSnapshot) -> (f64, f64) {
    let start_price = if movement.market_start_price > 0.0 {
        movement.market_start_price
    } else {
        movement.start_price
    };
    if start_price <= 0.0 || movement.direction == 0 {
        return (0.0, 0.0);
    }
    if movement.direction > 0 {
        (
            (movement.high - start_price) / start_price * 100.0,
            (movement.low - start_price) / start_price * 100.0,
        )
    } else {
        (
            (start_price - movement.low) / start_price * 100.0,
            (start_price - movement.high) / start_price * 100.0,
        )
    }
}

fn line_direction(from: u8, to: u8) -> i8 {
    match to.cmp(&from) {
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
    }
}

pub fn opposite_barrier(event: &MrcCrossEvent, crossing: &MrcCrossingState) -> Option<u8> {
    let barrier = if event.direction > 0 {
        crossing.reverted_short_line
    } else {
        crossing.reverted_long_line
    }?;
    (barrier < 9).then_some(barrier)
}

pub fn projected_price_range(
    market_price: f64,
    direction: i8,
    favorable_pct: f64,
    adverse_pct: f64,
) -> (f64, f64) {
    if market_price <= 0.0 || !market_price.is_finite() {
        return (market_price, market_price);
    }
    if direction > 0 {
        (
            market_price * (1.0 + adverse_pct / 100.0),
            market_price * (1.0 + favorable_pct / 100.0),
        )
    } else {
        (
            market_price * (1.0 - favorable_pct / 100.0),
            market_price * (1.0 - adverse_pct / 100.0),
        )
    }
}

fn adjacent_line(source: u8, direction: i8) -> Option<u8> {
    if direction > 0 {
        (source < 8).then_some(source + 1)
    } else {
        source.checked_sub(1)
    }
}

fn nearest_adverse_line(lines: [f64; 9], entry_price: f64, direction: i8) -> Option<u8> {
    if !entry_price.is_finite() || entry_price <= 0.0 {
        return None;
    }
    lines
        .into_iter()
        .enumerate()
        .filter(|(_, line)| line.is_finite() && *line > 0.0)
        .filter(|(_, line)| {
            if direction > 0 {
                *line < entry_price
            } else {
                *line > entry_price
            }
        })
        .min_by(|(_, left), (_, right)| {
            let left_distance = (*left - entry_price).abs();
            let right_distance = (*right - entry_price).abs();
            left_distance
                .partial_cmp(&right_distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(line, _)| line as u8)
}

pub fn build(
    event: &MrcCrossEvent,
    prediction: Option<&MrcPrediction>,
    lines: [f64; 9],
    crossing: &MrcCrossingState,
    cascade: CascadeSignal,
    volume: (f64, f64, f64, f64),
) -> MrcCrossAnalysis {
    let (realized_duration_ms, realized_favorable_move_pct, realized_adverse_move_pct) = event
        .completed_movement
        .map(|movement| {
            let (favorable, adverse) = movement_percentages(movement);
            (Some(movement.duration_ms), Some(favorable), Some(adverse))
        })
        .unwrap_or((None, None, None));
    let (
        realized_price_high,
        realized_price_low,
        realized_target_low,
        realized_target_high,
        realized_mrc_low,
        realized_mrc_high,
    ) = event
        .completed_movement
        .map(|movement| {
            (
                Some(movement.high),
                Some(movement.low),
                Some(movement.target_low),
                Some(movement.target_high),
                Some(movement.mrc_low),
                Some(movement.mrc_high),
            )
        })
        .unwrap_or((None, None, None, None, None, None));

    let barrier_line = opposite_barrier(event, crossing);
    let barrier_state = match barrier_line {
        Some(barrier) if barrier == event.line_idx => BarrierState::ConfirmedBreak,
        Some(barrier) if line_direction(event.line_idx, barrier) == event.direction => {
            BarrierState::Approaching
        }
        Some(_) => BarrierState::Rejected,
        None => BarrierState::None,
    };
    let (volume_gain, volume_gain_min, volume_gain_max, buy_sell_gain) = volume;
    let cascade_move_pct = (cascade.average_size > 0.0 && event.price > 0.0)
        .then_some(cascade.average_size / event.price * 100.0);
    let mut analysis = MrcCrossAnalysis {
        line_idx: event.line_idx,
        line: event.line,
        market_price: event.price,
        mrc_min: event.mrc_low,
        mrc_max: event.mrc_high,
        direction: event.direction,
        now_ms: event.now_ms,
        barrier_line,
        barrier_price: barrier_line.and_then(|line| lines.get(line as usize).copied()),
        barrier_state,
        target_line: None,
        predicted_line_price: None,
        trade_target_line: None,
        take_profit_line: None,
        stop_loss_line: None,
        target_price: None,
        stop_price: None,
        predicted_direction: 0,
        trade_direction: 0,
        pattern_kind: None,
        pattern_steps: 0,
        pattern_elapsed_ms: 0,
        probability: None,
        estimated_time_ms: None,
        favorable_move_pct: None,
        adverse_move_pct: None,
        effective_move_pct: None,
        effective_entry_price: None,
        predicted_end_price: None,
        realized_duration_ms,
        realized_favorable_move_pct,
        realized_adverse_move_pct,
        realized_price_high,
        realized_price_low,
        realized_target_low,
        realized_target_high,
        realized_mrc_low,
        realized_mrc_high,
        pattern: Vec::new(),
        pattern_samples: 0,
        context_samples: 0,
        no_pullback_count: event.no_pullback_count,
        reverted_line: event.reverted_line,
        target_samples: 0,
        movement_samples: 0,
        next_line_crosses: 0,
        support_resistance_moves: 0,
        price_only_moves: 0,
        successful_moves: 0,
        failed_moves: 0,
        movement_success_rate: None,
        expected_value_pct: None,
        expected_price_low: None,
        expected_price_high: None,
        cascade_dir: cascade.dir,
        cascade_strength: cascade.strength,
        cascade_movement_position: cascade.movement_position,
        cascade_stretched: cascade.stretched,
        cascade_phase: cascade.phase,
        cascade_move_pct,
        volume_gain,
        volume_gain_min,
        volume_gain_max,
        buy_sell_gain,
    };

    let Some(prediction) = prediction else {
        return analysis;
    };
    let predicted_direction = prediction.pattern_direction;
    let trade_direction = predicted_direction;
    let favorable_move_pct = prediction.stats.average_favorable_pct();
    let adverse_move_pct = prediction.stats.average_adverse_pct();
    let effective_move_pct = favorable_move_pct
        .or(cascade_move_pct)
        .filter(|value| value.is_finite() && *value > 0.0);
    let predicted_end_price = effective_move_pct
        .map(|move_pct| event.line * (1.0 + trade_direction as f64 * move_pct / 100.0));
    let entry_price = event.price;
    // The learned movement is the trigger distance from the crossed line. If
    // that distance already reaches the predicted line, that line is the
    // entry barrier, not a profitable exit. Continue to the next line for
    // the actual trade target; the original prediction remains visible as
    // `target_line`.
    let learned_line_price = lines
        .get(prediction.target_line as usize)
        .copied()
        .unwrap_or(0.0);
    let reaches_predicted_target = entry_price.is_finite() && entry_price > 0.0 && {
        if trade_direction > 0 {
            entry_price >= learned_line_price
        } else {
            entry_price <= learned_line_price
        }
    };
    let trade_target_line = if reaches_predicted_target {
        adjacent_line(prediction.target_line, trade_direction)
    } else {
        Some(prediction.target_line)
    };
    let (expected_price_low, expected_price_high) = projected_price_range(
        event.price,
        trade_direction,
        favorable_move_pct.unwrap_or(0.0),
        adverse_move_pct.unwrap_or(0.0),
    );
    let predicted_target_price =
        trade_target_line.and_then(|line| lines.get(line as usize).copied());
    let stop_loss_line = nearest_adverse_line(lines, entry_price, trade_direction)
        .or_else(|| adjacent_line(event.line_idx, -trade_direction));
    let predicted_stop_price = stop_loss_line.and_then(|line| lines.get(line as usize).copied());
    analysis.target_line = Some(prediction.target_line);
    analysis.predicted_line_price = Some(learned_line_price);
    analysis.trade_target_line = trade_target_line;
    analysis.take_profit_line = trade_target_line;
    analysis.stop_loss_line = stop_loss_line;
    analysis.target_price = predicted_target_price;
    analysis.stop_price = predicted_stop_price;
    analysis.predicted_direction = predicted_direction;
    analysis.trade_direction = trade_direction;
    analysis.pattern_kind = Some(prediction.pattern_kind);
    analysis.pattern_steps = prediction.pattern_steps;
    analysis.pattern_elapsed_ms = prediction.pattern_elapsed_ms;
    analysis.probability = Some(prediction.probability);
    analysis.estimated_time_ms = prediction.stats.average_duration_ms();
    analysis.favorable_move_pct = favorable_move_pct;
    analysis.adverse_move_pct = adverse_move_pct;
    analysis.effective_move_pct = effective_move_pct;
    analysis.effective_entry_price = Some(entry_price);
    analysis.predicted_end_price = predicted_end_price;
    analysis.pattern = prediction.pattern.clone();
    analysis.pattern_samples = prediction.samples;
    analysis.context_samples = prediction.context_samples;
    analysis.target_samples = prediction.stats.count;
    analysis.movement_samples = prediction.stats.movement_samples;
    analysis.next_line_crosses = prediction.next_line_crosses;
    analysis.support_resistance_moves = prediction.support_resistance_moves;
    analysis.price_only_moves = prediction.price_only_moves;
    analysis.successful_moves = prediction.stats.success_count;
    analysis.failed_moves = prediction.stats.failure_count;
    analysis.movement_success_rate = prediction.stats.movement_success_rate();
    analysis.expected_value_pct = prediction.stats.expected_value_pct();
    analysis.expected_price_low = Some(expected_price_low);
    analysis.expected_price_high = Some(expected_price_high);
    analysis
}
