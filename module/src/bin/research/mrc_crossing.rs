use std::collections::BTreeMap;
use std::time::Instant;

use long_scalping::detection::cascade::{CascadeTradeState, MovementPhase};
use long_scalping::detection::mrc_pattern_classification::{
    MrcPatternKind, StaircaseLifecycle, StaircaseTransition, MRC_PATTERN_COUNT,
};
use long_scalping::detection::mrc_prediction::MrcLinePlan;
use long_scalping::detection::{
    cascade, decision::DecisionComponents, mrc_cross_analysis, mrc_crossing, mrc_prediction,
    mrc_touch, mrc_touch_learning, vlines,
};
use long_scalping::model::Candle;
use long_scalping::state::{MrcCrossingModel, MrcCrossingState, VLineState};
use long_scalping::{fmt_duration_f64, fmt_duration_ms};
use serde::Serialize;

use super::data::CandleRow;
use super::virtual_trade::{
    TradeSetup, VirtualPosition, VirtualPredictionResult, VirtualPredictionSimulator,
    VirtualPredictionSummary, VirtualTradeResult, VirtualTradeSimulator, VirtualTradeSummary,
};

const LINE_COUNT: usize = 9;
const LABELS: [&str; LINE_COUNT] = mrc_crossing::LINE_LABELS;

fn duration_token(value: String) -> String {
    value.replace(' ', "")
}

fn sequence_duration(duration_ms: u64) -> String {
    let seconds = duration_ms / 1_000;
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        let minutes = seconds / 60;
        let remainder = seconds % 60;
        if remainder == 0 {
            format!("{minutes}m")
        } else {
            format!("{minutes}m{remainder}s")
        }
    }
}

fn active_move_percentages(crossing: &MrcCrossingState) -> Option<(f64, f64)> {
    let start = if crossing.movement_market_start > 0.0 {
        crossing.movement_market_start
    } else {
        crossing.movement_start
    };
    if start <= 0.0 || crossing.movement_direction == 0 {
        return None;
    }
    if crossing.movement_direction > 0 {
        Some((
            (crossing.movement_high - start) / start * 100.0,
            (crossing.movement_low - start) / start * 100.0,
        ))
    } else {
        Some((
            (start - crossing.movement_low) / start * 100.0,
            (start - crossing.movement_high) / start * 100.0,
        ))
    }
}

fn active_next_progress(crossing: &MrcCrossingState) -> Option<f64> {
    let start = if crossing.movement_market_start > 0.0 {
        crossing.movement_market_start
    } else {
        crossing.movement_start
    };
    let current = crossing.last_price?;
    if start <= 0.0 || crossing.movement_target <= 0.0 || crossing.movement_direction == 0 {
        return None;
    }
    let distance = if crossing.movement_direction > 0 {
        crossing.movement_target - start
    } else {
        start - crossing.movement_target
    };
    if distance <= 0.0 {
        return None;
    }
    let progress = if crossing.movement_direction > 0 {
        (current - start) / distance * 100.0
    } else {
        (start - current) / distance * 100.0
    };
    Some(progress.clamp(0.0, 100.0))
}

fn format_mrc_trace(
    crossing: &MrcCrossingState,
    line: u8,
    direction: i8,
    realized_movement_direction: Option<i8>,
    realized_favorable_move_pct: Option<f64>,
    realized_adverse_move_pct: Option<f64>,
    movement_completed: bool,
    cross_count: u32,
) -> String {
    let direction_name = match direction {
        1 => "long",
        -1 => "short",
        _ => "na",
    };
    let sequence = (0..crossing.sequence_len)
        .map(|index| {
            let label = LABELS[crossing.sequence[index] as usize];
            if index + 1 >= crossing.sequence_len {
                label.to_string()
            } else {
                format!(
                    "{label}|{}",
                    sequence_duration(
                        crossing.sequence_times_ms[index + 1]
                            .saturating_sub(crossing.sequence_times_ms[index])
                    )
                )
            }
        })
        .collect::<Vec<_>>()
        .join(">");
    let movement_direction = realized_movement_direction.unwrap_or(direction);
    let movement = realized_favorable_move_pct
        .zip(realized_adverse_move_pct)
        .or_else(|| active_move_percentages(crossing))
        .map(|(favorable, adverse)| {
            format!(
                "{}:+{:.2}%:-{:.2}%",
                if movement_direction > 0 {
                    "long"
                } else {
                    "short"
                },
                favorable.max(0.0),
                adverse.min(0.0)
            )
        })
        .unwrap_or_else(|| "na".to_string());
    let next = movement_completed
        .then_some("100.00%".to_string())
        .or_else(|| active_next_progress(crossing).map(|progress| format!("{progress:.2}%")))
        .unwrap_or_else(|| "na".to_string());
    format!(
        "[last={}:{}:{} track=na seq={} no_pullback={} revert=long:{}:short:{} move={} next={}]",
        LABELS[line as usize],
        direction_name,
        if cross_count <= 1 { "first" } else { "repeat" },
        if sequence.is_empty() { "na" } else { &sequence },
        crossing.no_pullback_count,
        crossing
            .reverted_long_line
            .map(|line| LABELS[line as usize])
            .unwrap_or("na"),
        crossing
            .reverted_short_line
            .map(|line| LABELS[line as usize])
            .unwrap_or("na"),
        movement,
        next,
    )
}

fn direction_name(direction: i8) -> &'static str {
    match direction {
        1 => "long",
        -1 => "short",
        _ => "na",
    }
}

fn format_line_sequence(sequence: &[usize]) -> String {
    if sequence.is_empty() {
        return "na".to_string();
    }
    sequence
        .iter()
        .filter_map(|line| LABELS.get(*line).copied())
        .collect::<Vec<_>>()
        .join(">")
        .to_string()
}

fn history_scope(context_samples: u32, has_prediction: bool) -> &'static str {
    if !has_prediction {
        "none"
    } else if context_samples > 0 {
        "contextual"
    } else {
        "broad-fallback"
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CrossingEvent {
    pub(crate) time_ms: i64,
    pub(crate) elapsed_ms: u64,
    pub(crate) line: usize,
    pub(crate) line_name: &'static str,
    pub(crate) direction: i8,
    pub(crate) price: f64,
    pub(crate) market_price: f64,
    pub(crate) mrc_min: f64,
    pub(crate) mrc_max: f64,
    pub(crate) mrc_trace: String,
    pub(crate) line_plans: [MrcLinePlan; LINE_COUNT],
    pub(crate) ntps: i64,
    pub(crate) cascade_direction: i8,
    pub(crate) cascade_speed_pct_per_min: Option<f64>,
    pub(crate) barrier_line: Option<usize>,
    pub(crate) barrier_line_name: Option<&'static str>,
    pub(crate) barrier_price: Option<f64>,
    pub(crate) barrier_state: String,
    pub(crate) history_scope: &'static str,
    pub(crate) pattern_kind: Option<&'static str>,
    pub(crate) pattern_steps: u32,
    pub(crate) pattern_elapsed_ms: u64,
    pub(crate) stair_sequence: Vec<usize>,
    pub(crate) stair_price_move_pct: f64,
    pub(crate) stair_favorable_move_pct: f64,
    pub(crate) stair_adverse_move_pct: f64,
    pub(crate) staircase_lifecycle: Option<StaircaseLifecycle>,
    pub(crate) staircase_transition: Option<StaircaseTransition>,
    pub(crate) staircase_protected_line: Option<usize>,
    pub(crate) staircase_extreme_line: Option<usize>,
    pub(crate) cascade_enabled: bool,
    pub(crate) cascade_phase: MovementPhase,
    pub(crate) cascade_potential_move_pct: Option<f64>,
    pub(crate) cascade_adverse_move_pct: Option<f64>,
    pub(crate) cascade_trade_state: CascadeTradeState,
    pub(crate) predicted_direction: i8,
    pub(crate) trade_direction: i8,
    pub(crate) predicted_line_price: Option<f64>,
    pub(crate) take_profit_line: Option<usize>,
    pub(crate) take_profit_name: Option<&'static str>,
    pub(crate) stop_loss_line: Option<usize>,
    pub(crate) stop_loss_name: Option<&'static str>,
    pub(crate) trade_target_line: Option<usize>,
    pub(crate) trade_target_name: Option<&'static str>,
    pub(crate) effective_move_pct: Option<f64>,
    pub(crate) effective_entry_price: Option<f64>,
    pub(crate) predicted_end_price: Option<f64>,
    pub(crate) volume_gain: f64,
    pub(crate) volume_gain_min: f64,
    pub(crate) volume_gain_max: f64,
    pub(crate) buy_sell_gain: f64,
    pub(crate) target_line: Option<usize>,
    pub(crate) target_name: Option<&'static str>,
    pub(crate) target_price: Option<f64>,
    pub(crate) stop_price: Option<f64>,
    pub(crate) pattern: Vec<usize>,
    pub(crate) pattern_names: Vec<&'static str>,
    pub(crate) pattern_samples: u32,
    pub(crate) context_samples: u32,
    pub(crate) no_pullback_count: u32,
    pub(crate) reverted_line: Option<usize>,
    pub(crate) reverted_line_name: Option<&'static str>,
    pub(crate) target_samples: u32,
    pub(crate) movement_samples: u32,
    pub(crate) next_line_crosses: u32,
    pub(crate) support_resistance_moves: u32,
    pub(crate) price_only_moves: u32,
    pub(crate) successful_moves: u32,
    pub(crate) failed_moves: u32,
    pub(crate) movement_success_rate: Option<f64>,
    pub(crate) expected_value_pct: Option<f64>,
    pub(crate) expected_price_low: Option<f64>,
    pub(crate) expected_price_high: Option<f64>,
    pub(crate) realized_duration_ms: Option<u64>,
    pub(crate) realized_movement_direction: Option<i8>,
    pub(crate) realized_favorable_move_pct: Option<f64>,
    pub(crate) realized_adverse_move_pct: Option<f64>,
    pub(crate) trace_outcome: Option<&'static str>,
    pub(crate) trace_result_move_pct: Option<f64>,
    pub(crate) prediction_side: Option<i8>,
    pub(crate) prediction_filled: Option<bool>,
    pub(crate) prediction_outcome: Option<&'static str>,
    pub(crate) prediction_result_move_pct: Option<f64>,
    pub(crate) prediction_resolved_at: Option<i64>,
    pub(crate) position_outcome: Option<&'static str>,
    pub(crate) position_pnl_pct: Option<f64>,
    pub(crate) position_mfe_pct: Option<f64>,
    pub(crate) position_mae_pct: Option<f64>,
    pub(crate) position_exit_reason: Option<String>,
    pub(crate) realized_price_high: Option<f64>,
    pub(crate) realized_price_low: Option<f64>,
    pub(crate) realized_target_low: Option<f64>,
    pub(crate) realized_target_high: Option<f64>,
    pub(crate) realized_mrc_low: Option<f64>,
    pub(crate) realized_mrc_high: Option<f64>,
    pub(crate) pattern_key: String,
}

fn format_pattern_key(event: &CrossingEvent) -> String {
    event.pattern_kind.unwrap_or("none").to_string()
}

fn prediction_result_move_pct(prediction: &VirtualPredictionResult) -> Option<f64> {
    let result_price = match prediction.outcome {
        "reached_and_followed" | "not_reached_but_followed" => prediction.target_price,
        "prediction_broken" => prediction.stop_price,
        _ => return None,
    };
    if !prediction.entry_price.is_finite()
        || prediction.entry_price <= 0.0
        || !result_price.is_finite()
        || !matches!(prediction.side, 1 | -1)
    {
        return None;
    }
    Some(
        (result_price - prediction.entry_price) / prediction.entry_price
            * 100.0
            * prediction.side as f64,
    )
}

fn crossing_event_from_analysis(
    event: &mrc_cross_analysis::MrcCrossAnalysis,
    crossing: &MrcCrossingState,
    previous_time_ms: Option<i64>,
    cross_count: u32,
) -> CrossingEvent {
    let mut crossing_event = CrossingEvent {
        time_ms: event.now_ms as i64,
        elapsed_ms: previous_time_ms
            .map(|previous| (event.now_ms as i64).saturating_sub(previous) as u64)
            .unwrap_or(0),
        line: event.line_idx as usize,
        line_name: LABELS[event.line_idx as usize],
        direction: event.direction,
        price: event.line,
        market_price: event.market_price,
        mrc_min: event.mrc_min,
        mrc_max: event.mrc_max,
        barrier_line: event.barrier_line.map(|line| line as usize),
        barrier_line_name: event.barrier_line.map(|line| LABELS[line as usize]),
        barrier_price: event.barrier_price,
        barrier_state: event.barrier_state.as_str().to_string(),
        history_scope: history_scope(event.context_samples, event.target_line.is_some()),
        pattern_kind: event.pattern_kind.map(|kind| kind.as_str()),
        pattern_steps: event.pattern_steps,
        pattern_elapsed_ms: event.pattern_elapsed_ms,
        stair_sequence: event
            .stair_sequence
            .iter()
            .map(|line| *line as usize)
            .collect(),
        stair_price_move_pct: event.stair_price_move_pct,
        stair_favorable_move_pct: event.stair_favorable_move_pct,
        stair_adverse_move_pct: event.stair_adverse_move_pct,
        staircase_lifecycle: event.staircase_lifecycle,
        staircase_transition: event.staircase_transition,
        staircase_protected_line: event.staircase_protected_line.map(|line| line as usize),
        staircase_extreme_line: event.staircase_extreme_line.map(|line| line as usize),
        cascade_enabled: event.cascade_enabled,
        cascade_phase: event.cascade_phase,
        cascade_potential_move_pct: event.cascade_potential_move_pct,
        cascade_adverse_move_pct: event.cascade_adverse_move_pct,
        cascade_trade_state: event.cascade_trade_state,
        predicted_direction: event.predicted_direction,
        trade_direction: event.trade_direction,
        predicted_line_price: event.predicted_line_price,
        take_profit_line: event.take_profit_line.map(|line| line as usize),
        take_profit_name: event.take_profit_line.map(|line| LABELS[line as usize]),
        stop_loss_line: event.stop_loss_line.map(|line| line as usize),
        stop_loss_name: event.stop_loss_line.map(|line| LABELS[line as usize]),
        trade_target_line: event.trade_target_line.map(|line| line as usize),
        trade_target_name: event.trade_target_line.map(|line| LABELS[line as usize]),
        effective_move_pct: event.effective_move_pct,
        effective_entry_price: event.effective_entry_price,
        predicted_end_price: event.predicted_end_price,
        volume_gain: event.volume_gain,
        volume_gain_min: event.volume_gain_min,
        volume_gain_max: event.volume_gain_max,
        buy_sell_gain: event.buy_sell_gain,
        mrc_trace: format_mrc_trace(
            crossing,
            event.line_idx,
            event.direction,
            event.realized_movement_direction,
            event.realized_favorable_move_pct,
            event.realized_adverse_move_pct,
            event.realized_duration_ms.is_some(),
            cross_count,
        ),
        line_plans: event.line_plans,
        ntps: event
            .line_plans
            .first()
            .map(|plan| plan.ntps)
            .unwrap_or_default(),
        cascade_direction: event
            .line_plans
            .first()
            .map(|plan| plan.cascade_direction)
            .unwrap_or_default(),
        cascade_speed_pct_per_min: event
            .line_plans
            .iter()
            .find_map(|plan| plan.cascade_speed_pct_per_min),
        target_line: event.target_line.map(|line| line as usize),
        target_name: event.target_line.map(|line| LABELS[line as usize]),
        target_price: event.target_price,
        stop_price: event.stop_price,
        pattern: event.pattern.iter().map(|line| *line as usize).collect(),
        pattern_names: event
            .pattern
            .iter()
            .map(|line| LABELS[*line as usize])
            .collect(),
        pattern_samples: event.pattern_samples,
        context_samples: event.context_samples,
        no_pullback_count: event.no_pullback_count,
        reverted_line: event.reverted_line.map(|line| line as usize),
        reverted_line_name: event.reverted_line.map(|line| LABELS[line as usize]),
        target_samples: event.target_samples,
        movement_samples: event.movement_samples,
        next_line_crosses: event.next_line_crosses,
        support_resistance_moves: event.support_resistance_moves,
        price_only_moves: event.price_only_moves,
        successful_moves: event.successful_moves,
        failed_moves: event.failed_moves,
        movement_success_rate: event.movement_success_rate,
        expected_value_pct: event.expected_value_pct,
        expected_price_low: event.expected_price_low,
        expected_price_high: event.expected_price_high,
        realized_duration_ms: event.realized_duration_ms,
        realized_movement_direction: event.realized_movement_direction,
        realized_favorable_move_pct: event.realized_favorable_move_pct,
        realized_adverse_move_pct: event.realized_adverse_move_pct,
        trace_outcome: event
            .realized_favorable_move_pct
            .zip(event.realized_adverse_move_pct)
            .and_then(|(favorable, adverse)| {
                mrc_cross_analysis::movement_outcome(favorable, adverse).map(|(outcome, _)| outcome)
            }),
        trace_result_move_pct: event
            .realized_favorable_move_pct
            .zip(event.realized_adverse_move_pct)
            .and_then(|(favorable, adverse)| {
                mrc_cross_analysis::movement_outcome(favorable, adverse).map(|(_, result)| result)
            }),
        prediction_side: None,
        prediction_filled: None,
        prediction_outcome: None,
        prediction_result_move_pct: None,
        prediction_resolved_at: None,
        position_outcome: None,
        position_pnl_pct: None,
        position_mfe_pct: None,
        position_mae_pct: None,
        position_exit_reason: None,
        realized_price_high: event.realized_price_high,
        realized_price_low: event.realized_price_low,
        realized_target_low: event.realized_target_low,
        realized_target_high: event.realized_target_high,
        realized_mrc_low: event.realized_mrc_low,
        realized_mrc_high: event.realized_mrc_high,
        pattern_key: String::new(),
    };
    crossing_event.pattern_key = format_pattern_key(&crossing_event);
    crossing_event
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PatternWinRateReport {
    pub(crate) pattern: String,
    pub(crate) matches: usize,
    pub(crate) filled_predictions: usize,
    pub(crate) closed_positions: usize,
    pub(crate) wins: usize,
    pub(crate) losses: usize,
    pub(crate) win_rate_pct: f64,
    pub(crate) total_gross_pnl_pct: f64,
    pub(crate) total_fees_pct: f64,
    pub(crate) total_pnl_pct: f64,
    pub(crate) average_win_pnl_pct: f64,
    pub(crate) average_loss_pnl_pct: f64,
    pub(crate) followed_predictions: usize,
    pub(crate) broken_predictions: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PatternReport {
    pub(crate) pattern_kind: &'static str,
    pub(crate) pattern: Vec<usize>,
    pub(crate) pattern_names: Vec<&'static str>,
    pub(crate) to: usize,
    pub(crate) to_name: &'static str,
    pub(crate) count: usize,
    pub(crate) movement_samples: u32,
    pub(crate) next_line_crosses: u32,
    pub(crate) support_resistance_moves: u32,
    pub(crate) price_only_moves: u32,
    pub(crate) successful_moves: u32,
    pub(crate) failed_moves: u32,
    pub(crate) movement_success_rate: Option<f64>,
    pub(crate) expected_value_pct: Option<f64>,
    pub(crate) average_duration_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Forecast {
    pub(crate) source_line: Option<usize>,
    pub(crate) source_name: Option<&'static str>,
    pub(crate) condition: &'static str,
    pub(crate) sample_count: usize,
    pub(crate) estimated_time_ms: Option<f64>,
    pub(crate) likely_destinations: Vec<PatternReport>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnalysisReport {
    pub(crate) crossing_count: usize,
    pub(crate) pattern_count: usize,
    pub(crate) patterns: Vec<PatternReport>,
    pub(crate) latest_event: Option<CrossingEvent>,
    pub(crate) forecast: Forecast,
    pub(crate) events: Vec<CrossingEvent>,
    pub(crate) trade_summary: VirtualTradeSummary,
    pub(crate) trades: Vec<VirtualTradeResult>,
    pub(crate) open_trade: Option<VirtualPosition>,
    pub(crate) prediction_summary: VirtualPredictionSummary,
    pub(crate) predictions: Vec<VirtualPredictionResult>,
    pub(crate) pending_predictions: usize,
    pub(crate) active_predictions: usize,
    pub(crate) pattern_win_rates: Vec<PatternWinRateReport>,
    pub(crate) stair_statistics: StairStatistics,
    pub(crate) pattern_performance: Vec<PatternPerformanceReport>,
    pub(crate) parallel_pattern_trades: Vec<PatternTradeReport>,
    pub(crate) active_pattern: Option<&'static str>,
    pub(crate) active_pattern_score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PatternPerformanceReport {
    pub(crate) pattern: &'static str,
    pub(crate) observations: u64,
    pub(crate) trades: u64,
    pub(crate) wins: u64,
    pub(crate) win_rate_pct: f64,
    pub(crate) total_pnl_pct: f64,
    pub(crate) average_pnl_pct: f64,
    pub(crate) ema_pnl_pct: f64,
    pub(crate) last_pnl_pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PatternTradeReport {
    pub(crate) pattern: &'static str,
    pub(crate) summary: VirtualTradeSummary,
    pub(crate) trades: Vec<VirtualTradeResult>,
    pub(crate) open_trade: Option<VirtualPosition>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct StairStepStatistics {
    pub(crate) pattern: &'static str,
    pub(crate) steps: u32,
    pub(crate) observations: usize,
    pub(crate) average_elapsed_ms: f64,
    pub(crate) average_volume_gain: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct StairPatternStatistics {
    pub(crate) pattern: &'static str,
    pub(crate) observations: usize,
    pub(crate) average_steps: f64,
    pub(crate) average_elapsed_ms: f64,
    pub(crate) average_volume_gain: f64,
    pub(crate) prediction_matches: usize,
    pub(crate) filled_predictions: usize,
    pub(crate) followed_predictions: usize,
    pub(crate) broken_predictions: usize,
    pub(crate) closed_positions: usize,
    pub(crate) wins: usize,
    pub(crate) losses: usize,
    pub(crate) win_rate_pct: f64,
    pub(crate) total_gross_pnl_pct: f64,
    pub(crate) total_fees_pct: f64,
    pub(crate) total_pnl_pct: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct StairStatistics {
    pub(crate) observations: usize,
    pub(crate) prediction_matches: usize,
    pub(crate) filled_predictions: usize,
    pub(crate) followed_predictions: usize,
    pub(crate) broken_predictions: usize,
    pub(crate) closed_positions: usize,
    pub(crate) wins: usize,
    pub(crate) losses: usize,
    pub(crate) win_rate_pct: f64,
    pub(crate) total_gross_pnl_pct: f64,
    pub(crate) total_fees_pct: f64,
    pub(crate) total_pnl_pct: f64,
    pub(crate) by_pattern: Vec<StairPatternStatistics>,
    pub(crate) by_steps: Vec<StairStepStatistics>,
}

fn pattern_win_rate_reports(
    trades: &[VirtualTradeResult],
    predictions: &[VirtualPredictionResult],
) -> Vec<PatternWinRateReport> {
    let mut grouped_trades: BTreeMap<String, Vec<&VirtualTradeResult>> = BTreeMap::new();
    for trade in trades {
        grouped_trades
            .entry(trade.pattern_key.clone())
            .or_default()
            .push(trade);
    }
    let mut grouped_predictions: BTreeMap<String, Vec<&VirtualPredictionResult>> = BTreeMap::new();
    for prediction in predictions {
        grouped_predictions
            .entry(prediction.pattern_key.clone())
            .or_default()
            .push(prediction);
    }
    let pattern_keys = grouped_trades
        .keys()
        .chain(grouped_predictions.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut reports = pattern_keys
        .into_iter()
        .map(|pattern| {
            let trades = grouped_trades.get(&pattern).cloned().unwrap_or_default();
            let predictions = grouped_predictions
                .get(&pattern)
                .cloned()
                .unwrap_or_default();
            let wins = trades.iter().filter(|trade| trade.pnl_pct > 0.0).count();
            let losses = trades.len().saturating_sub(wins);
            let winning_pnl = trades
                .iter()
                .filter(|trade| trade.pnl_pct > 0.0)
                .map(|trade| trade.pnl_pct)
                .sum::<f64>();
            let losing_pnl = trades
                .iter()
                .filter(|trade| trade.pnl_pct <= 0.0)
                .map(|trade| trade.pnl_pct)
                .sum::<f64>();
            PatternWinRateReport {
                pattern,
                matches: predictions.len(),
                filled_predictions: predictions
                    .iter()
                    .filter(|prediction| prediction.filled)
                    .count(),
                closed_positions: trades.len(),
                wins,
                losses,
                win_rate_pct: wins as f64 / trades.len().max(1) as f64 * 100.0,
                total_gross_pnl_pct: trades.iter().map(|trade| trade.gross_pnl_pct).sum(),
                total_fees_pct: trades
                    .iter()
                    .map(|trade| trade.entry_fee_pct + trade.exit_fee_pct)
                    .sum(),
                total_pnl_pct: trades.iter().map(|trade| trade.pnl_pct).sum(),
                average_win_pnl_pct: winning_pnl / wins.max(1) as f64,
                average_loss_pnl_pct: losing_pnl / losses.max(1) as f64,
                followed_predictions: predictions
                    .iter()
                    .filter(|prediction| {
                        prediction.outcome == "reached_and_followed"
                            || prediction.outcome == "not_reached_but_followed"
                    })
                    .count(),
                broken_predictions: predictions
                    .iter()
                    .filter(|prediction| prediction.outcome == "prediction_broken")
                    .count(),
            }
        })
        .collect::<Vec<_>>();
    reports.sort_by(|left, right| {
        right
            .matches
            .cmp(&left.matches)
            .then_with(|| left.pattern.cmp(&right.pattern))
    });
    reports
}

fn average_f64(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, count) = values.fold((0.0, 0usize), |(sum, count), value| {
        if value.is_finite() {
            (sum + value, count + 1)
        } else {
            (sum, count)
        }
    });
    if count == 0 {
        0.0
    } else {
        sum / count as f64
    }
}

fn stair_statistics(
    events: &[CrossingEvent],
    predictions: &[VirtualPredictionResult],
    trades: &[VirtualTradeResult],
) -> StairStatistics {
    const STAIR_PATTERNS: [&str; 2] = ["STAIRS_UP", "STAIRS_DOWN"];
    let mut by_pattern = Vec::with_capacity(STAIR_PATTERNS.len());
    for pattern in STAIR_PATTERNS {
        let observations = events
            .iter()
            .filter(|event| event.pattern_kind == Some(pattern))
            .collect::<Vec<_>>();
        let pattern_predictions = predictions
            .iter()
            .filter(|prediction| prediction.pattern_key == pattern)
            .collect::<Vec<_>>();
        let pattern_trades = trades
            .iter()
            .filter(|trade| trade.pattern_key == pattern)
            .collect::<Vec<_>>();
        let wins = pattern_trades
            .iter()
            .filter(|trade| trade.pnl_pct > 0.0)
            .count();
        let losses = pattern_trades.len().saturating_sub(wins);

        by_pattern.push(StairPatternStatistics {
            pattern,
            observations: observations.len(),
            average_steps: average_f64(observations.iter().map(|event| event.pattern_steps as f64)),
            average_elapsed_ms: average_f64(
                observations
                    .iter()
                    .map(|event| event.pattern_elapsed_ms as f64),
            ),
            average_volume_gain: average_f64(observations.iter().map(|event| event.volume_gain)),
            prediction_matches: pattern_predictions.len(),
            filled_predictions: pattern_predictions
                .iter()
                .filter(|prediction| prediction.filled)
                .count(),
            followed_predictions: pattern_predictions
                .iter()
                .filter(|prediction| {
                    prediction.outcome == "reached_and_followed"
                        || prediction.outcome == "not_reached_but_followed"
                })
                .count(),
            broken_predictions: pattern_predictions
                .iter()
                .filter(|prediction| prediction.outcome == "prediction_broken")
                .count(),
            closed_positions: pattern_trades.len(),
            wins,
            losses,
            win_rate_pct: wins as f64 / pattern_trades.len().max(1) as f64 * 100.0,
            total_gross_pnl_pct: pattern_trades.iter().map(|trade| trade.gross_pnl_pct).sum(),
            total_fees_pct: pattern_trades
                .iter()
                .map(|trade| trade.entry_fee_pct + trade.exit_fee_pct)
                .sum(),
            total_pnl_pct: pattern_trades.iter().map(|trade| trade.pnl_pct).sum(),
        });
    }

    let mut step_groups: BTreeMap<(&'static str, u32), Vec<&CrossingEvent>> = BTreeMap::new();
    for event in events {
        let Some(pattern) = event.pattern_kind else {
            continue;
        };
        if !STAIR_PATTERNS.contains(&pattern) {
            continue;
        }
        step_groups
            .entry((pattern, event.pattern_steps))
            .or_default()
            .push(event);
    }
    let by_steps = step_groups
        .into_iter()
        .map(|((pattern, steps), observations)| StairStepStatistics {
            pattern,
            steps,
            observations: observations.len(),
            average_elapsed_ms: average_f64(
                observations
                    .iter()
                    .map(|event| event.pattern_elapsed_ms as f64),
            ),
            average_volume_gain: average_f64(observations.iter().map(|event| event.volume_gain)),
        })
        .collect::<Vec<_>>();

    let observations = by_pattern.iter().map(|stats| stats.observations).sum();
    let prediction_matches = by_pattern
        .iter()
        .map(|stats| stats.prediction_matches)
        .sum();
    let filled_predictions = by_pattern
        .iter()
        .map(|stats| stats.filled_predictions)
        .sum();
    let followed_predictions = by_pattern
        .iter()
        .map(|stats| stats.followed_predictions)
        .sum();
    let broken_predictions = by_pattern
        .iter()
        .map(|stats| stats.broken_predictions)
        .sum();
    let closed_positions = by_pattern.iter().map(|stats| stats.closed_positions).sum();
    let wins = by_pattern.iter().map(|stats| stats.wins).sum();
    let losses = by_pattern.iter().map(|stats| stats.losses).sum();
    StairStatistics {
        observations,
        prediction_matches,
        filled_predictions,
        followed_predictions,
        broken_predictions,
        closed_positions,
        wins,
        losses,
        win_rate_pct: wins as f64 / closed_positions.max(1) as f64 * 100.0,
        total_gross_pnl_pct: by_pattern
            .iter()
            .map(|stats| stats.total_gross_pnl_pct)
            .sum(),
        total_fees_pct: by_pattern.iter().map(|stats| stats.total_fees_pct).sum(),
        total_pnl_pct: by_pattern.iter().map(|stats| stats.total_pnl_pct).sum(),
        by_pattern,
        by_steps,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Analyzer {
    vlines: [VLineState; LINE_COUNT],
    crossing: MrcCrossingState,
    touch: mrc_touch::MrcTouchState,
    touch_learning: mrc_touch_learning::MrcTouchLearningState,
    model: MrcCrossingModel,
    cascade: cascade::CascadeState,
    cascade_signal: cascade::CascadeSignal,
    events: Vec<CrossingEvent>,
    trades: VirtualTradeSimulator,
    pattern_trades: [VirtualTradeSimulator; MRC_PATTERN_COUNT],
    selected_completed_seen: usize,
    forecast_plan_keys: [Option<(u8, u8, u8)>; LINE_COUNT - 2],
    predictions: VirtualPredictionSimulator,
    rows_seen: usize,
    forecast_observations: usize,
    forecast_entries: usize,
    forecast_blocked: usize,
    entry_candidates: usize,
    entry_setups: usize,
    selected_candidates: usize,
    selected_unique_entries: usize,
    selected_setups: usize,
    selected_market_accepted: usize,
    selected_market_busy: usize,
    selected_plan_suppressed: usize,
    forecast_plan_suppressed: usize,
    candidate_counts: [usize; MRC_PATTERN_COUNT],
    candidate_entry_counts: [usize; MRC_PATTERN_COUNT],
    rejected_lifecycle: [usize; MRC_PATTERN_COUNT],
    rejected_cascade: [usize; MRC_PATTERN_COUNT],
    rejected_barrier: [usize; MRC_PATTERN_COUNT],
    rejected_levels: [usize; MRC_PATTERN_COUNT],
    rejected_other: [usize; MRC_PATTERN_COUNT],
    touch_observations: usize,
    touch_reliable: usize,
    touch_learned_up: usize,
    touch_learned_down: usize,
    started_at: Instant,
}

impl Analyzer {
    pub(crate) fn new(direction_lock: Option<i8>, decision_components: DecisionComponents) -> Self {
        let mut analyzer = Self {
            vlines: [VLineState::default(); LINE_COUNT],
            crossing: MrcCrossingState::default(),
            touch: mrc_touch::MrcTouchState::default(),
            touch_learning: mrc_touch_learning::MrcTouchLearningState::default(),
            model: MrcCrossingModel::default(),
            cascade: cascade::CascadeState::default(),
            cascade_signal: cascade::CascadeSignal::default(),
            events: Vec::new(),
            trades: VirtualTradeSimulator::default(),
            pattern_trades: std::array::from_fn(|_| VirtualTradeSimulator::default()),
            selected_completed_seen: 0,
            forecast_plan_keys: [None; LINE_COUNT - 2],
            predictions: VirtualPredictionSimulator::default(),
            rows_seen: 0,
            forecast_observations: 0,
            forecast_entries: 0,
            forecast_blocked: 0,
            entry_candidates: 0,
            entry_setups: 0,
            selected_candidates: 0,
            selected_unique_entries: 0,
            selected_setups: 0,
            selected_market_accepted: 0,
            selected_market_busy: 0,
            selected_plan_suppressed: 0,
            forecast_plan_suppressed: 0,
            candidate_counts: [0; MRC_PATTERN_COUNT],
            candidate_entry_counts: [0; MRC_PATTERN_COUNT],
            rejected_lifecycle: [0; MRC_PATTERN_COUNT],
            rejected_cascade: [0; MRC_PATTERN_COUNT],
            rejected_barrier: [0; MRC_PATTERN_COUNT],
            rejected_levels: [0; MRC_PATTERN_COUNT],
            rejected_other: [0; MRC_PATTERN_COUNT],
            touch_observations: 0,
            touch_reliable: 0,
            touch_learned_up: 0,
            touch_learned_down: 0,
            started_at: Instant::now(),
        };
        analyzer.model.trade_direction_lock = direction_lock;
        analyzer.model.decision_components = decision_components;
        analyzer
    }

    pub(crate) fn observe(&mut self, row: &CandleRow) {
        self.rows_seen = self.rows_seen.saturating_add(1);
        if !row.mrc.ordered() || !row.close.is_finite() || row.close <= 0.0 {
            if self.rows_seen == 1 || self.rows_seen % 1_000 == 0 {
                eprintln!(
                    "[mrc-research][analysis] progress rows={} crossings={} trades={} elapsed_ms={} skipped_invalid_row=true",
                    self.rows_seen,
                    self.events.len(),
                    self.predictions.results().len(),
                    self.started_at.elapsed().as_millis()
                );
            }
            return;
        }
        self.trades.update(row);
        for simulator in &mut self.pattern_trades {
            simulator.update(row);
        }
        self.record_selected_results();
        self.predictions.update(row);
        self.cascade_signal = cascade::update_from_candle_at(
            &mut self.cascade,
            &Candle {
                open: row.open,
                high: row.high,
                low: row.low,
                close: row.close,
            },
            row.time_ms.max(0) as u64,
        );
        let lines = row.mrc.lines();
        self.touch
            .update(lines, row.close, row.time_ms.max(0) as u64);
        let crossed = vlines::update_with_events(
            &mut self.vlines,
            &mut self.crossing,
            lines,
            row.close,
            self.events.len() as u64 + 1,
            row.time_ms.max(0) as u64,
        );
        let touched_lines = crossed
            .iter()
            .map(|event| event.line_idx)
            .collect::<Vec<_>>();
        self.touch_learning.update_with_events(
            lines,
            row.close,
            row.time_ms.max(0) as u64,
            lines,
            0,
            &touched_lines,
        );
        if self.touch_learning.signal.current_line.is_some() {
            self.touch_observations = self.touch_observations.saturating_add(1);
        }
        if self.touch_learning.signal.reliable {
            self.touch_reliable = self.touch_reliable.saturating_add(1);
            match self.touch_learning.signal.learned_direction {
                1 => self.touch_learned_up = self.touch_learned_up.saturating_add(1),
                -1 => self.touch_learned_down = self.touch_learned_down.saturating_add(1),
                _ => {}
            }
        }
        let expected_event_count = self.events.len().saturating_add(crossed.len());
        let log_prediction =
            !crossed.is_empty() && (self.events.is_empty() || expected_event_count % 100 == 0);
        if log_prediction {
            eprintln!(
                "[mrc-research][analysis] prediction-start rows={} crossings_before={} new_crossings={} history_before={} time_ms={} elapsed_ms={}",
                self.rows_seen,
                self.events.len(),
                crossed.len(),
                self.model.history.len(),
                row.time_ms,
                self.started_at.elapsed().as_millis()
            );
        }
        let prediction_started = Instant::now();
        let prediction_context = mrc_prediction::MrcPredictionContext {
            cascade_enabled: true,
            ntps: row.ntps,
            ntps_fast_time_ms: None,
            cascade_direction: self.cascade_signal.dir,
            cascade_speed_pct_per_min: self.cascade_signal.speed_pct_per_min,
            cascade_strength: self.cascade_signal.strength,
            cascade_stretched: self.cascade_signal.stretched,
            cascade_phase: self.cascade_signal.phase,
            cascade_potential_move_pct: self.cascade_signal.potential_move_pct,
            cascade_adverse_move_pct: self.cascade_signal.adverse_move_pct,
        };
        let observation = mrc_crossing::observe_with_forecast_and_touch_and_learning_context(
            &mut self.model,
            &crossed,
            lines,
            &self.crossing,
            (row.gain, 0.0, row.gain, row.buy_sell_gain),
            row.close,
            row.time_ms.max(0) as u64,
            prediction_context,
            self.touch.signal,
            self.touch_learning.signal,
        );
        if log_prediction {
            eprintln!(
                "[mrc-research][analysis] prediction-complete rows={} crossings_after={} history_after={} entries={} prediction_ms={} total_elapsed_ms={}",
                self.rows_seen,
                self.events.len().saturating_add(observation.events.len()),
                self.model.history.len(),
                observation.entry.is_some(),
                prediction_started.elapsed().as_millis(),
                self.started_at.elapsed().as_millis()
            );
        }
        let crossed_lines = crossed
            .iter()
            .map(|event| event.line_idx as usize)
            .collect::<Vec<_>>();
        self.predictions.observe_crossings(&crossed_lines);
        if observation.forecast.is_some() {
            self.forecast_observations = self.forecast_observations.saturating_add(1);
            if observation.entry.is_some() {
                self.forecast_entries = self.forecast_entries.saturating_add(1);
            } else {
                self.forecast_blocked = self.forecast_blocked.saturating_add(1);
            }
        }
        let actual_candidates = !crossed.is_empty();
        let mut selected_setup = None;
        let selected_candidate = observation.selected_candidate().cloned();
        if selected_candidate.is_some() {
            self.selected_candidates = self.selected_candidates.saturating_add(1);
        }
        if observation.entry.is_some() {
            self.selected_unique_entries = self.selected_unique_entries.saturating_add(1);
        } else if selected_candidate.is_some() {
            self.selected_plan_suppressed = self.selected_plan_suppressed.saturating_add(1);
        }
        for candidate in &observation.candidates {
            let candidate_slot = candidate.pattern_kind.index();
            if candidate_slot < self.candidate_counts.len() {
                self.candidate_counts[candidate_slot] =
                    self.candidate_counts[candidate_slot].saturating_add(1);
                if candidate.entry.is_some() {
                    self.candidate_entry_counts[candidate_slot] =
                        self.candidate_entry_counts[candidate_slot].saturating_add(1);
                } else if matches!(
                    candidate.analysis.staircase_lifecycle,
                    Some(StaircaseLifecycle::Invalidated | StaircaseLifecycle::Stalled)
                ) {
                    self.rejected_lifecycle[candidate_slot] =
                        self.rejected_lifecycle[candidate_slot].saturating_add(1);
                } else if !candidate.analysis.cascade_trade_state.allows_entry() {
                    self.rejected_cascade[candidate_slot] =
                        self.rejected_cascade[candidate_slot].saturating_add(1);
                } else if candidate.analysis.barrier_state
                    == mrc_cross_analysis::BarrierState::Approaching
                {
                    self.rejected_barrier[candidate_slot] =
                        self.rejected_barrier[candidate_slot].saturating_add(1);
                } else {
                    self.rejected_other[candidate_slot] =
                        self.rejected_other[candidate_slot].saturating_add(1);
                }
            }
            let should_queue = if candidate.forecast {
                let key = candidate_key(candidate);
                let slot = candidate.pattern_kind.index();
                let fresh = key.is_some() && self.forecast_plan_keys[slot] != key;
                self.forecast_plan_keys[slot] = if candidate.entry.is_some() { key } else { None };
                if !fresh && candidate.entry.is_some() {
                    self.forecast_plan_suppressed = self.forecast_plan_suppressed.saturating_add(1);
                }
                fresh
            } else {
                actual_candidates
            };
            if !should_queue {
                continue;
            }
            let crossing_event = crossing_event_from_analysis(
                &candidate.analysis,
                &self.crossing,
                self.events.last().map(|previous| previous.time_ms),
                self.vlines[candidate.analysis.line_idx as usize].cross_count,
            );
            let Some(setup) = self.setup_from_candidate(row, &crossing_event, candidate) else {
                continue;
            };
            self.entry_setups = self.entry_setups.saturating_add(1);
            if candidate.entry.is_some() {
                self.entry_candidates = self.entry_candidates.saturating_add(1);
            }
            self.pattern_trades[candidate.pattern_kind.index()].queue_next_candle(setup.clone());
            self.predictions.queue(setup.clone());
            if selected_candidate.as_ref() == Some(candidate) {
                self.selected_setups = self.selected_setups.saturating_add(1);
                selected_setup = self.market_setup_from_candidate(row, &crossing_event, candidate);
            }
        }
        if let Some(setup) = selected_setup {
            if self.trades.open_market(setup, row.close) {
                self.selected_market_accepted = self.selected_market_accepted.saturating_add(1);
            } else {
                self.selected_market_busy = self.selected_market_busy.saturating_add(1);
            }
        }
        for raw_event in &crossed {
            let event = observation
                .events
                .iter()
                .find(|event| {
                    event.line_idx == raw_event.line_idx && event.now_ms == raw_event.now_ms
                })
                .cloned()
                .unwrap_or_else(|| {
                    mrc_cross_analysis::build_with_context(
                        raw_event,
                        None,
                        lines,
                        &self.crossing,
                        (row.gain, 0.0, row.gain, row.buy_sell_gain),
                        prediction_context,
                    )
                });
            let crossing_event = crossing_event_from_analysis(
                &event,
                &self.crossing,
                self.events.last().map(|previous| previous.time_ms),
                self.vlines[event.line_idx as usize].cross_count,
            );
            self.events.push(crossing_event);
        }
        if self.rows_seen == 1 || self.rows_seen % 1_000 == 0 {
            eprintln!(
                "[mrc-research][analysis] progress rows={} crossings={} trades={} elapsed_ms={}",
                self.rows_seen,
                self.events.len(),
                self.predictions.results().len(),
                self.started_at.elapsed().as_millis()
            );
        }
    }

    pub(crate) fn finish(&mut self, row: &CandleRow) {
        self.trades.finish(row);
        for simulator in &mut self.pattern_trades {
            simulator.finish(row);
        }
        self.record_selected_results();
        self.predictions.finish(row);
        eprintln!(
                "[mrc-research][analysis] complete rows={} crossings={} positions={} predictions={} forecasts={} entries={} blocked={} open_trade={} elapsed_ms={}",
                self.rows_seen,
                self.events.len(),
                self.trades.completed_count(),
                self.predictions.results().len(),
                self.forecast_observations,
                self.forecast_entries,
                self.forecast_blocked,
                self.trades.active().is_some(),
            self.started_at.elapsed().as_millis()
        );
        eprintln!(
            "[mrc-research][entry-gen] candidates={} setups={} selected={} unique_entries={} selected_setups={} market_accepted={} market_busy={} selected_plan_suppressed={} forecast_plan_suppressed={}",
            self.entry_candidates,
            self.entry_setups,
            self.selected_candidates,
            self.selected_unique_entries,
            self.selected_setups,
            self.selected_market_accepted,
            self.selected_market_busy,
            self.selected_plan_suppressed,
            self.forecast_plan_suppressed,
        );
        eprintln!(
            "[mrc-research][touch-learning] observations={} reliable={} learned_up={} learned_down={}",
            self.touch_observations,
            self.touch_reliable,
            self.touch_learned_up,
            self.touch_learned_down,
        );
        for pattern in MrcPatternKind::ALL {
            let slot = pattern.index();
            eprintln!(
                "[mrc-research][entry-gen] pattern={} candidates={} valid={} lifecycle={} cascade={} barrier={} other={}",
                pattern.as_str(),
                self.candidate_counts[slot],
                self.candidate_entry_counts[slot],
                self.rejected_lifecycle[slot],
                self.rejected_cascade[slot],
                self.rejected_barrier[slot],
                self.rejected_other[slot],
            );
        }
    }

    fn setup_from_candidate(
        &self,
        row: &CandleRow,
        event: &CrossingEvent,
        candidate: &mrc_crossing::PatternCandidate,
    ) -> Option<TradeSetup> {
        // A prediction can have directionally valid levels while still being
        // rejected by the live analysis gate (cascade, barrier, or lifecycle).
        // It must not be counted as an entry prediction unless the same gate
        // already authorized the candidate.
        candidate.entry.as_ref()?;
        let prediction = &candidate.analysis;
        let target_line = prediction.trade_target_line? as usize;
        let target_price = prediction.target_price?;
        let stop_price = prediction.stop_price?;
        let entry_price = prediction.effective_entry_price?;
        if !mrc_crossing::valid_price_levels_for_pattern(
            prediction.trade_direction,
            entry_price,
            target_price,
            stop_price,
            None,
            prediction.pattern_kind,
        ) {
            return None;
        }
        let reason = format!(
            "mrc-prediction pattern={} pattern_steps={} pattern_elapsed_ms={} stair_sequence={} stair_price_move_pct={:.4} stair_favorable_move_pct={:.4} stair_adverse_move_pct={:.4} stair_lifecycle={} stair_transition={} stair_protected_line={} stair_extreme_line={} cascade_enabled={} cascade_direction={} cascade_speed_pct_per_min={} cascade_phase={} cascade_potential_move_pct={} cascade_adverse_move_pct={} cascade_trade_state={} source_line={} learned_line={}@{:.6} predicted_direction={} trade_direction={} barrier={}:{} resistance_line={}@{:.6} tp_line={} tp={:.6} sl_line={} sl={:.6} entry={:.6} volume_gain={:.4} volume_gain_min={:.4} volume_gain_max={:.4} buy_sell_gain={:.4} history_scope={} context_samples={} no_pullback={} reverted_line={} transition_samples={} movement_samples={} next_line_crosses={} support_resistance={} price_only_moves={} movement_wins={} movement_losses={} movement_success_rate={:.4} expected_move_pct={:.4} favorable_move_pct={:.4} adverse_move_pct={:.4} eta_ms={} market_price={:.6} mrc_range={:.6}..{:.6} expected_price={:.6}..{:.6} projection={}",
            event.pattern_key,
            event.pattern_steps,
            duration_token(fmt_duration_ms(event.pattern_elapsed_ms)),
            format_line_sequence(&event.stair_sequence),
            event.stair_price_move_pct,
            event.stair_favorable_move_pct,
            event.stair_adverse_move_pct,
            prediction
                .staircase_lifecycle
                .map(|value| value.as_str())
                .unwrap_or("na"),
            prediction
                .staircase_transition
                .map(|value| value.as_str())
                .unwrap_or("na"),
            prediction
                .staircase_protected_line
                .map(|line| LABELS[line as usize])
                .unwrap_or("na"),
            prediction
                .staircase_extreme_line
                .map(|line| LABELS[line as usize])
                .unwrap_or("na"),
            prediction.cascade_enabled,
            prediction.cascade_direction,
            prediction
                .cascade_speed_pct_per_min
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| "na".to_string()),
            prediction.cascade_phase.as_str(),
            prediction
                .cascade_potential_move_pct
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| "na".to_string()),
            prediction
                .cascade_adverse_move_pct
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| "na".to_string()),
            prediction.cascade_trade_state.as_str(),
            event.line_name,
            event.target_name.unwrap_or("na"),
            prediction.predicted_line_price.unwrap_or(0.0),
            direction_name(prediction.predicted_direction),
            direction_name(prediction.trade_direction),
            prediction.barrier_state.as_str(),
            prediction
                .barrier_line
                .map(|line| LABELS[line as usize])
                .unwrap_or("na"),
            prediction
                .barrier_line
                .map(|line| LABELS[line as usize])
                .unwrap_or("na"),
            prediction.barrier_price.unwrap_or(0.0),
            prediction
                .take_profit_line
                .map(|line| LABELS[line as usize])
                .unwrap_or("na"),
            target_price,
            prediction
                .stop_loss_line
                .map(|line| LABELS[line as usize])
                .unwrap_or("na"),
            stop_price,
            entry_price,
            prediction.volume_gain,
            prediction.volume_gain_min,
            prediction.volume_gain_max,
            prediction.buy_sell_gain,
            event.history_scope,
            prediction.context_samples,
            prediction.no_pullback_count,
            prediction
                .reverted_line
                .map(|line| LABELS[line as usize])
                .unwrap_or("na"),
            prediction.pattern_samples,
            prediction.movement_samples,
            prediction.next_line_crosses,
            prediction.support_resistance_moves,
            prediction.price_only_moves,
            prediction.successful_moves,
            prediction.failed_moves,
            prediction.movement_success_rate.unwrap_or(0.0),
            prediction.expected_value_pct.unwrap_or(0.0),
            prediction.favorable_move_pct.unwrap_or(0.0),
            prediction.adverse_move_pct.unwrap_or(0.0),
            duration_token(fmt_duration_f64(prediction.estimated_time_ms.unwrap_or(0.0))),
            prediction.market_price,
            prediction.mrc_min,
            prediction.mrc_max,
            prediction.expected_price_low.unwrap_or(prediction.market_price),
            prediction.expected_price_high.unwrap_or(prediction.market_price),
            mrc_crossing::price_projection(
                prediction.trade_direction,
                prediction.mrc_min,
                prediction.mrc_max,
                prediction.expected_price_low.unwrap_or(prediction.market_price),
                prediction.expected_price_high.unwrap_or(prediction.market_price),
            ),
        );
        Some(
            TradeSetup::with_levels(
                prediction.trade_direction,
                event.line,
                event.line_name,
                target_line,
                LABELS[target_line],
                entry_price,
                target_price,
                stop_price,
                row.time_ms,
                reason,
            )
            .with_execution(prediction.trade_direction, "limit", "market")
            .with_pattern_key(event.pattern_key.clone()),
        )
    }

    fn market_setup_from_candidate(
        &self,
        row: &CandleRow,
        event: &CrossingEvent,
        candidate: &mrc_crossing::PatternCandidate,
    ) -> Option<TradeSetup> {
        let mut setup = self.setup_from_candidate(row, event, candidate)?;
        let reason = candidate.entry.as_ref()?;
        let (target_price, stop_price) =
            mrc_crossing::exit_levels(reason, row.mrc.lines(), row.close)?;
        setup.entry_price = row.close;
        setup.target_price = target_price;
        setup.stop_price = stop_price;
        setup.reason = format!("{} market_fill={:.6}", setup.reason, row.close);
        Some(setup)
    }

    fn record_selected_results(&mut self) {
        let completed = self.trades.completed();
        for result in completed.iter().skip(self.selected_completed_seen) {
            if let Some(pattern) = MrcPatternKind::from_str(&result.pattern_key) {
                mrc_crossing::record_pattern_result(&mut self.model, pattern, result.pnl_pct);
            }
            // The selector key prevents repeated offers while this virtual
            // position is active. A closed position may seek a new route in
            // the reserved direction, just like the live engine.
            self.model.last_plan_key = None;
            self.forecast_plan_keys.fill(None);
        }
        self.selected_completed_seen = completed.len();
    }

    fn attach_position_result(events: &mut [CrossingEvent], trade: &VirtualTradeResult) {
        let Some(event) = events.iter_mut().rev().find(|event| {
            event.time_ms == trade.signal_time_ms
                && event.line == trade.source_line
                && event.trade_direction == trade.side
                && event.pattern_key == trade.pattern_key
        }) else {
            return;
        };
        event.position_outcome = Some(if trade.pnl_pct > 0.0 {
            "profitable"
        } else {
            "negative"
        });
        event.position_pnl_pct = Some(trade.pnl_pct);
        event.position_mfe_pct = Some(trade.mfe_pct);
        event.position_mae_pct = Some(trade.mae_pct);
        event.position_exit_reason = Some(trade.exit_reason.clone());
    }

    fn attach_prediction_result(
        events: &mut [CrossingEvent],
        prediction: &VirtualPredictionResult,
    ) {
        let Some(event) = events.iter_mut().rev().find(|event| {
            event.time_ms == prediction.created_at
                && event.line == prediction.source_line
                && event.pattern_key == prediction.pattern_key
        }) else {
            return;
        };
        event.prediction_side = Some(prediction.side);
        event.prediction_filled = Some(prediction.filled);
        event.prediction_outcome = Some(prediction.outcome);
        event.prediction_result_move_pct = prediction_result_move_pct(prediction);
        event.prediction_resolved_at = Some(prediction.resolved_at);
    }

    pub(crate) fn report(&self) -> AnalysisReport {
        let trades = self.trades.completed();
        let mut events = self.events.clone();
        for trade in &trades {
            Self::attach_position_result(&mut events, trade);
        }
        let predictions = self.predictions.results();
        for prediction in &predictions {
            Self::attach_prediction_result(&mut events, prediction);
        }
        let latest_event = events.last().cloned();
        let forecast = forecast(latest_event.clone(), &self.model);
        let patterns = forecast.likely_destinations.clone();
        let stair_statistics = stair_statistics(&events, &predictions, &trades);
        let pattern_win_rates = pattern_win_rate_reports(&trades, &predictions);
        AnalysisReport {
            crossing_count: events.len(),
            pattern_count: patterns.len(),
            patterns,
            latest_event,
            forecast,
            events,
            trade_summary: self.trades.summary(),
            trades,
            open_trade: self.trades.active(),
            prediction_summary: self.predictions.summary(),
            predictions,
            pending_predictions: self.predictions.pending_count(),
            active_predictions: self.predictions.active_count(),
            pattern_win_rates,
            stair_statistics,
            pattern_performance: MrcPatternKind::ALL
                .into_iter()
                .map(|pattern| {
                    let performance = self.model.pattern_performance[pattern.index()];
                    PatternPerformanceReport {
                        pattern: pattern.as_str(),
                        observations: performance.observations,
                        trades: performance.trades,
                        wins: performance.wins,
                        win_rate_pct: performance.win_rate() * 100.0,
                        total_pnl_pct: performance.pnl_sum_pct,
                        average_pnl_pct: performance.average_pnl_pct(),
                        ema_pnl_pct: performance.ema_pnl_pct,
                        last_pnl_pct: performance.last_pnl_pct,
                    }
                })
                .collect(),
            parallel_pattern_trades: MrcPatternKind::ALL
                .into_iter()
                .map(|pattern| PatternTradeReport {
                    pattern: pattern.as_str(),
                    summary: self.pattern_trades[pattern.index()].summary(),
                    trades: self.pattern_trades[pattern.index()].completed(),
                    open_trade: self.pattern_trades[pattern.index()].active(),
                })
                .collect(),
            active_pattern: self.model.active_pattern.and_then(|index| {
                MrcPatternKind::ALL
                    .get(index as usize)
                    .map(|pattern| pattern.as_str())
            }),
            active_pattern_score: self.model.active_pattern_score,
        }
    }
}

fn candidate_key(candidate: &mrc_crossing::PatternCandidate) -> Option<(u8, u8, u8)> {
    Some((
        candidate.pattern_kind.index() as u8,
        candidate.analysis.line_idx,
        candidate.analysis.trade_target_line?,
    ))
}

fn pattern_report(
    pattern_kind: &'static str,
    pattern: &[u8],
    to: usize,
    stats: long_scalping::state::MrcForecastStats,
    next_line_crosses: u32,
    support_resistance_moves: u32,
    price_only_moves: u32,
) -> PatternReport {
    PatternReport {
        pattern_kind,
        pattern: pattern.iter().map(|line| *line as usize).collect(),
        pattern_names: pattern.iter().map(|line| LABELS[*line as usize]).collect(),
        to,
        to_name: LABELS[to],
        count: stats.count as usize,
        movement_samples: stats.movement_samples,
        next_line_crosses,
        support_resistance_moves,
        price_only_moves,
        successful_moves: stats.success_count,
        failed_moves: stats.failure_count,
        movement_success_rate: stats.movement_success_rate(),
        expected_value_pct: stats.expected_value_pct(),
        average_duration_ms: stats.average_duration_ms(),
    }
}

fn forecast(latest: Option<CrossingEvent>, model: &MrcCrossingModel) -> Forecast {
    let Some(latest) = latest else {
        return Forecast {
            source_line: None,
            source_name: None,
            condition: "no-crossing-history",
            sample_count: 0,
            estimated_time_ms: None,
            likely_destinations: Vec::new(),
        };
    };
    let predictions = mrc_crossing::predict_all(model);
    if predictions.is_empty() {
        return Forecast {
            source_line: Some(latest.line),
            source_name: Some(latest.line_name),
            condition: "local-transition-not-observed",
            sample_count: mrc_crossing::warmup_samples(model) as usize,
            estimated_time_ms: None,
            likely_destinations: Vec::new(),
        };
    }
    let sample_count = predictions
        .iter()
        .map(|prediction| prediction.samples)
        .max()
        .unwrap_or_default();
    let estimated_time_ms = predictions
        .iter()
        .filter_map(|prediction| prediction.stats.average_duration_ms())
        .min_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let likely_destinations = predictions
        .iter()
        .map(|prediction| {
            pattern_report(
                prediction.pattern_kind.as_str(),
                &prediction.pattern,
                prediction.target_line as usize,
                prediction.stats,
                prediction.next_line_crosses,
                prediction.support_resistance_moves,
                prediction.price_only_moves,
            )
        })
        .collect();
    Forecast {
        source_line: Some(latest.line),
        source_name: Some(latest.line_name),
        condition: "classified-parallel-pattern-predictions",
        sample_count: sample_count as usize,
        estimated_time_ms,
        likely_destinations,
    }
}
