use crate::detection::cascade::CascadeSignal;
use crate::detection::mrc_cross_analysis::{self, MrcCrossAnalysis};
use crate::detection::mrc_pattern_classification::{self, MrcPatternKind};
use crate::detection::vlines::MrcCrossEvent;
use crate::model::{EntryReason, MarketState};
use crate::state::{
    MrcCrossingModel, MrcCrossingState, MrcForecastStats, MrcHistoryEvent, MrcMovementSnapshot,
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

const MAX_LEARNED_CYCLES: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct MrcPrediction {
    pub pattern_kind: MrcPatternKind,
    pub pattern_direction: i8,
    pub pattern_steps: u32,
    pub pattern_elapsed_ms: u64,
    pub pattern: Vec<u8>,
    pub elapsed_ms: Vec<u64>,
    pub target_line: u8,
    pub samples: u32,
    pub probability: f64,
    pub stats: MrcForecastStats,
    pub next_line_crosses: u32,
    pub support_resistance_moves: u32,
    pub price_only_moves: u32,
    pub context_samples: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MrcWaitReason {
    Crossing,
    NoDirectionalTarget,
    NoReliablePrediction,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Observation {
    pub entry: Option<EntryReason>,
    pub wait_reason: Option<MrcWaitReason>,
    pub samples: u32,
    pub events: Vec<MrcCrossAnalysis>,
}

pub fn valid_price_levels(
    direction: i8,
    entry_price: f64,
    target_price: f64,
    stop_price: f64,
    tick_size: Option<f64>,
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
    }
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
    let successful_move = favorable > adverse.abs();
    stats.outcome_pct_sum += if successful_move { favorable } else { adverse };
    if successful_move {
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
    for pair in state.history.windows(2) {
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

pub fn predict(state: &MrcCrossingModel, _observed_direction: i8) -> Option<MrcPrediction> {
    if state.history.len() < 2 {
        return None;
    }
    let current = state.history.last()?;
    let source = current.line;
    let pattern = mrc_pattern_classification::classify(&state.history)?;
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
    let cycle = current_cycle_pattern(state);
    let cycle_elapsed_ms = current_cycle_elapsed_ms(state);
    let target = pattern.target_line;
    if target == source || target as usize >= routes.len() {
        return None;
    }
    let stats = routes[target as usize];
    if stats.count == 0 {
        return None;
    }
    Some(MrcPrediction {
        pattern_kind: pattern.kind,
        pattern_direction: pattern.direction,
        pattern_steps: pattern.steps,
        pattern_elapsed_ms: pattern.elapsed_ms,
        pattern: cycle,
        elapsed_ms: cycle_elapsed_ms,
        target_line: target,
        samples: total,
        probability: stats.count as f64 / total.max(1) as f64,
        stats,
        next_line_crosses,
        support_resistance_moves,
        price_only_moves,
        context_samples,
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

fn append_history_event(state: &mut MrcCrossingModel, event: &MrcCrossEvent) {
    state.history.push(MrcHistoryEvent {
        line: event.line_idx,
        direction: event.direction,
        now_ms: event.now_ms,
        market_price: event.price,
        mrc_low: event.mrc_low,
        mrc_high: event.mrc_high,
        no_pullback_count: event.no_pullback_count,
        reverted_line: event.reverted_line,
        completed_movement: event.completed_movement,
    });
    trim_history_to_recent_cycles(&mut state.history);
    state.last_line = Some(event.line_idx);
    state.last_direction = event.direction;
    state.last_event_ms = event.now_ms;
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
        CascadeSignal::default(),
        (0.0, 0.0, 0.0, 0.0),
    )
}

pub fn observe_with_context(
    state: &mut MrcCrossingModel,
    events: &[MrcCrossEvent],
    lines: [f64; 9],
    crossing: &MrcCrossingState,
    cascade: CascadeSignal,
    volume: (f64, f64, f64, f64),
) -> Observation {
    state.current_entry = None;
    let mut observation = Observation::default();

    for event in events {
        let source = event.line_idx as usize;
        if source >= LINE_LABELS.len() {
            continue;
        }
        append_history_event(state, event);

        let prediction = predict(state, event.direction);
        let analysis =
            mrc_cross_analysis::build(event, prediction.as_ref(), lines, crossing, cascade, volume);
        observation.samples = prediction
            .as_ref()
            .map(|prediction| prediction.samples)
            .unwrap_or_else(|| warmup_samples(state));
        observation.events.push(analysis.clone());

        let Some(prediction) = prediction else {
            observation.wait_reason = Some(MrcWaitReason::NoReliablePrediction);
            continue;
        };
        let can_trade = match analysis.barrier_state {
            mrc_cross_analysis::BarrierState::Approaching => false,
            mrc_cross_analysis::BarrierState::Rejected => true,
            mrc_cross_analysis::BarrierState::None
            | mrc_cross_analysis::BarrierState::ConfirmedBreak => true,
        };
        let Some(entry_price) = analysis.effective_entry_price else {
            observation.wait_reason = Some(MrcWaitReason::NoReliablePrediction);
            continue;
        };
        let Some(target_price) = analysis.target_price else {
            observation.wait_reason = Some(MrcWaitReason::NoDirectionalTarget);
            continue;
        };
        let Some(stop_price) = analysis.stop_price else {
            observation.wait_reason = Some(MrcWaitReason::NoDirectionalTarget);
            continue;
        };
        if !can_trade
            || !valid_price_levels(
                analysis.trade_direction,
                entry_price,
                target_price,
                stop_price,
                None,
            )
        {
            observation.wait_reason = Some(if can_trade {
                MrcWaitReason::NoDirectionalTarget
            } else {
                MrcWaitReason::Crossing
            });
            continue;
        }
        let Some(_target_line) = analysis.trade_target_line else {
            observation.wait_reason = Some(MrcWaitReason::NoDirectionalTarget);
            continue;
        };
        let reason = EntryReason::MrcCrossing {
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
            probability: prediction.probability,
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
        };
        state.current_entry = Some(reason.clone());
        observation.entry = Some(reason);
    }
    observation
}

pub fn current_entry(state: &MrcCrossingModel) -> Option<EntryReason> {
    state.current_entry.clone()
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
    } = reason
    else {
        return 0;
    };
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
        for event in events {
            append_history_event(state, &event);
        }
    }
    state.current_entry = None;
    state.seeded_from_history = true;
}
