use std::collections::BTreeMap;
use std::time::Instant;

use long_scalping::detection::{cascade, mrc_cross_analysis, mrc_crossing, vlines};
use long_scalping::model::Candle;
use long_scalping::state::{CascadeState, MrcCrossingModel, MrcCrossingState, VLineState};
use serde::Serialize;

use super::data::CandleRow;
use super::virtual_trade::{
    TradeSetup, VirtualPosition, VirtualPredictionResult, VirtualPredictionSimulator,
    VirtualPredictionSummary, VirtualTradeResult, VirtualTradeSimulator, VirtualTradeSummary,
};

const LINE_COUNT: usize = 9;
const LABELS: [&str; LINE_COUNT] = mrc_crossing::LINE_LABELS;

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
    let movement = realized_favorable_move_pct
        .zip(realized_adverse_move_pct)
        .or_else(|| active_move_percentages(crossing))
        .map(|(favorable, adverse)| {
            format!(
                "{}:+{:.2}%:-{:.2}%",
                if direction > 0 { "long" } else { "short" },
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
    pub(crate) barrier_line: Option<usize>,
    pub(crate) barrier_line_name: Option<&'static str>,
    pub(crate) barrier_price: Option<f64>,
    pub(crate) barrier_state: String,
    pub(crate) history_scope: &'static str,
    pub(crate) pattern_kind: Option<&'static str>,
    pub(crate) pattern_steps: u32,
    pub(crate) pattern_elapsed_ms: u64,
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
    pub(crate) cascade_dir: i8,
    pub(crate) cascade_strength: f64,
    pub(crate) cascade_movement_position: f64,
    pub(crate) cascade_stretched: bool,
    pub(crate) cascade_move_pct: Option<f64>,
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
    pub(crate) probability: Option<f64>,
    pub(crate) movement_success_rate: Option<f64>,
    pub(crate) expected_value_pct: Option<f64>,
    pub(crate) expected_price_low: Option<f64>,
    pub(crate) expected_price_high: Option<f64>,
    pub(crate) realized_duration_ms: Option<u64>,
    pub(crate) realized_favorable_move_pct: Option<f64>,
    pub(crate) realized_adverse_move_pct: Option<f64>,
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

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PatternWinRateReport {
    pub(crate) pattern: String,
    pub(crate) matches: usize,
    pub(crate) filled_predictions: usize,
    pub(crate) closed_positions: usize,
    pub(crate) wins: usize,
    pub(crate) losses: usize,
    pub(crate) win_rate_pct: f64,
    pub(crate) average_probability_pct: f64,
    pub(crate) minimum_probability_pct: f64,
    pub(crate) maximum_probability_pct: f64,
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
    pub(crate) probability: f64,
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
    pub(crate) probability_next_crossing: Option<f64>,
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
            let probabilities = predictions
                .iter()
                .filter_map(|prediction| prediction.prediction_probability)
                .collect::<Vec<_>>();
            let average_probability_pct = if probabilities.is_empty() {
                0.0
            } else {
                probabilities.iter().sum::<f64>() / probabilities.len() as f64 * 100.0
            };
            let minimum_probability_pct = probabilities
                .iter()
                .copied()
                .reduce(f64::min)
                .unwrap_or(0.0)
                * 100.0;
            let maximum_probability_pct = probabilities
                .iter()
                .copied()
                .reduce(f64::max)
                .unwrap_or(0.0)
                * 100.0;
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
                average_probability_pct,
                minimum_probability_pct,
                maximum_probability_pct,
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

#[derive(Debug, Clone)]
pub(crate) struct Analyzer {
    vlines: [VLineState; LINE_COUNT],
    crossing: MrcCrossingState,
    cascade: CascadeState,
    model: MrcCrossingModel,
    events: Vec<CrossingEvent>,
    trades: VirtualTradeSimulator,
    predictions: VirtualPredictionSimulator,
    rows_seen: usize,
    started_at: Instant,
}

impl Analyzer {
    pub(crate) fn new() -> Self {
        Self {
            vlines: [VLineState::default(); LINE_COUNT],
            crossing: MrcCrossingState::default(),
            cascade: CascadeState::default(),
            model: MrcCrossingModel::default(),
            events: Vec::new(),
            trades: VirtualTradeSimulator::default(),
            predictions: VirtualPredictionSimulator::default(),
            rows_seen: 0,
            started_at: Instant::now(),
        }
    }

    pub(crate) fn observe(&mut self, row: &CandleRow, _hourly_ntps: &[i64]) {
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
        self.predictions.update(row);
        let candle = Candle {
            open: row.open,
            high: row.high,
            low: row.low,
            close: row.close,
        };
        let cascade_signal = cascade::update_from_candle(&mut self.cascade, &candle);
        let lines = row.mrc.lines();
        let crossed = vlines::update_with_events(
            &mut self.vlines,
            &mut self.crossing,
            lines,
            row.close,
            self.events.len() as u64 + 1,
            row.time_ms.max(0) as u64,
        );
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
        let observation = mrc_crossing::observe_with_context(
            &mut self.model,
            &crossed,
            lines,
            &self.crossing,
            cascade_signal,
            (row.gain, 0.0, row.gain, row.buy_sell_gain),
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
        for event in observation.events {
            let previous_time_ms = self.events.last().map(|previous| previous.time_ms);
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
                cascade_dir: event.cascade_dir,
                cascade_strength: event.cascade_strength,
                cascade_movement_position: event.cascade_movement_position,
                cascade_stretched: event.cascade_stretched,
                cascade_move_pct: event.cascade_move_pct,
                volume_gain: event.volume_gain,
                volume_gain_min: event.volume_gain_min,
                volume_gain_max: event.volume_gain_max,
                buy_sell_gain: event.buy_sell_gain,
                mrc_trace: format_mrc_trace(
                    &self.crossing,
                    event.line_idx,
                    event.direction,
                    event.realized_favorable_move_pct,
                    event.realized_adverse_move_pct,
                    event.realized_duration_ms.is_some(),
                    self.vlines[event.line_idx as usize].cross_count,
                ),
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
                probability: event.probability,
                movement_success_rate: event.movement_success_rate,
                expected_value_pct: event.expected_value_pct,
                expected_price_low: event.expected_price_low,
                expected_price_high: event.expected_price_high,
                realized_duration_ms: event.realized_duration_ms,
                realized_favorable_move_pct: event.realized_favorable_move_pct,
                realized_adverse_move_pct: event.realized_adverse_move_pct,
                realized_price_high: event.realized_price_high,
                realized_price_low: event.realized_price_low,
                realized_target_low: event.realized_target_low,
                realized_target_high: event.realized_target_high,
                realized_mrc_low: event.realized_mrc_low,
                realized_mrc_high: event.realized_mrc_high,
                pattern_key: String::new(),
            };
            crossing_event.pattern_key = format_pattern_key(&crossing_event);
            if event.tradeable() {
                if let (
                    Some(target_line),
                    Some(target_price),
                    Some(stop_price),
                    Some(entry_price),
                ) = (
                    event.trade_target_line,
                    event.target_price,
                    event.stop_price,
                    event.effective_entry_price,
                ) {
                    self.open_prediction(
                        row,
                        &crossing_event,
                        target_line as usize,
                        target_price,
                        stop_price,
                        entry_price,
                        &event,
                    );
                }
            }
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
        self.predictions.finish(row);
        eprintln!(
            "[mrc-research][analysis] complete rows={} crossings={} positions={} predictions={} open_trade={} elapsed_ms={}",
            self.rows_seen,
            self.events.len(),
            self.trades.completed_count(),
            self.predictions.results().len(),
            self.trades.active().is_some(),
            self.started_at.elapsed().as_millis()
        );
    }

    fn open_prediction(
        &mut self,
        row: &CandleRow,
        event: &CrossingEvent,
        target_line: usize,
        target_price: f64,
        stop_price: f64,
        entry_price: f64,
        prediction: &mrc_cross_analysis::MrcCrossAnalysis,
    ) {
        if !mrc_crossing::valid_price_levels(
            prediction.trade_direction,
            entry_price,
            target_price,
            stop_price,
            None,
        ) {
            return;
        }
        let reason = format!(
            "mrc-prediction pattern={} pattern_steps={} pattern_elapsed_ms={} source_line={} learned_line={}@{:.6} predicted_direction={} trade_direction={} barrier={}:{} resistance_line={}@{:.6} tp_line={} tp={:.6} sl_line={} sl={:.6} entry={:.6} history_scope={} context_samples={} no_pullback={} reverted_line={} probability={}/{}={:.4} transition_samples={} movement_samples={} next_line_crosses={} support_resistance={} price_only_moves={} movement_wins={} movement_losses={} movement_success_rate={:.4} expected_move_pct={:.4} favorable_move_pct={:.4} adverse_move_pct={:.4} eta_ms={:.0} market_price={:.6} mrc_range={:.6}..{:.6} expected_price={:.6}..{:.6} projection={}",
            event.pattern_key,
            event.pattern_steps,
            event.pattern_elapsed_ms,
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
            event.history_scope,
            prediction.context_samples,
            prediction.no_pullback_count,
            prediction
                .reverted_line
                .map(|line| LABELS[line as usize])
                .unwrap_or("na"),
            prediction.target_samples,
            prediction.pattern_samples,
            prediction.probability.unwrap_or(0.0),
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
            prediction.estimated_time_ms.unwrap_or(0.0),
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
        let setup = TradeSetup::with_levels(
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
        .with_execution(prediction.trade_direction, "market", "limit")
        .with_pattern_evidence(event.pattern_key.clone(), prediction.probability);
        self.trades.queue_next_candle(setup.clone());
        self.predictions.queue(setup);
    }

    pub(crate) fn report(&self) -> AnalysisReport {
        let latest_event = self.events.last().cloned();
        let forecast = forecast(latest_event.clone(), &self.model);
        let patterns = forecast.likely_destinations.clone();
        AnalysisReport {
            crossing_count: self.events.len(),
            pattern_count: patterns.len(),
            patterns,
            latest_event,
            forecast,
            events: self.events.clone(),
            trade_summary: self.trades.summary(),
            trades: self.trades.completed(),
            open_trade: self.trades.active(),
            prediction_summary: self.predictions.summary(),
            predictions: self.predictions.results(),
            pending_predictions: self.predictions.pending_count(),
            active_predictions: self.predictions.active_count(),
            pattern_win_rates: pattern_win_rate_reports(
                &self.trades.completed(),
                &self.predictions.results(),
            ),
        }
    }
}

fn pattern_report(
    pattern_kind: &'static str,
    pattern: &[u8],
    to: usize,
    stats: long_scalping::state::MrcForecastStats,
    total: usize,
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
        probability: stats.count as f64 / total.max(1) as f64,
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
            probability_next_crossing: None,
            estimated_time_ms: None,
            likely_destinations: Vec::new(),
        };
    };
    let Some(prediction) = mrc_crossing::predict(model, latest.direction) else {
        return Forecast {
            source_line: Some(latest.line),
            source_name: Some(latest.line_name),
            condition: "local-transition-not-observed",
            sample_count: mrc_crossing::warmup_samples(model) as usize,
            probability_next_crossing: None,
            estimated_time_ms: None,
            likely_destinations: Vec::new(),
        };
    };
    let destination = pattern_report(
        prediction.pattern_kind.as_str(),
        &prediction.pattern,
        prediction.target_line as usize,
        prediction.stats,
        prediction.samples as usize,
        prediction.next_line_crosses,
        prediction.support_resistance_moves,
        prediction.price_only_moves,
    );
    Forecast {
        source_line: Some(latest.line),
        source_name: Some(latest.line_name),
        condition: "classified-local-pattern-prediction",
        sample_count: prediction.samples as usize,
        probability_next_crossing: Some(prediction.probability),
        estimated_time_ms: prediction.stats.average_duration_ms(),
        likely_destinations: vec![destination],
    }
}
