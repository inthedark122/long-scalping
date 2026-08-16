use crate::detection::vlines::MrcCrossEvent;
use crate::model::{EntryReason, MarketState};
use crate::state::{
    MrcCrossingModel, MrcCrossingState, MrcForecastStats, MrcMovementSnapshot, MRC_PATTERN_LENGTH,
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrossingObservation {
    pub line_idx: u8,
    pub line: f64,
    pub direction: i8,
    pub now_ms: u64,
    pub target_line: Option<u8>,
    pub target_price: Option<f64>,
    pub stop_price: Option<f64>,
    pub probability: Option<f64>,
    pub estimated_time_ms: Option<f64>,
    pub favorable_move_pct: Option<f64>,
    pub adverse_move_pct: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MrcWaitReason {
    Crossing,
    Warmup,
    NoDirectionalTarget,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Observation {
    pub entry: Option<EntryReason>,
    pub wait_reason: Option<MrcWaitReason>,
    pub samples: u32,
    pub events: Vec<CrossingObservation>,
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
    pattern_stats(state)
        .map(|stats| stats.observations())
        .unwrap_or(0)
}

fn pattern_stats(state: &MrcCrossingModel) -> Option<&crate::state::MrcPatternStats> {
    (state.pattern_len == MRC_PATTERN_LENGTH).then(|| {
        &state.patterns[state.pattern_lines[0] as usize][state.pattern_lines[1] as usize]
            [state.pattern_lines[2] as usize]
    })
}

fn push_pattern(state: &mut MrcCrossingModel, line: u8) {
    if state.pattern_len < MRC_PATTERN_LENGTH {
        state.pattern_lines[state.pattern_len] = line;
        state.pattern_len += 1;
        return;
    }
    state.pattern_lines.copy_within(1.., 0);
    state.pattern_lines[MRC_PATTERN_LENGTH - 1] = line;
}

fn movement_percentages(movement: MrcMovementSnapshot) -> (f64, f64) {
    if movement.start_price <= 0.0 || movement.direction == 0 {
        return (0.0, 0.0);
    }
    if movement.direction > 0 {
        (
            (movement.high - movement.start_price) / movement.start_price * 100.0,
            (movement.low - movement.start_price) / movement.start_price * 100.0,
        )
    } else {
        (
            (movement.start_price - movement.low) / movement.start_price * 100.0,
            (movement.start_price - movement.high) / movement.start_price * 100.0,
        )
    }
}

fn record_next_crossing(
    state: &mut MrcCrossingModel,
    event: &MrcCrossEvent,
    fallback_duration_ms: u64,
) {
    if state.pattern_len == MRC_PATTERN_LENGTH {
        let pattern = state.pattern_lines;
        let destination = event.line_idx as usize;
        let movement = event.completed_movement;
        let stats = &mut state.patterns[pattern[0] as usize][pattern[1] as usize]
            [pattern[2] as usize]
            .next[destination];
        stats.count = stats.count.saturating_add(1);
        stats.duration_sum_ms = stats.duration_sum_ms.saturating_add(
            movement
                .map(|movement| movement.duration_ms)
                .unwrap_or(fallback_duration_ms),
        );
        if let Some(movement) = movement {
            let (favorable, adverse) = movement_percentages(movement);
            stats.favorable_pct_sum += favorable;
            stats.adverse_pct_sum += adverse;
        }
    }
    push_pattern(state, event.line_idx);
}

fn choose_target(
    stats: &crate::state::MrcPatternStats,
    source: usize,
    direction: i8,
) -> Option<(usize, MrcForecastStats, f64)> {
    let total = stats.observations();
    if total == 0 {
        return None;
    }
    let mut selected: Option<(usize, MrcForecastStats)> = None;
    for (line, candidate) in stats.next.iter().copied().enumerate() {
        let directional = if direction > 0 {
            line > source
        } else {
            line < source
        };
        if !directional || candidate.count == 0 {
            continue;
        }
        if selected
            .as_ref()
            .is_none_or(|(_, current)| candidate.count > current.count)
        {
            selected = Some((line, candidate));
        }
    }
    selected.map(|(line, stats)| (line, stats, stats.count as f64 / total as f64))
}

fn wait_event(event: &MrcCrossEvent) -> CrossingObservation {
    CrossingObservation {
        line_idx: event.line_idx,
        line: event.line,
        direction: event.direction,
        now_ms: event.now_ms,
        target_line: None,
        target_price: None,
        stop_price: None,
        probability: None,
        estimated_time_ms: None,
        favorable_move_pct: None,
        adverse_move_pct: None,
    }
}

pub fn observe(
    state: &mut MrcCrossingModel,
    events: &[MrcCrossEvent],
    lines: [f64; 9],
    required_samples: u32,
) -> Observation {
    state.pending_entry = None;
    let mut observation = Observation::default();

    for event in events {
        let source = event.line_idx as usize;
        if source >= LINE_LABELS.len() {
            continue;
        }
        let fallback_duration_ms = event.now_ms.saturating_sub(state.last_event_ms);
        record_next_crossing(state, event, fallback_duration_ms);
        state.last_line = Some(event.line_idx);
        state.last_direction = event.direction;
        state.last_event_ms = event.now_ms;

        let sample_count = warmup_samples(state);
        observation.samples = sample_count;
        let Some(stats) = pattern_stats(state) else {
            observation.wait_reason = Some(MrcWaitReason::Warmup);
            observation.events.push(wait_event(event));
            continue;
        };
        if sample_count < required_samples {
            observation.wait_reason = Some(MrcWaitReason::Warmup);
            observation.events.push(wait_event(event));
            continue;
        }
        let Some((target, target_stats, probability)) =
            choose_target(stats, source, event.direction)
        else {
            observation.wait_reason = Some(MrcWaitReason::NoDirectionalTarget);
            observation.events.push(wait_event(event));
            continue;
        };
        let stop = if event.direction > 0 {
            source.checked_sub(1)
        } else {
            (source + 1 < LINE_LABELS.len()).then_some(source + 1)
        };
        let Some(stop) = stop else {
            observation.wait_reason = Some(MrcWaitReason::NoDirectionalTarget);
            observation.events.push(wait_event(event));
            continue;
        };
        let target_price = lines[target];
        let stop_price = lines[stop];
        let direction = if event.direction > 0 { 1 } else { -1 };
        if !valid_price_levels(direction, event.line, target_price, stop_price, None) {
            observation.wait_reason = Some(MrcWaitReason::NoDirectionalTarget);
            observation.events.push(wait_event(event));
            continue;
        }
        let reason = EntryReason::MrcCrossing {
            price: event.line,
            target_price,
            stop_price,
            source_line: event.line_idx,
            target_line: target as u8,
            probability,
            estimated_time_ms: target_stats.average_duration_ms().unwrap_or(0.0),
            favorable_move_pct: target_stats.average_favorable_pct().unwrap_or(0.0),
            adverse_move_pct: target_stats.average_adverse_pct().unwrap_or(0.0),
        };
        state.pending_entry = Some(reason.clone());
        observation.entry = Some(reason);
        observation.events.push(CrossingObservation {
            line_idx: event.line_idx,
            line: event.line,
            direction: event.direction,
            now_ms: event.now_ms,
            target_line: Some(target as u8),
            target_price: Some(target_price),
            stop_price: Some(stop_price),
            probability: Some(probability),
            estimated_time_ms: target_stats.average_duration_ms(),
            favorable_move_pct: target_stats.average_favorable_pct(),
            adverse_move_pct: target_stats.average_adverse_pct(),
        });
    }
    observation
}

pub fn pending(state: &MrcCrossingModel) -> Option<EntryReason> {
    state.pending_entry.clone()
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
        source_line,
        target_line,
        ..
    } = reason
    else {
        return 0;
    };
    match target_line.cmp(source_line) {
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
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
    required_samples: u32,
) {
    const HISTORY_LIMIT: usize = 10_080;
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
        let _ = observe(state, &events, lines, required_samples);
    }
    state.pending_entry = None;
}
