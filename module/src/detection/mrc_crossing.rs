use crate::detection::decision::CandidateView;
use crate::detection::mrc_cross_analysis::{self, MrcCrossAnalysis};
use crate::detection::mrc_direction_prediction::{
    self, MrcDirectionContext, MrcDirectionPrediction,
};
use crate::detection::mrc_pattern_classification::{
    self, MrcPatternKind, StaircaseLifecycle, StaircaseTransition,
};
use crate::detection::mrc_prediction;
use crate::detection::mrc_touch::{MrcTouchSignal, MrcTouchState};
use crate::detection::mrc_touch_learning::MrcTouchLearningSignal;
use crate::detection::vlines::MrcCrossEvent;
use crate::model::{
    EntryGate, EntryOpenReason, EntryReason, EntryWait, EntryWaitSolution, MarketState,
};
use crate::state::{
    MrcCrossingModel, MrcCrossingState, MrcForecastStats, MrcHistoryEvent, MrcMovementSnapshot,
    PatternPerformance,
};
use lte_strategy_bridge::abi::ModuleIndicatorValue;
use lte_strategy_bridge::indicator_access::{
    IndicatorField, IndicatorFieldKey, IndicatorHistory, IndicatorKey, TimeframeSec,
};

pub const LINE_LABELS: [&str; 9] = [
    "down_big",
    "down_outer",
    "down_small",
    "down_inner",
    "mean",
    "up_inner",
    "up_small",
    "up_outer",
    "up_big",
];

pub fn entry_gate(state: &MrcCrossingModel) -> EntryGate {
    if let Some(reason) = current_entry(state) {
        return EntryGate::Open {
            notes: vec![
                "strategy=pattern-engine".to_string(),
                pattern_entry_note(&reason),
            ],
            reason: EntryOpenReason::Entry(reason),
        };
    }
    wait_entry_gate(MrcWaitReason::Crossing)
}

pub fn wait_entry_gate(wait_reason: MrcWaitReason) -> EntryGate {
    let (reason, solutions, notes) = match wait_reason {
        MrcWaitReason::Crossing => (
            "pattern-wait-crossing".to_string(),
            vec![EntryWaitSolution::MrcCrossing],
            vec!["waiting for a pattern candidate with a directional target".to_string()],
        ),
        MrcWaitReason::NoDirectionalTarget => (
            "pattern-no-directional-target".to_string(),
            vec![EntryWaitSolution::MrcDirectionalTarget],
            vec!["classified patterns have no valid directional target".to_string()],
        ),
        MrcWaitReason::NoReliablePrediction => (
            "pattern-no-reliable-prediction".to_string(),
            vec![EntryWaitSolution::MrcCrossing],
            vec!["pattern history has no reliable directional prediction".to_string()],
        ),
        MrcWaitReason::Touch => (
            "pattern-wait-mrc-touch".to_string(),
            vec![EntryWaitSolution::MrcCrossing],
            vec!["waiting for a current touch or crossing of the selected MRC line".to_string()],
        ),
    };
    EntryGate::Wait(EntryWait {
        reason,
        solutions,
        notes,
    })
}

fn pattern_entry_note(reason: &EntryReason) -> String {
    let EntryReason::MrcCrossing {
        pattern_kind,
        pattern,
        market_price,
        mrc_min,
        mrc_max,
        pattern_samples,
        context_samples,
        no_pullback_count,
        reverted_line,
        target_samples,
        movement_samples,
        next_line_crosses,
        support_resistance_moves,
        price_only_moves,
        successful_moves,
        failed_moves,
        movement_success_rate,
        expected_value_pct,
        expected_price_low,
        expected_price_high,
        ..
    } = reason;
    format!(
        "pattern={} cycle={} context_samples={} no_pullback={} reverted_line={} market_price={:.4} mrc_range={:.4}..{:.4} transition_samples={} target_samples={} movement_samples={} next_line_crosses={} support_resistance={} price_only_moves={} movement_wins={} movement_losses={} movement_success_rate={:.3} expected_move_pct={:.3} expected_price={:.4}..{:.4} projection={}",
        pattern_kind,
        pattern
            .iter()
            .map(|line| LINE_LABELS[*line as usize])
            .collect::<Vec<_>>()
            .join(">"),
        context_samples,
        no_pullback_count,
        reverted_line
            .map(|line| LINE_LABELS[line as usize])
            .unwrap_or("na"),
        market_price,
        mrc_min,
        mrc_max,
        pattern_samples,
        target_samples,
        movement_samples,
        next_line_crosses,
        support_resistance_moves,
        price_only_moves,
        successful_moves,
        failed_moves,
        movement_success_rate,
        expected_value_pct,
        expected_price_low,
        expected_price_high,
        price_projection(
            reason_direction(reason),
            *mrc_min,
            *mrc_max,
            *expected_price_low,
            *expected_price_high,
        )
    )
}

const MAX_LEARNED_CYCLES: usize = 5;
const MAX_PREDICTION_HISTORY: usize = 10_080;

#[derive(Debug, Clone, PartialEq)]
pub struct MrcPrediction {
    pub pattern_kind: MrcPatternKind,
    pub pattern_direction: i8,
    pub pattern_steps: u32,
    pub pattern_elapsed_ms: u64,
    pub stair_sequence: Vec<u8>,
    pub stair_price_move_pct: f64,
    pub stair_favorable_move_pct: f64,
    pub stair_adverse_move_pct: f64,
    pub staircase_lifecycle: Option<StaircaseLifecycle>,
    pub staircase_transition: Option<StaircaseTransition>,
    pub staircase_protected_line: Option<u8>,
    pub staircase_extreme_line: Option<u8>,
    pub pattern: Vec<u8>,
    pub elapsed_ms: Vec<u64>,
    pub target_line: u8,
    pub samples: u32,
    pub stats: MrcForecastStats,
    /// Route statistics for every destination line from the current source.
    /// These are execution timing/movement inputs, not probabilities.
    pub line_stats: [MrcForecastStats; mrc_prediction::MRC_LINE_COUNT],
    pub next_line_crosses: u32,
    pub support_resistance_moves: u32,
    pub price_only_moves: u32,
    pub context_samples: u32,
    /// For FAST_BREAK_RETURN, the exact line that was crossed and then
    /// returned from. This is the support/resistance barrier being learned.
    pub failed_break_line: Option<u8>,
    /// Direction of the original break, before the return to the source line.
    pub failed_break_direction: i8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MrcWaitReason {
    Crossing,
    NoDirectionalTarget,
    NoReliablePrediction,
    Touch,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternCandidate {
    pub pattern_kind: MrcPatternKind,
    pub score: f64,
    pub analysis: MrcCrossAnalysis,
    pub direction_prediction: MrcDirectionPrediction,
    pub entry: Option<EntryReason>,
    pub forecast: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Observation {
    pub entry: Option<EntryReason>,
    pub wait_reason: Option<MrcWaitReason>,
    pub samples: u32,
    pub events: Vec<MrcCrossAnalysis>,
    /// The latest learned route evaluated against the current candle. This is
    /// intentionally separate from `events`: it is a forecast, not a new
    /// crossing event.
    pub forecast: Option<MrcCrossAnalysis>,
    pub candidates: Vec<PatternCandidate>,
    pub selected_candidate_index: Option<usize>,
    pub selected_pattern: Option<MrcPatternKind>,
    pub selected_score: Option<f64>,
}

impl Observation {
    pub fn selected_candidate(&self) -> Option<&PatternCandidate> {
        if let Some(index) = self.selected_candidate_index {
            return self
                .candidates
                .get(index)
                .filter(|candidate| candidate.entry.is_some());
        }
        let selected_pattern = self.selected_pattern?;
        self.candidates.iter().rev().find(|candidate| {
            candidate.pattern_kind == selected_pattern && candidate.entry.is_some()
        })
    }

    /// Return the analysis that belongs to the selected candidate. The event
    /// list contains every candidate evaluated for the crossing, so its last
    /// item is not necessarily the candidate that was selected for execution.
    pub fn selected_analysis(&self) -> Option<&MrcCrossAnalysis> {
        self.selected_candidate_index
            .and_then(|index| self.candidates.get(index))
            .map(|candidate| &candidate.analysis)
            .or_else(|| self.forecast.as_ref())
            .or_else(|| self.events.last())
    }
}

pub fn valid_price_levels(
    direction: i8,
    entry_price: f64,
    target_price: f64,
    stop_price: f64,
    tick_size: Option<f64>,
) -> bool {
    valid_price_levels_for_pattern(
        direction,
        entry_price,
        target_price,
        stop_price,
        tick_size,
        None,
    )
}

pub fn valid_price_levels_for_pattern(
    direction: i8,
    entry_price: f64,
    target_price: f64,
    stop_price: f64,
    tick_size: Option<f64>,
    pattern_kind: Option<MrcPatternKind>,
) -> bool {
    if !matches!(direction, 1 | -1)
        || !entry_price.is_finite()
        || !target_price.is_finite()
        || !stop_price.is_finite()
        || entry_price <= 0.0
        || target_price <= 0.0
        || stop_price <= 0.0
    {
        return false;
    }

    let directionally_valid =
        if let Some(tick_size) = tick_size.filter(|tick| tick.is_finite() && *tick > 0.0) {
            let entry_ticks = (entry_price / tick_size).round();
            let target_ticks = (target_price / tick_size).round();
            let stop_ticks = (stop_price / tick_size).round();
            if direction > 0 {
                target_ticks > entry_ticks && stop_ticks < entry_ticks
            } else {
                target_ticks < entry_ticks && stop_ticks > entry_ticks
            }
        } else if direction > 0 {
            target_price > entry_price && stop_price < entry_price
        } else {
            target_price < entry_price && stop_price > entry_price
        };

    if !directionally_valid {
        return false;
    }

    // A staircase is an alternating zigzag with progressively deeper legs.
    // Its MRC stop can span more than the next line, so a strict
    // reward/risk check would suppress the very continuation being learned.
    if matches!(
        pattern_kind,
        Some(MrcPatternKind::StairsUp | MrcPatternKind::StairsDown)
    ) {
        return true;
    }

    // An MRC plan is not tradable when its predicted reward is smaller than
    // the MRC risk required to invalidate the setup.
    (target_price - entry_price).abs() >= (stop_price - entry_price).abs()
}

pub fn warmup_samples(state: &MrcCrossingModel) -> u32 {
    let Some(current) = state.history.last() else {
        return 0;
    };
    available_samples(state, current.direction)
}

fn record_movement(
    stats: &mut MrcForecastStats,
    target_line: u8,
    movement: Option<MrcMovementSnapshot>,
    duration_ms: u64,
) {
    stats.count = stats.count.saturating_add(1);
    stats.duration_sum_ms = stats.duration_sum_ms.saturating_add(
        movement
            .map(|movement| movement.duration_ms.max(duration_ms))
            .unwrap_or(duration_ms),
    );
    let Some(movement) = movement else {
        return;
    };
    stats.movement_samples = stats.movement_samples.saturating_add(1);
    let (favorable, adverse) = mrc_cross_analysis::movement_percentages(movement);
    stats.favorable_pct_sum += favorable;
    stats.adverse_pct_sum += adverse;
    let expected_line = if movement.direction > 0 {
        movement.start_line.checked_add(1)
    } else {
        movement.start_line.checked_sub(1)
    };
    if expected_line == Some(target_line) {
        stats.next_line_cross_count = stats.next_line_cross_count.saturating_add(1);
    } else if (movement.direction > 0 && target_line < movement.start_line)
        || (movement.direction < 0 && target_line > movement.start_line)
    {
        stats.support_resistance_count = stats.support_resistance_count.saturating_add(1);
    } else {
        stats.price_only_count = stats.price_only_count.saturating_add(1);
    }
    let Some((_, outcome_pct)) = mrc_cross_analysis::movement_outcome(favorable, adverse) else {
        return;
    };
    stats.outcome_pct_sum += outcome_pct;
    if favorable > adverse.abs() {
        stats.success_count = stats.success_count.saturating_add(1);
    } else {
        stats.failure_count = stats.failure_count.saturating_add(1);
    }
}

fn transition_stats(
    state: &MrcCrossingModel,
    source: u8,
    direction: i8,
    context: Option<(bool, Option<u8>)>,
) -> ([MrcForecastStats; 9], u32) {
    let mut routes = [MrcForecastStats::default(); 9];
    for pair in prediction_history(state).windows(2) {
        let [from, next] = pair else {
            continue;
        };
        if from.line != source || from.direction != direction {
            continue;
        }
        if let Some((no_pullback, reverted_line)) = context {
            if (from.no_pullback_count == 0) != no_pullback || from.reverted_line != reverted_line {
                continue;
            }
        }
        if next.line as usize >= routes.len() {
            continue;
        }
        let duration_ms = next.now_ms.saturating_sub(from.now_ms);
        record_movement(
            &mut routes[next.line as usize],
            next.line,
            next.completed_movement,
            duration_ms,
        );
    }
    let total = routes.iter().map(|stats| stats.count).sum();
    (routes, total)
}

fn route_counts(routes: &[MrcForecastStats; 9]) -> (u32, u32, u32) {
    routes
        .iter()
        .fold((0, 0, 0), |(next, support, price_only), stats| {
            (
                next.saturating_add(stats.next_line_cross_count),
                support.saturating_add(stats.support_resistance_count),
                price_only.saturating_add(stats.price_only_count),
            )
        })
}

fn line_direction(from: u8, to: u8) -> i8 {
    match to.cmp(&from) {
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct FastBreakEvidence {
    rejection: MrcForecastStats,
    continuation: MrcForecastStats,
    neutral: MrcForecastStats,
}

impl FastBreakEvidence {
    fn total(self) -> u32 {
        self.rejection
            .count
            .saturating_add(self.continuation.count)
            .saturating_add(self.neutral.count)
    }
}

/// Learn what happened after the exact failed break, rather than learning
/// generic transitions from the returned line.
///
/// For `A -> B -> A`, the next observed line is classified as:
///
/// - rejection: movement away from B;
/// - continuation: movement back toward and through B;
/// - neutral: no directional decision.
fn fast_break_evidence(
    state: &MrcCrossingModel,
    source_line: u8,
    failed_break_line: u8,
) -> FastBreakEvidence {
    let break_direction = line_direction(source_line, failed_break_line);
    if break_direction == 0 || source_line.abs_diff(failed_break_line) != 1 {
        return FastBreakEvidence::default();
    }

    let mut evidence = FastBreakEvidence::default();
    for window in prediction_history(state).windows(4) {
        let [source, crossed, returned, next] = window else {
            continue;
        };
        if source.line != source_line
            || crossed.line != failed_break_line
            || returned.line != source_line
            || line_direction(source.line, crossed.line) != break_direction
        {
            continue;
        }

        let next_direction = line_direction(returned.line, next.line);
        let duration_ms = next.now_ms.saturating_sub(returned.now_ms);
        let stats = if next_direction == -break_direction {
            &mut evidence.rejection
        } else if next_direction == break_direction {
            &mut evidence.continuation
        } else {
            &mut evidence.neutral
        };
        record_movement(stats, next.line, next.completed_movement, duration_ms);
    }
    evidence
}

fn available_samples(state: &MrcCrossingModel, direction: i8) -> u32 {
    let Some(current) = state.history.last() else {
        return 0;
    };
    let context = Some((current.no_pullback_count == 0, current.reverted_line));
    let contextual = transition_stats(state, current.line, direction, context).1;
    if contextual > 0 {
        contextual
    } else {
        transition_stats(state, current.line, direction, None).1
    }
}

fn current_cycle_pattern(state: &MrcCrossingModel) -> Vec<u8> {
    state.history.iter().map(|event| event.line).collect()
}

fn current_cycle_elapsed_ms(state: &MrcCrossingModel) -> Vec<u64> {
    let Some(first) = state.history.first() else {
        return Vec::new();
    };
    let mut elapsed = Vec::with_capacity(state.history.len());
    elapsed.push(0);
    let mut previous = first.now_ms;
    for event in state.history.iter().skip(1) {
        elapsed.push(event.now_ms.saturating_sub(previous));
        previous = event.now_ms;
    }
    elapsed
}

fn pattern_prior(prediction: &MrcPrediction) -> f64 {
    prediction
        .stats
        .expected_value_pct()
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
}

fn pattern_score(performance: PatternPerformance, prediction: &MrcPrediction) -> f64 {
    // A pattern's global ledger is only relevant when the current target has
    // some supporting route evidence. Keep this continuous: one observation
    // contributes, but it cannot carry the same weight as a repeatedly
    // observed target. This is especially important for FAST_BREAK_RETURN,
    // where a single exact rejection used to outrank better-supported routes.
    let target_confidence = prediction.stats.count as f64 / (prediction.stats.count as f64 + 4.0);
    let route_share = if prediction.samples > 0 {
        prediction.stats.count as f64 / prediction.samples as f64
    } else {
        0.0
    };
    let evidence_confidence = target_confidence * route_share;
    let prior = pattern_prior(prediction) * evidence_confidence;
    let base = if performance.trades == 0 {
        prior
    } else {
        let confidence = performance.trades as f64 / (performance.trades as f64 + 4.0);
        // The recent EMA is useful for adaptation, but using it alone lets a
        // pattern with a losing history become the winner after a short
        // streak. Keep the long-run realized average in the score so
        // selection reflects both the current regime and the full record.
        let realized = (performance.average_pnl_pct() + performance.ema_pnl_pct) / 2.0;
        (realized * confidence + prior * (1.0 - confidence)) * evidence_confidence
    };
    base
}

fn direction_prediction(
    touch: Option<MrcTouchSignal>,
    touch_learning: MrcTouchLearningSignal,
    analysis: &MrcCrossAnalysis,
) -> MrcDirectionPrediction {
    let Some(touch) = touch else {
        return MrcDirectionPrediction::default();
    };
    let entry = analysis.effective_entry_price.unwrap_or(0.0);
    let target_distance_pct = analysis
        .target_price
        .filter(|price| price.is_finite() && *price > 0.0 && entry > 0.0)
        .map(|price| (price - entry).abs() / entry * 100.0)
        .unwrap_or(0.0);
    let stop_distance_pct = analysis
        .stop_price
        .filter(|price| price.is_finite() && *price > 0.0 && entry > 0.0)
        .map(|price| (price - entry).abs() / entry * 100.0)
        .unwrap_or(0.0);
    let mean_distance_pct = touch
        .line(4)
        .map(|line| line.distance_ema_pct)
        .unwrap_or(0.0);
    mrc_direction_prediction::predict(
        touch,
        MrcDirectionContext {
            structural_direction: analysis.trade_direction,
            candidate_direction: analysis.trade_direction,
            mean_distance_pct,
            historical_expected_pct: analysis.expected_value_pct,
            historical_success_rate: analysis.movement_success_rate,
            pattern_samples: analysis.pattern_samples,
            target_samples: analysis.target_samples,
            target_distance_pct,
            stop_distance_pct,
            touch_learning,
        },
    )
}

fn prediction_allows_entry(prediction: MrcDirectionPrediction) -> bool {
    !prediction.has_evidence() || prediction.candidate_allowed
}

fn candidate_key(candidate: &PatternCandidate) -> Option<(u8, u8, u8)> {
    Some((
        candidate.pattern_kind.index() as u8,
        candidate.analysis.line_idx,
        candidate.analysis.trade_target_line?,
    ))
}

fn select_candidate(
    state: &mut MrcCrossingModel,
    candidates: &[PatternCandidate],
    now_ms: u64,
) -> Option<usize> {
    let views = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| CandidateView {
            index,
            pattern_kind: candidate.pattern_kind,
            score: candidate.score,
            entry: candidate.entry.is_some(),
            direction: candidate.analysis.trade_direction,
            volume_gain: candidate.analysis.volume_gain,
            realized_favorable_pct: candidate.analysis.realized_favorable_move_pct,
            realized_adverse_pct: candidate.analysis.realized_adverse_move_pct,
            market_price: candidate.analysis.market_price,
            staircase_lifecycle: candidate.analysis.staircase_lifecycle,
            staircase_transition: candidate.analysis.staircase_transition,
        })
        .collect::<Vec<_>>();
    let result = state.decision_state.choose_with_direction_lock(
        state.decision_components,
        &views,
        state.trade_direction_lock,
        now_ms,
    );
    let Some(selected_index) = result.selected_index else {
        if result.primary_kind.is_none() {
            state.active_pattern = None;
            state.active_pattern_score = 0.0;
        }
        return None;
    };
    let selected = &candidates[selected_index];
    let pattern_index = selected.pattern_kind.index() as u8;
    if state.active_pattern != Some(pattern_index) {
        state.active_pattern_since_ms = now_ms;
    }
    state.active_pattern = Some(pattern_index);
    state.active_pattern_score = selected.score;
    Some(selected_index)
}

pub fn predict_all(state: &MrcCrossingModel) -> Vec<MrcPrediction> {
    if state.history.len() < 2 {
        return Vec::new();
    }
    mrc_pattern_classification::classify_all(&state.history)
        .into_iter()
        .filter_map(|pattern| predict_for_pattern(state, &pattern))
        .collect()
}

fn predict_for_pattern(
    state: &MrcCrossingModel,
    pattern: &mrc_pattern_classification::MrcPatternSignal,
) -> Option<MrcPrediction> {
    let current = state.history.last()?;
    let source = current.line;
    let (
        pattern_direction,
        target,
        total,
        stats,
        line_stats,
        next_line_crosses,
        support_resistance_moves,
        price_only_moves,
        context_samples,
        failed_break_line,
        failed_break_direction,
    ) = if pattern.kind == MrcPatternKind::FastBreakReturn {
        let failed_break_line = pattern.failed_break_line?;
        let failed_break_direction = pattern.failed_break_direction;
        if !matches!(failed_break_direction, 1 | -1) {
            return None;
        }
        let evidence = fast_break_evidence(state, source, failed_break_line);
        let total = evidence.total();
        if total == 0 {
            return None;
        }

        // Use only the exact outcome counts. A tie or a mostly-neutral result
        // is not a directional support/resistance prediction.
        let (pattern_direction, target, stats) = if evidence.rejection.count
            > evidence
                .continuation
                .count
                .saturating_add(evidence.neutral.count)
        {
            (
                -failed_break_direction,
                mrc_cross_analysis::adjacent_line(source, -failed_break_direction)?,
                evidence.rejection,
            )
        } else if evidence.continuation.count
            > evidence
                .rejection
                .count
                .saturating_add(evidence.neutral.count)
        {
            (
                failed_break_direction,
                failed_break_line,
                evidence.continuation,
            )
        } else {
            return None;
        };
        let mut line_stats = [MrcForecastStats::default(); mrc_prediction::MRC_LINE_COUNT];
        line_stats[target as usize] = stats;
        (
            pattern_direction,
            target,
            total,
            stats,
            line_stats,
            evidence.continuation.count,
            evidence.rejection.count,
            evidence.neutral.count,
            0,
            Some(failed_break_line),
            failed_break_direction,
        )
    } else {
        if !matches!(pattern.direction, 1 | -1) {
            return None;
        }
        let context = Some((current.no_pullback_count == 0, current.reverted_line));
        let contextual = transition_stats(state, source, pattern.direction, context);
        let (routes, total, context_samples) = if contextual.1 > 0 {
            (contextual.0, contextual.1, contextual.1)
        } else {
            let broad = transition_stats(state, source, pattern.direction, None);
            (broad.0, broad.1, 0)
        };
        let (next_line_crosses, support_resistance_moves, price_only_moves) = route_counts(&routes);
        let target = pattern.target_line;
        if target == source || target as usize >= routes.len() {
            return None;
        }
        let stats = routes[target as usize];
        (
            pattern.direction,
            target,
            total,
            stats,
            routes,
            next_line_crosses,
            support_resistance_moves,
            price_only_moves,
            context_samples,
            None,
            0,
        )
    };
    let cycle = current_cycle_pattern(state);
    let cycle_elapsed_ms = current_cycle_elapsed_ms(state);
    if target == source || target as usize >= LINE_LABELS.len() {
        return None;
    }
    Some(MrcPrediction {
        pattern_kind: pattern.kind,
        pattern_direction,
        pattern_steps: pattern.steps,
        pattern_elapsed_ms: pattern.elapsed_ms,
        stair_sequence: pattern.sequence.clone(),
        stair_price_move_pct: pattern.price_move_pct,
        stair_favorable_move_pct: pattern.favorable_move_pct,
        stair_adverse_move_pct: pattern.adverse_move_pct,
        staircase_lifecycle: pattern.staircase_lifecycle,
        staircase_transition: pattern.staircase_transition,
        staircase_protected_line: pattern.staircase_protected_line,
        staircase_extreme_line: pattern.staircase_extreme_line,
        pattern: cycle,
        elapsed_ms: cycle_elapsed_ms,
        target_line: target,
        samples: total,
        stats,
        line_stats,
        next_line_crosses,
        support_resistance_moves,
        price_only_moves,
        context_samples,
        failed_break_line,
        failed_break_direction,
    })
}

pub fn price_projection(
    direction: i8,
    mrc_min: f64,
    mrc_max: f64,
    expected_price_low: f64,
    expected_price_high: f64,
) -> &'static str {
    let range = (mrc_max - mrc_min).abs();
    let tolerance = (range * 0.02).max(1e-12);
    if direction > 0 {
        if expected_price_high > mrc_max + tolerance {
            "above_mrc_max"
        } else if expected_price_high + tolerance < mrc_max {
            "below_mrc_max"
        } else {
            "same_mrc_max"
        }
    } else if expected_price_low < mrc_min - tolerance {
        "below_mrc_min"
    } else if expected_price_low > mrc_min + tolerance {
        "above_mrc_min"
    } else {
        "same_mrc_min"
    }
}

fn compact_plan_move(analysis: &MrcCrossAnalysis, line_idx: Option<u8>) -> String {
    line_idx
        .and_then(|line_idx| {
            analysis
                .line_plans
                .iter()
                .find(|plan| plan.line_idx == line_idx)
        })
        .map(|plan| format!("{:+.3}%", plan.move_pct))
        .unwrap_or_else(|| "na".to_string())
}

/// Compact live/debug representation of the selected MRC route.
///
/// The complete per-line array remains available to the research JSON/HTML
/// reports. Live debug only needs the route that can affect execution:
/// current line, actual target, and stop.
pub fn line_plan_summary(analysis: &MrcCrossAnalysis) -> String {
    let target_line = analysis.take_profit_line.or(analysis.trade_target_line);
    let line_label = |line_idx: Option<u8>| match line_idx {
        Some(0) => "db",
        Some(1) => "do",
        Some(2) => "ds",
        Some(3) => "di",
        Some(4) => "m",
        Some(5) => "ui",
        Some(6) => "us",
        Some(7) => "uo",
        Some(8) => "ub",
        _ => "na",
    };
    format!(
        "line_plans[e:{} tp:{}{} sl={}{}]",
        line_label(Some(analysis.line_idx)),
        line_label(target_line),
        compact_plan_move(analysis, target_line),
        line_label(analysis.stop_loss_line),
        compact_plan_move(analysis, analysis.stop_loss_line),
    )
}

fn append_history_event(state: &mut MrcCrossingModel, event: &MrcCrossEvent) {
    // Older persisted models and test fixtures only have the short local
    // history. Promote that history before adding the first new observation
    // so the long-lived predictor does not silently discard its evidence.
    if state.prediction_history.is_empty() && !state.history.is_empty() {
        state
            .prediction_history
            .extend(state.history.iter().copied());
    }
    let history_event = MrcHistoryEvent {
        line: event.line_idx,
        direction: event.direction,
        now_ms: event.now_ms,
        market_price: event.price,
        mrc_low: event.mrc_low,
        mrc_high: event.mrc_high,
        no_pullback_count: event.no_pullback_count,
        reverted_line: event.reverted_line,
        completed_movement: event.completed_movement,
    };
    state.prediction_history.push(history_event);
    if state.prediction_history.len() > MAX_PREDICTION_HISTORY {
        let trim_count = state
            .prediction_history
            .len()
            .saturating_sub(MAX_PREDICTION_HISTORY);
        state.prediction_history.drain(..trim_count);
    }
    state.history.push(history_event);
    trim_history_to_recent_cycles(&mut state.history);
    state.last_line = Some(event.line_idx);
    state.last_direction = event.direction;
    state.last_event_ms = event.now_ms;
}

fn prediction_history(state: &MrcCrossingModel) -> &[MrcHistoryEvent] {
    if state.prediction_history.is_empty() {
        // Keep hand-built model fixtures and old persisted states compatible;
        // live observations populate the long-lived history explicitly.
        &state.history
    } else {
        &state.prediction_history
    }
}

fn trim_history_to_recent_cycles(history: &mut Vec<MrcHistoryEvent>) {
    let mut direction_changes = 0usize;
    let mut trim_before = None;
    for index in (1..history.len()).rev() {
        if history[index].direction != history[index - 1].direction {
            direction_changes += 1;
            if direction_changes >= MAX_LEARNED_CYCLES {
                trim_before = Some(index);
                break;
            }
        }
    }
    if let Some(index) = trim_before {
        history.drain(..index);
    }
}

pub fn observe(
    state: &mut MrcCrossingModel,
    events: &[MrcCrossEvent],
    lines: [f64; 9],
) -> Observation {
    observe_with_context(
        state,
        events,
        lines,
        &MrcCrossingState::default(),
        (0.0, 0.0, 0.0, 0.0),
    )
}

pub fn observe_with_context(
    state: &mut MrcCrossingModel,
    events: &[MrcCrossEvent],
    lines: [f64; 9],
    crossing: &MrcCrossingState,
    volume: (f64, f64, f64, f64),
) -> Observation {
    observe_events(
        state,
        events,
        lines,
        crossing,
        volume,
        None,
        mrc_prediction::MrcPredictionContext::default(),
        None,
        None,
    )
}

/// Observe new crossings and, when there is no fresh crossing, evaluate the
/// learned route against the current candle. The forecast lets the strategy
/// enter while the next line is still ahead instead of waiting for a fill at
/// an already stale crossing price.
pub fn observe_with_forecast(
    state: &mut MrcCrossingModel,
    events: &[MrcCrossEvent],
    lines: [f64; 9],
    crossing: &MrcCrossingState,
    volume: (f64, f64, f64, f64),
    current_price: f64,
    now_ms: u64,
) -> Observation {
    observe_with_forecast_context(
        state,
        events,
        lines,
        crossing,
        volume,
        current_price,
        now_ms,
        mrc_prediction::MrcPredictionContext::default(),
    )
}

pub fn observe_with_forecast_context(
    state: &mut MrcCrossingModel,
    events: &[MrcCrossEvent],
    lines: [f64; 9],
    crossing: &MrcCrossingState,
    volume: (f64, f64, f64, f64),
    current_price: f64,
    now_ms: u64,
    context: mrc_prediction::MrcPredictionContext,
) -> Observation {
    observe_events(
        state,
        events,
        lines,
        crossing,
        volume,
        Some((current_price, now_ms)),
        context,
        None,
        None,
    )
}

/// Forecast path with the incremental MRC touch signal available to
/// candidate scoring. The pure observation APIs remain unchanged for
/// research and unit-test callers that do not have realtime touch state.
pub fn observe_with_forecast_and_touch_context(
    state: &mut MrcCrossingModel,
    events: &[MrcCrossEvent],
    lines: [f64; 9],
    crossing: &MrcCrossingState,
    volume: (f64, f64, f64, f64),
    current_price: f64,
    now_ms: u64,
    context: mrc_prediction::MrcPredictionContext,
    touch: MrcTouchSignal,
) -> Observation {
    observe_with_forecast_and_touch_and_learning_context(
        state,
        events,
        lines,
        crossing,
        volume,
        current_price,
        now_ms,
        context,
        touch,
        MrcTouchLearningSignal::default(),
    )
}

/// Forecast path with both incremental touch state and empirical touch
/// outcomes. The old touch-only API remains available for callers that do not
/// maintain the learner.
pub fn observe_with_forecast_and_touch_and_learning_context(
    state: &mut MrcCrossingModel,
    events: &[MrcCrossEvent],
    lines: [f64; 9],
    crossing: &MrcCrossingState,
    volume: (f64, f64, f64, f64),
    current_price: f64,
    now_ms: u64,
    context: mrc_prediction::MrcPredictionContext,
    touch: MrcTouchSignal,
    touch_learning: MrcTouchLearningSignal,
) -> Observation {
    observe_events(
        state,
        events,
        lines,
        crossing,
        volume,
        Some((current_price, now_ms)),
        context,
        Some(touch),
        Some(touch_learning),
    )
}

fn observe_events(
    state: &mut MrcCrossingModel,
    events: &[MrcCrossEvent],
    lines: [f64; 9],
    crossing: &MrcCrossingState,
    volume: (f64, f64, f64, f64),
    forecast: Option<(f64, u64)>,
    context: mrc_prediction::MrcPredictionContext,
    touch: Option<MrcTouchSignal>,
    touch_learning: Option<MrcTouchLearningSignal>,
) -> Observation {
    state.current_entry = None;
    let mut observation = Observation::default();
    let mut saw_prediction = false;
    let mut saw_crossing_barrier = false;
    let mut saw_untradeable_prediction = false;
    let mut saw_unreliable_prediction = false;
    let mut saw_missing_touch = false;
    if touch_learning.is_some_and(|learning| !learning.exact_touch) {
        saw_missing_touch = true;
    }

    for event in events {
        let source = event.line_idx as usize;
        if source >= LINE_LABELS.len() {
            continue;
        }
        append_history_event(state, event);

        let predictions = predict_all(state);
        if predictions.is_empty() {
            saw_unreliable_prediction = true;
            continue;
        }
        for prediction in predictions {
            saw_prediction = true;
            let performance = &mut state.pattern_performance[prediction.pattern_kind.index()];
            performance.observe();
            observation.samples = observation.samples.max(prediction.samples);
            let analysis = mrc_cross_analysis::build_with_context(
                event,
                Some(&prediction),
                lines,
                crossing,
                volume,
                context,
            );
            let direction_prediction =
                direction_prediction(touch, touch_learning.unwrap_or_default(), &analysis);
            let touch_ready = touch_learning
                .map(|learning| learning.current_touch_matches(analysis.line_idx))
                .unwrap_or(true);
            let entry = (analysis.tradeable()
                && touch_ready
                && prediction_allows_entry(direction_prediction))
            .then(|| {
                analysis
                    .effective_entry_price
                    .zip(analysis.target_price)
                    .zip(analysis.stop_price)
                    .map(|((entry_price, target_price), stop_price)| {
                        entry_reason(
                            event,
                            &prediction,
                            &analysis,
                            entry_price,
                            target_price,
                            stop_price,
                        )
                    })
            })
            .flatten();
            if entry.is_none() {
                saw_untradeable_prediction = true;
                saw_missing_touch |= !touch_ready;
                if analysis.barrier_state == mrc_cross_analysis::BarrierState::Approaching {
                    saw_crossing_barrier = true;
                }
            }
            observation.events.push(analysis.clone());
            observation.candidates.push(PatternCandidate {
                pattern_kind: prediction.pattern_kind,
                score: pattern_score(*performance, &prediction),
                analysis,
                direction_prediction,
                entry,
                forecast: false,
            });
        }
    }

    if events.is_empty() {
        if let Some((current_price, now_ms)) = forecast {
            if let Some(event) = forecast_event(state, lines, current_price, now_ms) {
                let predictions = predict_all(state);
                for prediction in predictions {
                    let performance =
                        &mut state.pattern_performance[prediction.pattern_kind.index()];
                    performance.observe();
                    observation.samples = observation.samples.max(prediction.samples);
                    let analysis = mrc_cross_analysis::build_with_context(
                        &event,
                        Some(&prediction),
                        lines,
                        crossing,
                        volume,
                        context,
                    );
                    let direction_prediction =
                        direction_prediction(touch, touch_learning.unwrap_or_default(), &analysis);
                    let touch_ready = touch_learning
                        .map(|learning| learning.current_touch_matches(analysis.line_idx))
                        .unwrap_or(true);
                    let forecast_allowed = (forecast_can_reach(&analysis)
                        || matches!(
                            prediction.pattern_kind,
                            MrcPatternKind::StairsUp | MrcPatternKind::StairsDown
                        ))
                        && analysis.forecast_tradeable()
                        && touch_ready
                        && prediction_allows_entry(direction_prediction);
                    saw_missing_touch |= !touch_ready;
                    let entry = if forecast_allowed {
                        analysis
                            .effective_entry_price
                            .zip(analysis.target_price)
                            .zip(analysis.stop_price)
                            .map(|((entry_price, target_price), stop_price)| {
                                entry_reason(
                                    &event,
                                    &prediction,
                                    &analysis,
                                    entry_price,
                                    target_price,
                                    stop_price,
                                )
                            })
                    } else {
                        None
                    };
                    observation.candidates.push(PatternCandidate {
                        pattern_kind: prediction.pattern_kind,
                        score: pattern_score(*performance, &prediction),
                        analysis,
                        direction_prediction,
                        entry,
                        forecast: true,
                    });
                }
            }
        }
    }

    if let Some(selected_index) = select_candidate(
        state,
        &observation.candidates,
        forecast.map(|(_, now_ms)| now_ms).unwrap_or_else(|| {
            observation
                .events
                .last()
                .map(|event| event.now_ms)
                .unwrap_or_default()
        }),
    ) {
        observation.selected_candidate_index = Some(selected_index);
        let selected = &observation.candidates[selected_index];
        observation.selected_pattern = Some(selected.pattern_kind);
        observation.selected_score = Some(selected.score);
        if selected.forecast {
            observation.forecast = Some(selected.analysis.clone());
        }
        if let Some(entry) = selected.entry.clone() {
            if let Some(key) = candidate_key(selected) {
                if state.last_plan_key != Some(key) {
                    state.last_plan_key = Some(key);
                    state.current_entry = Some(entry.clone());
                    observation.entry = Some(entry);
                }
            }
        }
    }
    if observation.entry.is_none() {
        observation.wait_reason = if saw_missing_touch {
            Some(MrcWaitReason::Touch)
        } else if saw_crossing_barrier {
            Some(MrcWaitReason::Crossing)
        } else if saw_prediction || saw_untradeable_prediction {
            Some(MrcWaitReason::NoDirectionalTarget)
        } else if saw_unreliable_prediction {
            Some(MrcWaitReason::NoReliablePrediction)
        } else {
            Some(MrcWaitReason::NoReliablePrediction)
        };
    }
    observation
}

fn entry_reason(
    event: &MrcCrossEvent,
    prediction: &MrcPrediction,
    analysis: &MrcCrossAnalysis,
    entry_price: f64,
    target_price: f64,
    stop_price: f64,
) -> EntryReason {
    EntryReason::MrcCrossing {
        price: entry_price,
        market_price: event.price,
        mrc_min: event.mrc_low,
        mrc_max: event.mrc_high,
        target_price,
        stop_price,
        source_line: event.line_idx,
        target_line: prediction.target_line,
        pattern_kind: prediction.pattern_kind.as_str().to_string(),
        pattern: prediction.pattern.clone(),
        pattern_elapsed_ms: prediction.elapsed_ms.clone(),
        pattern_samples: prediction.samples,
        context_samples: prediction.context_samples,
        no_pullback_count: event.no_pullback_count,
        reverted_line: event.reverted_line,
        target_samples: prediction.stats.count,
        movement_samples: prediction.stats.movement_samples,
        next_line_crosses: prediction.next_line_crosses,
        support_resistance_moves: prediction.support_resistance_moves,
        price_only_moves: prediction.price_only_moves,
        successful_moves: prediction.stats.success_count,
        failed_moves: prediction.stats.failure_count,
        movement_success_rate: prediction.stats.movement_success_rate().unwrap_or(0.0),
        expected_value_pct: prediction.stats.expected_value_pct().unwrap_or(0.0),
        expected_price_low: analysis.expected_price_low.unwrap_or(event.price),
        expected_price_high: analysis.expected_price_high.unwrap_or(event.price),
        estimated_time_ms: prediction.stats.average_duration_ms().unwrap_or(0.0),
        favorable_move_pct: prediction.stats.average_favorable_pct().unwrap_or(0.0),
        adverse_move_pct: prediction.stats.average_adverse_pct().unwrap_or(0.0),
    }
}

fn forecast_event(
    state: &MrcCrossingModel,
    lines: [f64; 9],
    current_price: f64,
    now_ms: u64,
) -> Option<MrcCrossEvent> {
    if !current_price.is_finite() || current_price <= 0.0 {
        return None;
    }
    let last = state.history.last()?;
    let source = last.line as usize;
    let line = lines.get(source).copied()?;
    if !line.is_finite() || line <= 0.0 || !matches!(last.direction, 1 | -1) {
        return None;
    }
    let event = MrcCrossEvent {
        line_idx: last.line,
        line,
        price: current_price,
        mrc_low: lines[0],
        mrc_high: lines[8],
        direction: last.direction,
        tick: now_ms,
        now_ms,
        no_pullback_count: last.no_pullback_count,
        reverted_line: last.reverted_line,
        completed_movement: None,
    };
    Some(event)
}

fn forecast_can_reach(analysis: &MrcCrossAnalysis) -> bool {
    let Some(move_pct) = analysis
        .effective_move_pct
        .filter(|value| value.is_finite() && *value > 0.0)
    else {
        return false;
    };
    let Some(entry_price) = analysis
        .effective_entry_price
        .filter(|value| value.is_finite() && *value > 0.0)
    else {
        return false;
    };
    let reaches = |price: Option<f64>| {
        let Some(price) = price.filter(|value| value.is_finite() && *value > 0.0) else {
            return false;
        };
        let distance_pct = (price - entry_price).abs() / entry_price * 100.0;
        distance_pct <= move_pct + f64::EPSILON
    };
    if !reaches(analysis.predicted_line_price) {
        return false;
    }
    match analysis.barrier_state {
        mrc_cross_analysis::BarrierState::Approaching => reaches(analysis.barrier_price),
        mrc_cross_analysis::BarrierState::None
        | mrc_cross_analysis::BarrierState::ConfirmedBreak
        | mrc_cross_analysis::BarrierState::Rejected => true,
    }
}

/// Recalculate MRC exits from the current levels and actual fill/reference
/// price. The line identities in the reason remain the learned plan, while
/// the prices follow moving MRC bands.
pub fn exit_levels(reason: &EntryReason, lines: [f64; 9], entry_price: f64) -> Option<(f64, f64)> {
    let EntryReason::MrcCrossing {
        source_line,
        target_line,
        pattern_kind,
        ..
    } = reason;
    let direction = reason_direction(reason);
    let mut target_line = *target_line;
    let target_price = loop {
        let price = lines.get(target_line as usize).copied()?;
        if price.is_finite()
            && price > 0.0
            && ((direction > 0 && price > entry_price) || (direction < 0 && price < entry_price))
        {
            break price;
        }
        target_line = mrc_cross_analysis::adjacent_line(target_line, direction)?;
    };
    let stop_line = mrc_cross_analysis::mrc_stop_line(lines, *source_line, entry_price, direction)?;
    let stop_price = lines.get(stop_line as usize).copied()?;
    valid_price_levels_for_pattern(
        direction,
        entry_price,
        target_price,
        stop_price,
        None,
        MrcPatternKind::from_str(pattern_kind),
    )
    .then_some((target_price, stop_price))
}

pub fn current_entry(state: &MrcCrossingModel) -> Option<EntryReason> {
    state.current_entry.clone()
}

pub fn record_trade_result(
    state: &mut MrcCrossingModel,
    reason: Option<&EntryReason>,
    pnl_pct: f64,
) {
    let Some(EntryReason::MrcCrossing { pattern_kind, .. }) = reason else {
        return;
    };
    let Some(pattern_kind) = MrcPatternKind::from_str(pattern_kind) else {
        return;
    };
    record_pattern_result(state, pattern_kind, pnl_pct);
}

pub fn record_pattern_result(
    state: &mut MrcCrossingModel,
    pattern_kind: MrcPatternKind,
    pnl_pct: f64,
) {
    state.pattern_performance[pattern_kind.index()].record_trade(pnl_pct);
}

pub fn market_state(direction: i8) -> MarketState {
    if direction > 0 {
        MarketState::Long
    } else if direction < 0 {
        MarketState::Short
    } else {
        MarketState::Sideways
    }
}

pub fn reason_direction(reason: &EntryReason) -> i8 {
    let EntryReason::MrcCrossing {
        price,
        target_price,
        source_line,
        ..
    } = reason;
    match target_price.partial_cmp(price) {
        Some(std::cmp::Ordering::Greater) => 1,
        Some(std::cmp::Ordering::Less) => -1,
        _ => match source_line {
            0..=3 => -1,
            5..=8 => 1,
            _ => 0,
        },
    }
}

fn history_float(
    history: &IndicatorHistory,
    indicator: IndicatorKey,
    field: IndicatorField,
    offset: usize,
) -> Option<f64> {
    match history.get_prev(
        IndicatorFieldKey {
            timeframe: TimeframeSec::Tf1m,
            indicator,
            field,
        },
        offset,
    ) {
        Some(ModuleIndicatorValue::Float(value)) => Some(*value),
        Some(ModuleIndicatorValue::String(value)) => value.parse().ok(),
        _ => None,
    }
}

pub fn seed_from_history(
    state: &mut MrcCrossingModel,
    vlines: &mut [crate::state::VLineState; 9],
    crossing: &mut MrcCrossingState,
    history: &IndicatorHistory,
    now_ms: u64,
) {
    seed_from_history_impl(state, vlines, crossing, history, now_ms, None);
}

/// Seed the discrete crossing model and the incremental touch analyzer from
/// the same historical MRC/candle samples. Keeping this replay here avoids a
/// second history reader in the touch analyzer and makes its EMA immediately
/// useful after startup.
pub fn seed_from_history_with_touch(
    state: &mut MrcCrossingModel,
    vlines: &mut [crate::state::VLineState; 9],
    crossing: &mut MrcCrossingState,
    history: &IndicatorHistory,
    now_ms: u64,
    touch: &mut MrcTouchState,
) {
    seed_from_history_impl(state, vlines, crossing, history, now_ms, Some(touch));
}

fn seed_from_history_impl(
    state: &mut MrcCrossingModel,
    vlines: &mut [crate::state::VLineState; 9],
    crossing: &mut MrcCrossingState,
    history: &IndicatorHistory,
    now_ms: u64,
    mut touch: Option<&mut MrcTouchState>,
) {
    const HISTORY_LIMIT: usize = 10_080;
    if now_ms == 0 {
        return;
    }
    let seed_start = now_ms.saturating_sub((HISTORY_LIMIT as u64 - 1) * 60_000);
    for offset in (0..HISTORY_LIMIT).rev() {
        let fields = [
            IndicatorField::DownBig,
            IndicatorField::DownOuter,
            IndicatorField::DownSmall,
            IndicatorField::DownInner,
            IndicatorField::MeanLine,
            IndicatorField::UpInner,
            IndicatorField::UpSmall,
            IndicatorField::UpOuter,
            IndicatorField::UpBig,
        ];
        let Some(lines) = fields
            .map(|field| history_float(history, IndicatorKey::Mrc200, field, offset))
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .and_then(|values| values.try_into().ok())
        else {
            continue;
        };
        let Some(price) =
            history_float(history, IndicatorKey::Candle, IndicatorField::Close, offset)
        else {
            continue;
        };
        let now = seed_start.saturating_add((HISTORY_LIMIT - 1 - offset) as u64 * 60_000);
        let events = crate::detection::vlines::update_with_events(
            vlines,
            crossing,
            lines,
            price,
            (HISTORY_LIMIT - offset) as u64,
            now,
        );
        if let Some(touch) = touch.as_deref_mut() {
            touch.update(lines, price, now);
        }
        for event in events {
            append_history_event(state, &event);
        }
    }
    state.current_entry = None;
    state.seeded_from_history = true;
}
