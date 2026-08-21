use crate::detection::cascade::{CascadeTradeState, MovementPhase};
use crate::detection::mrc_crossing::MrcPrediction;
use crate::detection::mrc_pattern_classification::{
    MrcPatternKind, StaircaseLifecycle, StaircaseTransition,
};
use crate::detection::mrc_prediction::{self, MrcLinePlan, MrcPredictionContext};
use crate::detection::vlines::MrcCrossEvent;
use crate::state::{MrcCrossingState, MrcForecastStats, MrcMovementSnapshot};

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
    /// Compact execution information for every MRC destination line.
    pub line_plans: [MrcLinePlan; mrc_prediction::MRC_LINE_COUNT],
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
    pub stair_sequence: Vec<u8>,
    pub stair_price_move_pct: f64,
    pub stair_favorable_move_pct: f64,
    pub stair_adverse_move_pct: f64,
    pub stair_favorable_extreme_price: f64,
    pub stair_favorable_extreme_time_ms: u64,
    pub stair_adverse_extreme_price: f64,
    pub stair_adverse_extreme_time_ms: u64,
    pub staircase_lifecycle: Option<StaircaseLifecycle>,
    pub staircase_transition: Option<StaircaseTransition>,
    pub staircase_protected_line: Option<u8>,
    pub staircase_extreme_line: Option<u8>,
    pub cascade_enabled: bool,
    pub cascade_direction: i8,
    pub cascade_speed_pct_per_min: Option<f64>,
    pub cascade_phase: MovementPhase,
    pub cascade_potential_move_pct: Option<f64>,
    pub cascade_adverse_move_pct: Option<f64>,
    pub cascade_trade_state: CascadeTradeState,
    pub estimated_time_ms: Option<f64>,
    pub favorable_move_pct: Option<f64>,
    pub adverse_move_pct: Option<f64>,
    pub effective_move_pct: Option<f64>,
    pub effective_entry_price: Option<f64>,
    pub predicted_end_price: Option<f64>,
    pub realized_duration_ms: Option<u64>,
    pub realized_movement_direction: Option<i8>,
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
    pub volume_gain: f64,
    pub volume_gain_min: f64,
    pub volume_gain_max: f64,
    pub buy_sell_gain: f64,
}

impl MrcCrossAnalysis {
    fn levels_tradeable(&self) -> bool {
        matches!(self.trade_direction, 1 | -1)
            && self
                .effective_entry_price
                .zip(self.target_price)
                .zip(self.stop_price)
                .is_some_and(|((entry, target), stop)| {
                    crate::detection::mrc_crossing::valid_price_levels_for_pattern(
                        self.trade_direction,
                        entry,
                        target,
                        stop,
                        None,
                        self.pattern_kind,
                    )
                })
    }

    pub fn tradeable(&self) -> bool {
        // Stairs is the execution pattern. Its structural direction and
        // directional levels are the entry authority; cascade remains
        // diagnostic context instead of suppressing nearly every valid leg.
        if matches!(
            self.pattern_kind,
            Some(MrcPatternKind::StairsUp | MrcPatternKind::StairsDown)
        ) {
            return !matches!(
                self.staircase_lifecycle,
                Some(StaircaseLifecycle::Invalidated)
            ) && self.levels_tradeable();
        }
        if matches!(
            self.staircase_lifecycle,
            Some(StaircaseLifecycle::Invalidated | StaircaseLifecycle::Stalled)
        ) {
            return false;
        }
        if !self.cascade_trade_state.allows_entry() {
            return false;
        }
        let barrier_allows_trade = match self.barrier_state {
            BarrierState::Approaching => false,
            BarrierState::Rejected => true,
            BarrierState::None | BarrierState::ConfirmedBreak => true,
        };
        barrier_allows_trade && self.levels_tradeable()
    }

    /// A forecast uses the learned favorable excursion to justify an
    /// anticipatory entry before the next line is crossed. Its entry is still
    /// validated against the current MRC target and stop levels.
    pub fn forecast_tradeable(&self) -> bool {
        if matches!(
            self.pattern_kind,
            Some(MrcPatternKind::StairsUp | MrcPatternKind::StairsDown)
        ) {
            return !matches!(
                self.staircase_lifecycle,
                Some(StaircaseLifecycle::Invalidated)
            ) && self.levels_tradeable();
        }
        if matches!(
            self.staircase_lifecycle,
            Some(StaircaseLifecycle::Invalidated | StaircaseLifecycle::Stalled)
        ) {
            return false;
        }
        if !self.cascade_trade_state.allows_entry() {
            return false;
        }
        self.levels_tradeable()
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

/// Classify a completed price trace using the same rule used by route
/// statistics: a movement is profitable when its favorable excursion is
/// larger than its absolute adverse excursion. The returned percentage is
/// the excursion that determined the classification.
pub fn movement_outcome(favorable_pct: f64, adverse_pct: f64) -> Option<(&'static str, f64)> {
    if !favorable_pct.is_finite() || !adverse_pct.is_finite() {
        return None;
    }
    if favorable_pct > adverse_pct.abs() {
        Some(("profitable", favorable_pct))
    } else {
        Some(("negative", adverse_pct))
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

pub(crate) fn adjacent_line(source: u8, direction: i8) -> Option<u8> {
    if direction > 0 {
        (source < 8).then_some(source + 1)
    } else {
        source.checked_sub(1)
    }
}

pub(crate) fn mrc_stop_line(
    lines: [f64; 9],
    source_line: u8,
    entry_price: f64,
    direction: i8,
) -> Option<u8> {
    if !entry_price.is_finite() || entry_price <= 0.0 || !matches!(direction, 1 | -1) {
        return None;
    }

    // The source line is the line that generated the setup. It is never a
    // stop: a fill can already be beyond it, making it an invalid and
    // misleading stop level. Walk away from the source on the adverse MRC
    // side until the level is actually beyond the fill price.
    let mut candidate = adjacent_line(source_line, -direction)?;
    loop {
        let price = lines.get(candidate as usize).copied()?;
        let is_adverse = price.is_finite()
            && price > 0.0
            && if direction > 0 {
                price < entry_price
            } else {
                price > entry_price
            };
        if is_adverse {
            return Some(candidate);
        }
        candidate = adjacent_line(candidate, -direction)?;
    }
}

pub fn build(
    event: &MrcCrossEvent,
    prediction: Option<&MrcPrediction>,
    lines: [f64; 9],
    crossing: &MrcCrossingState,
    volume: (f64, f64, f64, f64),
) -> MrcCrossAnalysis {
    build_with_context(
        event,
        prediction,
        lines,
        crossing,
        volume,
        MrcPredictionContext::default(),
    )
}

pub fn build_with_context(
    event: &MrcCrossEvent,
    prediction: Option<&MrcPrediction>,
    lines: [f64; 9],
    crossing: &MrcCrossingState,
    volume: (f64, f64, f64, f64),
    context: MrcPredictionContext,
) -> MrcCrossAnalysis {
    let (
        realized_duration_ms,
        realized_movement_direction,
        realized_favorable_move_pct,
        realized_adverse_move_pct,
    ) = event
        .completed_movement
        .map(|movement| {
            let (favorable, adverse) = movement_percentages(movement);
            (
                Some(movement.duration_ms),
                Some(movement.direction),
                Some(favorable),
                Some(adverse),
            )
        })
        .unwrap_or((None, None, None, None));
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

    let mut barrier_line = opposite_barrier(event, crossing);
    let mut barrier_state = match barrier_line {
        Some(barrier) if barrier == event.line_idx => BarrierState::ConfirmedBreak,
        Some(barrier) if line_direction(event.line_idx, barrier) == event.direction => {
            BarrierState::Approaching
        }
        Some(_) => BarrierState::Rejected,
        None => BarrierState::None,
    };
    let (volume_gain, volume_gain_min, volume_gain_max, buy_sell_gain) = volume;
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
        line_plans: [MrcLinePlan::default(); mrc_prediction::MRC_LINE_COUNT],
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
        stair_sequence: Vec::new(),
        stair_price_move_pct: 0.0,
        stair_favorable_move_pct: 0.0,
        stair_adverse_move_pct: 0.0,
        stair_favorable_extreme_price: 0.0,
        stair_favorable_extreme_time_ms: 0,
        stair_adverse_extreme_price: 0.0,
        stair_adverse_extreme_time_ms: 0,
        staircase_lifecycle: None,
        staircase_transition: None,
        staircase_protected_line: None,
        staircase_extreme_line: None,
        cascade_enabled: context.cascade_enabled,
        cascade_direction: context.cascade_direction,
        cascade_speed_pct_per_min: context.cascade_speed_pct_per_min,
        cascade_phase: context.cascade_phase,
        cascade_potential_move_pct: context.cascade_potential_move_pct,
        cascade_adverse_move_pct: context.cascade_adverse_move_pct,
        cascade_trade_state: CascadeTradeState::NotConfigured,
        estimated_time_ms: None,
        favorable_move_pct: None,
        adverse_move_pct: None,
        effective_move_pct: None,
        effective_entry_price: None,
        predicted_end_price: None,
        realized_duration_ms,
        realized_movement_direction,
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
        volume_gain,
        volume_gain_min,
        volume_gain_max,
        buy_sell_gain,
    };

    let fallback_stop = mrc_stop_line(lines, event.line_idx, event.line, event.direction)
        .and_then(|line| lines.get(line as usize).copied());
    analysis.line_plans = mrc_prediction::build_line_plans(
        lines,
        event.line_idx,
        event.line,
        event.direction,
        fallback_stop,
        None,
        [MrcForecastStats::default(); mrc_prediction::MRC_LINE_COUNT],
        context,
    );
    let Some(prediction) = prediction else {
        return analysis;
    };
    let predicted_direction = prediction.pattern_direction;
    let trade_direction = predicted_direction;

    // FAST_BREAK_RETURN has its own barrier: the exact line B that was
    // crossed and then rejected/revisited in A -> B -> A. The generic
    // crossing state can describe another movement and must not replace this
    // pattern-specific support/resistance line.
    if prediction.pattern_kind == MrcPatternKind::FastBreakReturn {
        if let Some(failed_break_line) = prediction.failed_break_line {
            barrier_line = Some(failed_break_line);
            barrier_state = if predicted_direction == -prediction.failed_break_direction {
                BarrierState::Rejected
            } else if predicted_direction == prediction.failed_break_direction {
                BarrierState::Approaching
            } else {
                BarrierState::None
            };
        }
    }

    analysis.barrier_line = barrier_line;
    analysis.barrier_price = barrier_line.and_then(|line| lines.get(line as usize).copied());
    analysis.barrier_state = barrier_state;
    let favorable_move_pct = prediction.stats.average_favorable_pct();
    let adverse_move_pct = prediction.stats.average_adverse_pct();
    let effective_move_pct = favorable_move_pct.filter(|value| value.is_finite() && *value > 0.0);
    // A staircase entry is anchored to the line that produced the
    // prediction. Do not shift it to the next inner line: that changes the
    // setup's entry identity (for example, down_outer becomes down_small)
    // and can make the stop/target pair describe a different movement than
    // the one being traded. Other patterns continue to use the live market
    // price as their entry reference.
    let entry_price = if matches!(
        prediction.pattern_kind,
        MrcPatternKind::StairsUp | MrcPatternKind::StairsDown
    ) {
        event.line
    } else {
        event.price
    };
    let predicted_end_price = effective_move_pct
        .map(|move_pct| entry_price * (1.0 + trade_direction as f64 * move_pct / 100.0));
    // The learned movement is the trigger distance from the crossed line. If
    // that distance already reaches the predicted line, that line is the
    // entry barrier, not a profitable exit. Continue to the next line for
    // the actual trade target; the original prediction remains visible as
    // `target_line`.
    let learned_line_price = lines
        .get(prediction.target_line as usize)
        .copied()
        .unwrap_or(0.0);
    let mut trade_target_line = Some(prediction.target_line);
    while trade_target_line.is_some_and(|line| {
        lines.get(line as usize).copied().is_some_and(|price| {
            price.is_finite()
                && price > 0.0
                && if trade_direction > 0 {
                    entry_price >= price
                } else {
                    entry_price <= price
                }
        })
    }) {
        trade_target_line = trade_target_line.and_then(|line| adjacent_line(line, trade_direction));
    }
    let (expected_price_low, expected_price_high) = projected_price_range(
        entry_price,
        trade_direction,
        favorable_move_pct.unwrap_or(0.0),
        adverse_move_pct.unwrap_or(0.0),
    );
    let predicted_target_price =
        trade_target_line.and_then(|line| lines.get(line as usize).copied());
    let stop_loss_line = mrc_stop_line(lines, event.line_idx, entry_price, trade_direction);
    let predicted_stop_price = stop_loss_line.and_then(|line| lines.get(line as usize).copied());
    let target_move_pct = predicted_target_price
        .filter(|price| price.is_finite() && *price > 0.0 && entry_price > 0.0)
        .map(|price| (price - entry_price).abs() / entry_price * 100.0)
        .unwrap_or(0.0);
    let stop_move_pct = predicted_stop_price
        .filter(|price| price.is_finite() && *price > 0.0 && entry_price > 0.0)
        .map(|price| (price - entry_price).abs() / entry_price * 100.0)
        .unwrap_or(0.0);
    let cascade_trade_state = if matches!(
        prediction.pattern_kind,
        MrcPatternKind::StairsUp | MrcPatternKind::StairsDown
    ) {
        context.trade_state(trade_direction, target_move_pct, stop_move_pct)
    } else {
        CascadeTradeState::NotConfigured
    };
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
    analysis.stair_sequence = prediction.stair_sequence.clone();
    analysis.stair_price_move_pct = prediction.stair_price_move_pct;
    analysis.stair_favorable_move_pct = prediction.stair_favorable_move_pct;
    analysis.stair_adverse_move_pct = prediction.stair_adverse_move_pct;
    analysis.stair_favorable_extreme_price = prediction.favorable_extreme_price;
    analysis.stair_favorable_extreme_time_ms = prediction.favorable_extreme_time_ms;
    analysis.stair_adverse_extreme_price = prediction.adverse_extreme_price;
    analysis.stair_adverse_extreme_time_ms = prediction.adverse_extreme_time_ms;
    analysis.staircase_lifecycle = prediction.staircase_lifecycle;
    analysis.staircase_transition = prediction.staircase_transition;
    analysis.staircase_protected_line = prediction.staircase_protected_line;
    analysis.staircase_extreme_line = prediction.staircase_extreme_line;
    analysis.cascade_trade_state = cascade_trade_state;
    analysis.estimated_time_ms = prediction.stats.average_duration_ms();
    analysis.favorable_move_pct = favorable_move_pct;
    analysis.adverse_move_pct = adverse_move_pct;
    analysis.effective_move_pct = effective_move_pct;
    analysis.effective_entry_price = Some(entry_price);
    analysis.predicted_end_price = predicted_end_price;
    analysis.line_plans = mrc_prediction::build_line_plans(
        lines,
        event.line_idx,
        entry_price,
        trade_direction,
        predicted_stop_price,
        trade_target_line,
        prediction.line_stats,
        context,
    );
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
