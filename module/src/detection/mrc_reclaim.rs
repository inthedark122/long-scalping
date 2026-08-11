use crate::detection::vlines::MrcCrossEvent;
use crate::model::{EntryReason, MarketState, PositionState};
use crate::state::MrcReclaimState;
use crate::state::{MrcCrossingState, VLineState};
use lte_strategy_bridge::abi::ModuleIndicatorValue;
use lte_strategy_bridge::indicator_access::{
    IndicatorField, IndicatorFieldKey, IndicatorHistory, IndicatorKey, TimeframeSec,
};

pub const BREAK_LINE_MS: u64 = 30 * 60 * 1_000;
pub const FEATURE_WINDOW_MS: u64 = 30 * 60 * 1_000;
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

/// Check that a learned MRC setup has a usable target and stop on the
/// correct side of the entry. When a tick size is supplied, compare the
/// prices after exchange tick normalization as well; two distinct raw MRC
/// lines can otherwise become the same executable price.
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReclaimEvent {
    pub line_idx: u8,
    pub line: f64,
    pub direction: i8,
    pub now_ms: u64,
    pub break_line: bool,
    pub small_ntps: bool,
    pub recent_nearest_crossings: usize,
    pub frequent_nearest_crossings: bool,
    pub buy_volume: bool,
    pub target_line: Option<u8>,
    pub target_price: Option<f64>,
    pub stop_price: Option<f64>,
    pub probability: Option<f64>,
    pub estimated_time_ms: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Observation {
    pub entry: Option<EntryReason>,
    pub wait_reason: Option<String>,
    pub samples: u32,
    pub events: Vec<ReclaimEvent>,
}

pub fn begin(state: &mut MrcReclaimState, now_ms: u64) {
    for first_seen in &mut state.first_seen_ms {
        if *first_seen == 0 {
            *first_seen = now_ms;
        }
    }
}

fn target_for(
    state: &MrcReclaimState,
    source: usize,
    direction: i8,
    use_scenario: bool,
    required_samples: u32,
) -> Option<(usize, u32)> {
    let matrix = if use_scenario {
        &state.scenario_transitions
    } else {
        &state.transitions
    };
    let range: Box<dyn Iterator<Item = usize>> = if direction > 0 {
        Box::new((source + 1)..9)
    } else {
        Box::new((0..source).rev())
    };
    range
        .filter_map(|target| {
            let count = matrix[source][target].count;
            (count >= required_samples).then_some((target, count))
        })
        .max_by_key(|(_, count)| *count)
}

/// Return the strongest learned transition from the current source line.
/// MRC Reclaim needs one specific source -> target transition to reach the
/// required sample count; total line crossings are not interchangeable with
/// that value.
pub fn warmup_samples(state: &MrcReclaimState) -> u32 {
    let Some(source) = state
        .last_line
        .map(usize::from)
        .filter(|source| *source < 9)
    else {
        return 0;
    };
    let use_scenario = state.last_event_small_ntps
        && state.last_event_frequent_nearest
        && state.scenario_source_counts[source] > 0;
    let matrix = if use_scenario {
        &state.scenario_transitions
    } else {
        &state.transitions
    };
    matrix[source]
        .iter()
        .map(|stats| stats.count)
        .max()
        .unwrap_or(0)
}

fn push_recent(state: &mut MrcReclaimState, line: u8, now_ms: u64) {
    if state.recent_len == state.recent_lines.len() {
        state.recent_lines.copy_within(1.., 0);
        state.recent_line_times_ms.copy_within(1.., 0);
        state.recent_len -= 1;
    }
    state.recent_lines[state.recent_len] = line;
    state.recent_line_times_ms[state.recent_len] = now_ms;
    state.recent_len += 1;
}

pub fn observe(
    state: &mut MrcReclaimState,
    events: &[MrcCrossEvent],
    lines: [f64; 9],
    price: f64,
    now_ms: u64,
    buy_sell_gain: f64,
    required_samples: u32,
) -> Observation {
    observe_with_context(
        state,
        events,
        lines,
        price,
        now_ms,
        buy_sell_gain,
        None,
        required_samples,
    )
}

pub fn observe_with_context(
    state: &mut MrcReclaimState,
    events: &[MrcCrossEvent],
    lines: [f64; 9],
    price: f64,
    now_ms: u64,
    buy_sell_gain: f64,
    ntps: Option<(i64, Option<(i64, i64)>)>,
    required_samples: u32,
) -> Observation {
    if state.last_price.is_none() {
        begin(state, now_ms);
    }
    let previous_price = state.last_price.replace(price);
    let mut observation = Observation::default();
    // A reclaim is an event-driven signal. Do not carry an unfilled entry
    // reason into a later tick: its MRC line and direction belong to the
    // crossing that created it. A resting order keeps its reason in
    // EngineState::position.entry_reason after submission/fill.
    state.pending_entry = None;
    let small_ntps = ntps.is_some_and(|(value, range)| {
        range.is_some_and(|(min, max)| {
            value <= min || value as f64 <= min as f64 + (max - min) as f64 / 3.0
        })
    });

    if state.wait_direction != 0 {
        let price_direction = previous_price
            .map(|previous| {
                if price > previous {
                    1
                } else if price < previous {
                    -1
                } else {
                    0
                }
            })
            .unwrap_or(0);
        if price_direction == state.wait_direction
            || (buy_sell_gain.is_finite() && buy_sell_gain.signum() as i8 == state.wait_direction)
        {
            state.wait_direction = 0;
        } else {
            observation.wait_reason = Some("break-line-confirmation".to_string());
        }
    }

    for event in events {
        let source = event.line_idx as usize;
        if source >= 9 {
            continue;
        }
        let cutoff = event.now_ms.saturating_sub(FEATURE_WINDOW_MS);
        while state.recent_len > 0 && state.recent_line_times_ms[0] < cutoff {
            state.recent_lines.copy_within(1..state.recent_len, 0);
            state
                .recent_line_times_ms
                .copy_within(1..state.recent_len, 0);
            state.recent_len -= 1;
        }
        let recent_nearest_crossings = state.recent_lines[..state.recent_len]
            .iter()
            .filter(|line| line.abs_diff(event.line_idx) == 1)
            .count();
        let frequent_nearest_crossings = recent_nearest_crossings >= 2;
        if small_ntps && frequent_nearest_crossings {
            state.scenario_source_counts[source] =
                state.scenario_source_counts[source].saturating_add(1);
        }
        state.source_counts[source] = state.source_counts[source].saturating_add(1);
        if let Some(previous) = state.last_line {
            let previous = previous as usize;
            let duration = event.now_ms.saturating_sub(state.last_event_ms);
            state.transitions[previous][source].count =
                state.transitions[previous][source].count.saturating_add(1);
            state.transitions[previous][source].duration_sum_ms = state.transitions[previous]
                [source]
                .duration_sum_ms
                .saturating_add(duration);
            if state.last_event_small_ntps && state.last_event_frequent_nearest {
                state.scenario_transitions[previous][source].count = state.scenario_transitions
                    [previous][source]
                    .count
                    .saturating_add(1);
                state.scenario_transitions[previous][source].duration_sum_ms = state
                    .scenario_transitions[previous][source]
                    .duration_sum_ms
                    .saturating_add(duration);
            }
        }
        state.last_event_small_ntps = small_ntps;
        state.last_event_frequent_nearest = frequent_nearest_crossings;

        let is_break_line = state.last_cross_ms[source] == 0
            && event.now_ms.saturating_sub(state.first_seen_ms[source]) >= BREAK_LINE_MS;
        state.last_cross_ms[source] = event.now_ms;
        state.last_line = Some(event.line_idx);
        state.last_direction = event.direction;
        state.last_event_ms = event.now_ms;

        if is_break_line {
            state.wait_direction = event.direction;
            state.pending_entry = None;
            observation.entry = None;
            observation.wait_reason = Some(format!("break-line-{}", event.line_idx));
            observation.events.push(ReclaimEvent {
                line_idx: event.line_idx,
                line: event.line,
                direction: event.direction,
                now_ms: event.now_ms,
                break_line: true,
                small_ntps,
                recent_nearest_crossings,
                frequent_nearest_crossings,
                buy_volume: buy_sell_gain > 0.0,
                target_line: None,
                target_price: None,
                stop_price: None,
                probability: None,
                estimated_time_ms: None,
            });
            push_recent(state, event.line_idx, event.now_ms);
            continue;
        }

        if state.wait_direction != 0 {
            observation.entry = None;
            observation.events.push(ReclaimEvent {
                line_idx: event.line_idx,
                line: event.line,
                direction: event.direction,
                now_ms: event.now_ms,
                break_line: false,
                small_ntps,
                recent_nearest_crossings,
                frequent_nearest_crossings,
                buy_volume: buy_sell_gain > 0.0,
                target_line: None,
                target_price: None,
                stop_price: None,
                probability: None,
                estimated_time_ms: None,
            });
            push_recent(state, event.line_idx, event.now_ms);
            continue;
        }

        let use_scenario =
            small_ntps && frequent_nearest_crossings && state.scenario_source_counts[source] > 0;
        let Some((target, count)) = target_for(
            state,
            source,
            event.direction,
            use_scenario,
            required_samples,
        ) else {
            observation.samples = warmup_samples(state);
            observation.wait_reason = Some("mrc-warmup".to_string());
            observation.events.push(ReclaimEvent {
                line_idx: event.line_idx,
                line: event.line,
                direction: event.direction,
                now_ms: event.now_ms,
                break_line: false,
                small_ntps,
                recent_nearest_crossings,
                frequent_nearest_crossings,
                buy_volume: buy_sell_gain > 0.0,
                target_line: None,
                target_price: None,
                stop_price: None,
                probability: None,
                estimated_time_ms: None,
            });
            push_recent(state, event.line_idx, event.now_ms);
            continue;
        };
        let stop = if event.direction > 0 {
            source.checked_sub(1)
        } else {
            (source + 1 < 9).then_some(source + 1)
        };
        let Some(stop) = stop else {
            continue;
        };
        let target_line = lines[target];
        let stop_line = lines[stop];
        let stats = if use_scenario {
            state.scenario_transitions[source][target]
        } else {
            state.transitions[source][target]
        };
        let duration = stats.average_duration_ms().unwrap_or(0.0);
        let source_observations = if use_scenario {
            state.scenario_source_counts[source]
        } else {
            state.source_counts[source]
        };
        let probability = count as f64 / source_observations.max(1) as f64;
        let side = if event.direction > 0 {
            PositionState::Long
        } else {
            PositionState::Short
        };
        if matches!(side, PositionState::Long | PositionState::Short) {
            let reason = EntryReason::MrcReclaim {
                price: event.line,
                target_price: target_line,
                stop_price: stop_line,
                source_line: event.line_idx,
                target_line: target as u8,
                probability,
                estimated_time_ms: duration,
            };
            state.pending_entry = Some(reason.clone());
            observation.entry = Some(reason);
            observation.samples = count;
            observation.events.push(ReclaimEvent {
                line_idx: event.line_idx,
                line: event.line,
                direction: event.direction,
                now_ms: event.now_ms,
                break_line: false,
                small_ntps,
                recent_nearest_crossings,
                frequent_nearest_crossings,
                buy_volume: buy_sell_gain > 0.0,
                target_line: Some(target as u8),
                target_price: Some(target_line),
                stop_price: Some(stop_line),
                probability: Some(probability),
                estimated_time_ms: Some(duration),
            });
        }
        push_recent(state, event.line_idx, event.now_ms);
    }
    observation
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

/// Return the trade direction encoded by an MRC source -> target transition.
/// This is deliberately derived from the reason itself, not from the latest
/// crossing, because a pending order may outlive the crossing that created it.
pub fn reason_direction(reason: &EntryReason) -> i8 {
    let EntryReason::MrcReclaim {
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

pub fn pending(state: &MrcReclaimState) -> Option<EntryReason> {
    state.pending_entry.clone()
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
    state: &mut MrcReclaimState,
    vlines: &mut [VLineState; 9],
    crossing: &mut MrcCrossingState,
    history: &IndicatorHistory,
    now_ms: u64,
    required_samples: u32,
) {
    // IndicatorHistory is newest-first. Replay oldest-to-newest so the same
    // crossing and duration logic is used for startup data and live ticks.
    let seed_start = now_ms.saturating_sub(9 * 60 * 1_000);
    for offset in (0..10).rev() {
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
        let now = seed_start.saturating_add((9 - offset) as u64 * 60 * 1_000);
        let events = crate::detection::vlines::update_with_events(
            vlines,
            crossing,
            lines,
            price,
            offset as u64 + 1,
            now,
        );
        let _ = observe(state, &events, lines, price, now, 0.0, required_samples);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detection::vlines::MrcCrossEvent;

    fn event(line_idx: u8, direction: i8, now_ms: u64) -> MrcCrossEvent {
        MrcCrossEvent {
            line_idx,
            line: 100.0 + line_idx as f64,
            direction,
            tick: now_ms,
            now_ms,
        }
    }

    #[test]
    fn learns_three_transitions_and_estimated_duration() {
        let mut state = MrcReclaimState::default();
        begin(&mut state, 1);
        for n in 0..3 {
            observe(
                &mut state,
                &[event(4, 1, 10 + n * 100)],
                [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0],
                100.0,
                10 + n * 100,
                0.0,
                3,
            );
            observe(
                &mut state,
                &[event(5, 1, 60 + n * 100)],
                [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0],
                105.0,
                60 + n * 100,
                0.0,
                3,
            );
        }
        assert_eq!(state.transitions[4][5].count, 3);
        assert_eq!(state.transitions[4][5].average_duration_ms(), Some(50.0));
        let result = observe(
            &mut state,
            &[event(4, 1, 500)],
            [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0],
            100.0,
            500,
            0.0,
            3,
        );
        assert!(matches!(
            result.entry,
            Some(EntryReason::MrcReclaim { target_line: 5, .. })
        ));
    }

    #[test]
    fn break_line_enters_confirmation_wait() {
        let mut state = MrcReclaimState::default();
        begin(&mut state, 1);
        let result = observe(
            &mut state,
            &[event(4, 1, BREAK_LINE_MS + 2)],
            [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0],
            100.0,
            BREAK_LINE_MS + 2,
            0.0,
            3,
        );
        assert!(result.entry.is_none());
        assert_eq!(state.wait_direction, 1);
        let result = observe(
            &mut state,
            &[],
            [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0],
            101.0,
            BREAK_LINE_MS + 3,
            0.0,
            3,
        );
        assert!(result.wait_reason.is_none());
        assert_eq!(state.wait_direction, 0);
    }

    #[test]
    fn price_levels_require_directional_separation() {
        assert!(valid_price_levels(1, 100.0, 104.0, 96.0, None));
        assert!(valid_price_levels(-1, 100.0, 96.0, 104.0, None));
        assert!(!valid_price_levels(1, 100.0, 100.0, 96.0, None));
        assert!(!valid_price_levels(-1, 100.0, 96.0, 100.0, None));
    }

    #[test]
    fn price_levels_require_separation_after_tick_normalization() {
        assert!(!valid_price_levels(
            1,
            0.1182,
            0.11824,
            0.11816,
            Some(0.0001),
        ));
        assert!(valid_price_levels(1, 0.1182, 0.1183, 0.1181, Some(0.0001),));
    }

    #[test]
    fn warmup_samples_reports_the_strongest_current_transition() {
        let mut state = MrcReclaimState {
            last_line: Some(4),
            ..Default::default()
        };
        state.transitions[4][5].count = 2;
        state.transitions[4][3].count = 1;
        assert_eq!(warmup_samples(&state), 2);
    }

    #[test]
    fn required_samples_setting_controls_transition_warmup() {
        let lines = [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0];

        let mut zero_state = MrcReclaimState::default();
        let immediate = observe(&mut zero_state, &[event(4, 1, 1)], lines, 100.0, 1, 0.0, 0);
        assert!(matches!(
            immediate.entry,
            Some(EntryReason::MrcReclaim { .. })
        ));

        let mut state = MrcReclaimState::default();
        state.transitions[4][5].count = 3;

        let waiting = observe(&mut state, &[event(4, 1, 1)], lines, 100.0, 1, 0.0, 4);
        assert!(waiting.entry.is_none());
        assert_eq!(waiting.samples, 3);

        state.transitions[4][5].count = 4;
        let ready = observe(&mut state, &[event(4, 1, 2)], lines, 100.0, 2, 0.0, 4);
        assert!(matches!(
            ready.entry,
            Some(EntryReason::MrcReclaim { target_line: 5, .. })
        ));
    }

    #[test]
    fn reason_direction_comes_from_source_to_target_transition() {
        let long = EntryReason::MrcReclaim {
            price: 100.0,
            target_price: 104.0,
            stop_price: 96.0,
            source_line: 4,
            target_line: 5,
            probability: 0.5,
            estimated_time_ms: 0.0,
        };
        let short = EntryReason::MrcReclaim {
            price: 100.0,
            target_price: 96.0,
            stop_price: 104.0,
            source_line: 4,
            target_line: 3,
            probability: 0.5,
            estimated_time_ms: 0.0,
        };
        assert_eq!(reason_direction(&long), 1);
        assert_eq!(reason_direction(&short), -1);
    }

    #[test]
    fn pending_entry_expires_without_a_new_crossing() {
        let mut state = MrcReclaimState::default();
        state.transitions[4][5].count = 3;
        let lines = [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0];

        let first = observe(&mut state, &[event(4, 1, 1)], lines, 100.0, 1, 0.0, 3);
        assert!(first.entry.is_some());
        assert!(state.pending_entry.is_some());

        let second = observe(&mut state, &[], lines, 101.0, 2, 0.0, 3);
        assert!(second.entry.is_none());
        assert!(state.pending_entry.is_none());
    }
}
