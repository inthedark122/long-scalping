//! Current-entry classification for staircase predictions.
//!
//! A staircase describes a directional structure that has already happened.
//! This module decides whether the current price is still a useful place to
//! enter that structure.  It deliberately keeps staircase direction as the
//! authority and uses cascade only for movement phase/timing.

use crate::detection::cascade::{CascadeTradeState, MovementPhase};
use crate::detection::mrc_cross_analysis::{self, MrcCrossAnalysis};
use crate::detection::mrc_pattern_classification::{
    MrcPatternKind, StaircaseLifecycle, StaircaseTransition,
};
use crate::detection::mrc_prediction::MrcPredictionContext;
use crate::detection::mrc_touch::{MrcTouchRegime, MrcTouchSignal};

const TOUCH_SPEED_EPSILON_PCT_PER_MIN: f64 = 0.01;
const TOUCH_PRESSURE_EPSILON: f64 = 0.05;
const TOUCH_INTEREST_EPSILON: f64 = 0.10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StairEntryState {
    Pullback,
    Continuation,
    Retest,
    Top,
    RiskRejected,
    Unsupported,
}

impl StairEntryState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pullback => "pullback",
            Self::Continuation => "continuation",
            Self::Retest => "retest",
            Self::Top => "top",
            Self::RiskRejected => "risk-rejected",
            Self::Unsupported => "unsupported",
        }
    }

    pub const fn opens(self) -> bool {
        matches!(self, Self::Pullback | Self::Continuation | Self::Retest)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReversalState {
    None,
    Potential,
    Confirmed,
}

impl Default for ReversalState {
    fn default() -> Self {
        Self::None
    }
}

impl ReversalState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Potential => "potential",
            Self::Confirmed => "confirmed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StairEntryPlan {
    pub state: StairEntryState,
    pub entry_line: Option<u8>,
    pub target_line: Option<u8>,
    pub stop_line: Option<u8>,
    pub reason: &'static str,
}

fn directional(value: f64, direction: i8) -> f64 {
    value * direction as f64
}

fn supportive_touch(touch: MrcTouchSignal, direction: i8) -> bool {
    let speed = directional(touch.price_speed_pct_per_min, direction);
    let pressure = directional(touch.stairs_pressure, direction);
    let regime = match direction {
        1 => matches!(
            touch.regime,
            MrcTouchRegime::Long | MrcTouchRegime::LongTransition
        ),
        -1 => matches!(
            touch.regime,
            MrcTouchRegime::Short | MrcTouchRegime::ShortTransition
        ),
        _ => false,
    };
    speed > TOUCH_SPEED_EPSILON_PCT_PER_MIN
        && pressure > TOUCH_PRESSURE_EPSILON
        && (regime || touch.interest > TOUCH_INTEREST_EPSILON)
}

fn supportive_higher_timeframe(context: MrcPredictionContext, direction: i8) -> bool {
    context.higher_timeframe_slope == direction
        || (direction > 0 && context.higher_timeframe_bullish)
        || (direction < 0 && context.higher_timeframe_bearish)
}

fn movement_is_active(
    analysis: &MrcCrossAnalysis,
    touch: MrcTouchSignal,
    context: MrcPredictionContext,
) -> bool {
    let direction = analysis.trade_direction;
    let cascade_moving = context.cascade_enabled
        && matches!(
            context.cascade_phase,
            MovementPhase::Starting | MovementPhase::Building
        )
        && context
            .cascade_speed_pct_per_min
            .is_some_and(|speed| speed.is_finite() && speed > 0.0)
        && !context.cascade_stretched;
    let touch_moving = supportive_touch(touch, direction);
    let higher_timeframe_moving = supportive_higher_timeframe(context, direction);

    (cascade_moving && (touch_moving || higher_timeframe_moving))
        || (touch_moving && touch.interest > TOUCH_INTEREST_EPSILON)
}

fn pullback_is_supported(
    analysis: &MrcCrossAnalysis,
    touch: MrcTouchSignal,
    context: MrcPredictionContext,
) -> bool {
    let line_touch = touch
        .line(analysis.line_idx)
        .is_some_and(|line| line.near_ema > 0.10 || line.debounce_ema > 0.05)
        || touch.nearest_line == Some(analysis.line_idx);
    if !line_touch {
        return false;
    }
    supportive_touch(touch, analysis.trade_direction)
        || supportive_higher_timeframe(context, analysis.trade_direction)
        || (touch.sideways_score > 0.25 && touch.interest > TOUCH_INTEREST_EPSILON)
}

fn at_favorable_extreme(analysis: &MrcCrossAnalysis) -> bool {
    analysis
        .staircase_extreme_line
        .is_some_and(|line| line == analysis.line_idx)
}

fn line_is_beyond(line: u8, reference: u8, direction: i8) -> bool {
    if direction > 0 {
        line > reference
    } else {
        line < reference
    }
}

/// A stall is diagnostic evidence for a potential reversal. It does not
/// remove the staircase's own entry route; the confirmed opposite route is
/// emitted separately when the protected swing is crossed. Reaching the
/// outer/big line is also useful reversal evidence because the next move can
/// retrace a much larger part of the range.
pub fn potential_reversal_reason(analysis: &MrcCrossAnalysis) -> Option<&'static str> {
    let direction = analysis.trade_direction;
    if !matches!(
        analysis.pattern_kind,
        Some(MrcPatternKind::StairsUp | MrcPatternKind::StairsDown)
    ) || !matches!(direction, 1 | -1)
        || analysis.staircase_lifecycle == Some(StaircaseLifecycle::Invalidated)
    {
        return None;
    }

    let at_big_line = analysis
        .staircase_extreme_line
        .is_some_and(|line| (direction > 0 && line == 8) || (direction < 0 && line == 0));
    if at_big_line {
        return Some("big-line-exhaustion");
    }

    let stalled = analysis.staircase_lifecycle == Some(StaircaseLifecycle::Stalled)
        || analysis.staircase_transition == Some(StaircaseTransition::AttemptingExtreme);
    if !stalled {
        return None;
    }

    let expected_line_not_reached = analysis
        .trade_target_line
        .zip(analysis.staircase_extreme_line)
        .is_some_and(|(target, extreme)| line_is_beyond(target, extreme, direction));
    if expected_line_not_reached {
        Some("stalled-before-expected-line")
    } else {
        Some("favorable-extreme-stalled")
    }
}

fn forward_line(lines: [f64; 9], line: u8, direction: i8) -> Option<u8> {
    let next = mrc_cross_analysis::adjacent_line(line, direction)?;
    lines
        .get(next as usize)
        .copied()
        .filter(|price| price.is_finite() && *price > 0.0)
        .map(|_| next)
}

fn line_is_forward(entry_line: u8, target_line: u8, direction: i8) -> bool {
    if direction > 0 {
        target_line > entry_line
    } else {
        target_line < entry_line
    }
}

fn target_for_analysis(
    lines: [f64; 9],
    entry_line: u8,
    initial_target: Option<u8>,
    direction: i8,
) -> Option<u8> {
    initial_target
        .filter(|line| line_is_forward(entry_line, *line, direction))
        .or_else(|| forward_line(lines, entry_line, direction))
}

fn plan_for_lines(
    lines: [f64; 9],
    state: StairEntryState,
    entry_line: u8,
    initial_target: Option<u8>,
    direction: i8,
    reason: &'static str,
) -> StairEntryPlan {
    let Some(entry_price) = lines.get(entry_line as usize).copied() else {
        return StairEntryPlan {
            state: StairEntryState::Unsupported,
            entry_line: None,
            target_line: None,
            stop_line: None,
            reason: "entry-line-unavailable",
        };
    };
    let Some(stop_line) =
        mrc_cross_analysis::mrc_stop_line(lines, entry_line, entry_price, direction)
    else {
        return StairEntryPlan {
            state: StairEntryState::Unsupported,
            entry_line: None,
            target_line: None,
            stop_line: None,
            reason: "stop-line-unavailable",
        };
    };
    let Some(target_line) = target_for_analysis(lines, entry_line, initial_target, direction)
    else {
        return StairEntryPlan {
            state: StairEntryState::Unsupported,
            entry_line: None,
            target_line: None,
            stop_line: None,
            reason: "no-positive-route",
        };
    };
    StairEntryPlan {
        state,
        entry_line: Some(entry_line),
        target_line: Some(target_line),
        stop_line: Some(stop_line),
        reason,
    }
}

/// Classify the current staircase entry location and, when appropriate,
/// return a structural plan. The plan is intentionally a small state machine:
/// pullbacks use the current support line, live continuation uses the current
/// line, and an entry at the latest extreme is converted into a retest order.
/// The target line is retained for analysis; execution uses the learned
/// movement from the selected entry price.
pub fn plan(
    analysis: &MrcCrossAnalysis,
    lines: [f64; 9],
    touch: MrcTouchSignal,
    context: MrcPredictionContext,
) -> StairEntryPlan {
    if !matches!(
        analysis.pattern_kind,
        Some(MrcPatternKind::StairsUp | MrcPatternKind::StairsDown)
    ) || !matches!(analysis.trade_direction, 1 | -1)
    {
        return StairEntryPlan {
            state: StairEntryState::Unsupported,
            entry_line: None,
            target_line: None,
            stop_line: None,
            reason: "not-staircase",
        };
    }

    let direction = analysis.trade_direction;
    let hard_risk = matches!(
        analysis.cascade_trade_state,
        CascadeTradeState::AdverseMoveTooLarge | CascadeTradeState::Reversal
    );
    if hard_risk {
        return StairEntryPlan {
            state: StairEntryState::RiskRejected,
            entry_line: None,
            target_line: None,
            stop_line: None,
            reason: "cascade-hard-risk",
        };
    }

    let entry_line = analysis.line_idx;
    let at_extreme = at_favorable_extreme(analysis);
    let moving = movement_is_active(analysis, touch, context);
    let pullback = analysis.staircase_transition
        == Some(StaircaseTransition::PullbackWithinStructure)
        && analysis.staircase_protected_line.is_some()
        && !at_extreme
        && pullback_is_supported(analysis, touch, context);
    let initial_target = analysis.take_profit_line.or(analysis.trade_target_line);

    if pullback {
        return plan_for_lines(
            lines,
            StairEntryState::Pullback,
            entry_line,
            initial_target,
            direction,
            "protected-pullback",
        );
    }

    if at_extreme {
        if !moving {
            return StairEntryPlan {
                state: StairEntryState::Top,
                entry_line: None,
                target_line: None,
                stop_line: None,
                reason: "latest-extreme-without-fresh-movement",
            };
        }

        let Some(retest_line) = mrc_cross_analysis::adjacent_line(entry_line, -direction) else {
            return StairEntryPlan {
                state: StairEntryState::Unsupported,
                entry_line: None,
                target_line: None,
                stop_line: None,
                reason: "no-retest-line",
            };
        };
        return plan_for_lines(
            lines,
            StairEntryState::Retest,
            retest_line,
            initial_target,
            direction,
            "fresh-movement-retest",
        );
    }

    if moving
        && matches!(
            analysis.staircase_transition,
            Some(StaircaseTransition::HigherExtreme | StaircaseTransition::AttemptingExtreme)
        )
        && !matches!(
            analysis.cascade_phase,
            MovementPhase::CounterMove | MovementPhase::Reversal
        )
    {
        return plan_for_lines(
            lines,
            StairEntryState::Continuation,
            entry_line,
            initial_target,
            direction,
            "active-movement-before-extreme",
        );
    }

    StairEntryPlan {
        state: StairEntryState::Top,
        entry_line: None,
        target_line: None,
        stop_line: None,
        reason: "no-current-entry-edge",
    }
}

/// Attach the selected staircase entry plan to the analysis used by both the
/// realtime engine and the historical report generator.
pub fn apply(
    analysis: &mut MrcCrossAnalysis,
    lines: [f64; 9],
    touch: MrcTouchSignal,
    context: MrcPredictionContext,
) {
    if !matches!(
        analysis.pattern_kind,
        Some(MrcPatternKind::StairsUp | MrcPatternKind::StairsDown)
    ) || analysis.transition.is_some()
        || analysis.staircase_lifecycle.is_none()
        || analysis.staircase_transition.is_none()
        || (!context.cascade_enabled && touch.now_ms == 0)
    {
        return;
    }
    if let Some(reason) = potential_reversal_reason(analysis) {
        analysis.reversal_state = ReversalState::Potential;
        analysis.reversal_reason = Some(reason.to_string());
    }
    let plan = plan(analysis, lines, touch, context);
    analysis.stair_entry_state = Some(plan.state);
    analysis.stair_entry_reason = Some(plan.reason.to_string());
    analysis.stair_entry_line = plan.entry_line;
    analysis.stair_entry_approved = plan.state.opens();

    let (Some(entry_line), Some(stop_line)) = (plan.entry_line, plan.stop_line) else {
        return;
    };
    let target_line = plan.target_line;
    let Some(entry_price) = lines.get(entry_line as usize).copied() else {
        analysis.stair_entry_approved = false;
        return;
    };
    let Some(stop_price) = lines.get(stop_line as usize).copied() else {
        analysis.stair_entry_approved = false;
        return;
    };
    analysis.effective_entry_price = Some(entry_price);
    analysis.trade_target_line = target_line;
    analysis.take_profit_line = None;
    analysis.stop_loss_line = Some(stop_line);
    analysis.target_price = None;
    analysis.target_is_movement = true;
    analysis.effective_move_pct = None;
    analysis.predicted_end_price = None;
    analysis.stop_price = Some(stop_price);
    analysis.line_plans = crate::detection::mrc_prediction::build_line_plans(
        lines,
        entry_line,
        entry_price,
        analysis.trade_direction,
        Some(stop_price),
        target_line,
        analysis.line_stats,
        context,
    );
    let Some((selected_line, move_pct)) = crate::detection::mrc_crossing::max_reachable_movement(
        analysis,
        &analysis.line_plans,
        entry_line,
        target_line,
        analysis.trade_direction,
        context,
    ) else {
        analysis.stair_entry_approved = false;
        return;
    };
    analysis.trade_target_line = Some(selected_line);
    analysis.target_price =
        Some(entry_price * (1.0 + analysis.trade_direction as f64 * move_pct / 100.0));
    analysis.effective_move_pct = Some(move_pct);
    analysis.predicted_end_price = analysis.target_price;
    analysis.line_plans = crate::detection::mrc_prediction::build_line_plans(
        lines,
        entry_line,
        entry_price,
        analysis.trade_direction,
        Some(stop_price),
        Some(selected_line),
        analysis.line_stats,
        context,
    );
}
