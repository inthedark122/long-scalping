use long_scalping::detection::cascade::{self, CascadeSignal, CascadeState};
use long_scalping::detection::movement_window::{self, MovementWindow};
use long_scalping::model::{Candle, MrcBands};
use serde::Serialize;

use super::data::{CandleRow, MRC_PERIOD};

const NEIGHBOR_COUNT: usize = 8;
const MIN_TRAINING_SAMPLES: usize = 12;
pub(crate) const MRC_WARMUP_BARS: usize = 400;
const FEATURE_WEIGHTS: [f64; 12] = [
    2.0, 1.5, 1.5, 0.75, 0.75, 0.75, 1.0, 0.75, 0.75, 0.5, 0.75, 0.5,
];

#[derive(Debug, Clone, Copy, Default)]
struct FeatureVector([f64; 12]);

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WindowCandle {
    pub(crate) time_ms: i64,
    pub(crate) open: f64,
    pub(crate) high: f64,
    pub(crate) low: f64,
    pub(crate) close: f64,
    pub(crate) mrc: MrcBands,
    pub(crate) cascade_direction: i8,
    pub(crate) cascade_phase: &'static str,
    pub(crate) cascade_strength: f64,
    pub(crate) window_low: Option<f64>,
    pub(crate) window_high: Option<f64>,
    pub(crate) window_width_pct: Option<f64>,
    pub(crate) window_direction_changes: Option<usize>,
    pub(crate) window_price_location: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Forecast {
    pub(crate) time_ms: i64,
    pub(crate) target_time_ms: i64,
    pub(crate) duration_bars: usize,
    pub(crate) duration_minutes: usize,
    pub(crate) close: f64,
    pub(crate) predicted_return_pct: f64,
    pub(crate) predicted_price: f64,
    pub(crate) lower_price: f64,
    pub(crate) upper_price: f64,
    pub(crate) direction_confidence_pct: f64,
    pub(crate) training_samples: usize,
    pub(crate) analog_samples: usize,
    pub(crate) realized_price: Option<f64>,
    #[serde(skip)]
    analogs: Vec<WeightedAnalog>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProjectedCandle {
    pub(crate) step: usize,
    pub(crate) time_ms: i64,
    pub(crate) open: f64,
    pub(crate) high: f64,
    pub(crate) low: f64,
    pub(crate) close: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ValidationSummary {
    pub(crate) forecasts: usize,
    pub(crate) resolved_forecasts: usize,
    pub(crate) direction_accuracy_pct: Option<f64>,
    pub(crate) mean_absolute_error_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct LearningSettings {
    pub(crate) method: &'static str,
    pub(crate) target_timing: &'static str,
    pub(crate) neighbors: usize,
    pub(crate) minimum_training_samples: usize,
    pub(crate) training_updates: &'static str,
    pub(crate) leakage_control: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ResearchReport {
    pub(crate) schema_version: u32,
    pub(crate) symbol_key: String,
    pub(crate) timeframe_seconds: u64,
    pub(crate) history_requested: usize,
    pub(crate) history_loaded: usize,
    pub(crate) training_history_loaded: usize,
    pub(crate) mrc_period: usize,
    pub(crate) mrc_warmup_bars: usize,
    pub(crate) analysis_offset_minutes: usize,
    pub(crate) analysis_index: Option<usize>,
    pub(crate) analysis_time_ms: Option<i64>,
    pub(crate) realized_future_bars: usize,
    pub(crate) source_start_ms: Option<i64>,
    pub(crate) source_end_ms: Option<i64>,
    pub(crate) learning: LearningSettings,
    pub(crate) validation: ValidationSummary,
    pub(crate) latest_forecast: Option<Forecast>,
    pub(crate) latest_projection: Vec<ProjectedCandle>,
    pub(crate) candles: Vec<WindowCandle>,
    pub(crate) forecasts: Vec<Option<Forecast>>,
}

#[derive(Clone, Copy)]
struct TrainingSample {
    sample_index: usize,
    features: FeatureVector,
    forward_return_pct: f64,
}

#[derive(Clone, Copy)]
struct Neighbor {
    sample_index: usize,
    distance: f64,
    forward_return_pct: f64,
}

#[derive(Debug, Clone, Copy)]
struct WeightedAnalog {
    sample_index: usize,
    weight: f64,
}

fn clamp(value: f64, low: f64, high: f64) -> f64 {
    if value.is_finite() {
        value.clamp(low, high)
    } else {
        0.0
    }
}

fn channel_width_pct(mrc: &MrcBands, close: f64) -> f64 {
    if close <= 0.0 || !close.is_finite() {
        return 0.001;
    }
    (((mrc.up_big - mrc.down_big) / close * 100.0).abs()).max(0.001)
}

fn make_feature(
    index: usize,
    rows: &[CandleRow],
    signal: CascadeSignal,
    window: Option<MovementWindow>,
) -> FeatureVector {
    let row = &rows[index];
    let width = (row.mrc.up_big - row.mrc.down_big).abs().max(f64::EPSILON);
    let width_pct = channel_width_pct(&row.mrc, row.close);
    let mrc_position = clamp((row.close - row.mrc.mean_line) / width, -1.0, 1.0);
    let mean_slope = clamp(row.mrc.mean_line_slope / width, -1.0, 1.0);
    let return_5 = index
        .checked_sub(5)
        .map(|previous| {
            clamp(
                ((row.close / rows[previous].close - 1.0) * 100.0) / width_pct,
                -2.0,
                2.0,
            )
        })
        .unwrap_or_default();
    let candle_range = (row.high - row.low).abs().max(f64::EPSILON);
    let body = clamp((row.close - row.open) / candle_range, -1.0, 1.0);
    let cascade_direction = signal.dir as f64;
    let cascade_strength = clamp(signal.strength / 3.0, 0.0, 1.0);
    let (remaining_duration_ratio, duration_available) = signal
        .average_duration_ms
        .filter(|average| average.is_finite() && *average > 0.0)
        .map(|average| {
            (
                clamp((average - signal.elapsed_ms as f64) / average, 0.0, 1.0),
                1.0,
            )
        })
        .unwrap_or((0.0, 0.0));

    let (window_width, direction_changes, location, available) = match window {
        Some(window) => {
            let turns = window
                .direction_changes
                .min(window.movements.saturating_sub(1)) as f64;
            let turn_ratio = if window.movements > 1 {
                turns / (window.movements - 1) as f64
            } else {
                0.0
            };
            let location = match window.price_location {
                movement_window::PriceLocation::Above => 1.0,
                movement_window::PriceLocation::Below => -1.0,
                movement_window::PriceLocation::Inside => 0.0,
            };
            (
                clamp(window.width_pct / width_pct / 2.0, 0.0, 1.0),
                turn_ratio,
                location,
                1.0,
            )
        }
        None => (0.0, 0.0, 0.0, 0.0),
    };

    FeatureVector([
        mrc_position,
        mean_slope,
        return_5,
        body,
        cascade_direction,
        cascade_strength,
        window_width,
        direction_changes,
        location,
        available,
        remaining_duration_ratio,
        duration_available,
    ])
}

fn dynamic_target_duration_bars(signal: CascadeSignal, timeframe_seconds: u64) -> Option<usize> {
    let average_duration_ms = signal
        .average_duration_ms
        .filter(|duration| duration.is_finite() && *duration > 0.0)?;
    let candle_duration_ms = timeframe_seconds.max(1) as f64 * 1_000.0;
    let remaining_duration_ms =
        (average_duration_ms - signal.elapsed_ms as f64).max(candle_duration_ms);
    let bars = (remaining_duration_ms / candle_duration_ms).ceil();
    (bars.is_finite() && bars <= usize::MAX as f64).then_some((bars as usize).max(1))
}

fn feature_distance(left: FeatureVector, right: FeatureVector) -> f64 {
    let mut weighted_distance = 0.0;
    let mut total_weight = 0.0;
    for index in 0..FEATURE_WEIGHTS.len() {
        let weight = FEATURE_WEIGHTS[index];
        let difference = left.0[index] - right.0[index];
        weighted_distance += weight * difference * difference;
        total_weight += weight;
    }
    (weighted_distance / total_weight).sqrt()
}

fn quantile(sorted_values: &[f64], fraction: f64) -> Option<f64> {
    if sorted_values.is_empty() {
        return None;
    }
    let index = (fraction.clamp(0.0, 1.0) * (sorted_values.len() - 1) as f64).round() as usize;
    sorted_values.get(index).copied()
}

fn predict(
    index: usize,
    rows: &[CandleRow],
    features: &[FeatureVector],
    signal: CascadeSignal,
    timeframe_seconds: u64,
    training_start: usize,
) -> Option<Forecast> {
    let duration_bars = dynamic_target_duration_bars(signal, timeframe_seconds)?;
    let last_training_index = index.checked_sub(duration_bars)?;
    let samples = (training_start..=last_training_index)
        .filter_map(|sample_index| {
            let target_index = sample_index.checked_add(duration_bars)?;
            let start = rows[sample_index].close;
            let finish = rows[target_index].close;
            if start <= 0.0 || finish <= 0.0 || !start.is_finite() || !finish.is_finite() {
                return None;
            }
            let forward_return_pct = (finish / start - 1.0) * 100.0;
            forward_return_pct.is_finite().then_some(TrainingSample {
                sample_index,
                features: features[sample_index],
                forward_return_pct,
            })
        })
        .collect::<Vec<_>>();
    if samples.len() < MIN_TRAINING_SAMPLES {
        return None;
    }

    let mut neighbors = samples
        .iter()
        .map(|sample| Neighbor {
            sample_index: sample.sample_index,
            distance: feature_distance(features[index], sample.features),
            forward_return_pct: sample.forward_return_pct,
        })
        .collect::<Vec<_>>();
    neighbors.sort_by(|left, right| left.distance.total_cmp(&right.distance));
    neighbors.truncate(NEIGHBOR_COUNT.min(neighbors.len()));

    let weights = neighbors
        .iter()
        .map(|neighbor| 1.0 / (neighbor.distance * neighbor.distance + 0.01))
        .collect::<Vec<_>>();
    let analogs = neighbors
        .iter()
        .zip(&weights)
        .map(|(neighbor, weight)| WeightedAnalog {
            sample_index: neighbor.sample_index,
            weight: *weight,
        })
        .collect::<Vec<_>>();
    let total_weight = weights.iter().sum::<f64>();
    let predicted_return_pct = neighbors
        .iter()
        .zip(&weights)
        .map(|(neighbor, weight)| neighbor.forward_return_pct * weight)
        .sum::<f64>()
        / total_weight;
    let up_weight = neighbors
        .iter()
        .zip(&weights)
        .filter(|(neighbor, _)| neighbor.forward_return_pct > 0.0)
        .map(|(_, weight)| weight)
        .sum::<f64>();
    let down_weight = neighbors
        .iter()
        .zip(&weights)
        .filter(|(neighbor, _)| neighbor.forward_return_pct < 0.0)
        .map(|(_, weight)| weight)
        .sum::<f64>();
    let direction_confidence_pct =
        (up_weight.max(down_weight) / total_weight * 100.0).clamp(0.0, 100.0);
    let mut returns = neighbors
        .iter()
        .map(|neighbor| neighbor.forward_return_pct)
        .collect::<Vec<_>>();
    returns.sort_by(f64::total_cmp);
    let lower_return = quantile(&returns, 0.10)?;
    let upper_return = quantile(&returns, 0.90)?;
    let close = rows[index].close;
    let predicted_price = close * (1.0 + predicted_return_pct / 100.0);
    let lower_price = close * (1.0 + lower_return / 100.0);
    let upper_price = close * (1.0 + upper_return / 100.0);

    Some(Forecast {
        time_ms: rows[index].time_ms,
        target_time_ms: rows[index].time_ms.saturating_add(
            (duration_bars as i64).saturating_mul(timeframe_seconds as i64 * 1_000),
        ),
        duration_bars,
        duration_minutes: duration_bars.saturating_mul(timeframe_seconds as usize) / 60,
        close,
        predicted_return_pct,
        predicted_price,
        lower_price: lower_price.min(upper_price),
        upper_price: lower_price.max(upper_price),
        direction_confidence_pct,
        training_samples: samples.len(),
        analog_samples: neighbors.len(),
        realized_price: index
            .checked_add(duration_bars)
            .filter(|target| *target < rows.len())
            .map(|target| rows[target].close),
        analogs,
    })
}

fn project_future(
    rows: &[CandleRow],
    source_index: usize,
    forecast: &Forecast,
    timeframe_seconds: u64,
) -> Vec<ProjectedCandle> {
    let source_price = rows[source_index].close;
    if source_price <= 0.0 || !source_price.is_finite() {
        return Vec::new();
    }
    let total_weight = forecast
        .analogs
        .iter()
        .map(|analog| analog.weight)
        .sum::<f64>();
    if total_weight <= 0.0 {
        return Vec::new();
    }

    (1..=forecast.duration_bars)
        .filter_map(|step| {
            let mut open = 0.0;
            let mut high = 0.0;
            let mut low = 0.0;
            let mut close = 0.0;
            let mut weight_sum = 0.0;
            for analog in &forecast.analogs {
                let Some(start) = rows.get(analog.sample_index) else {
                    continue;
                };
                let Some(candle) = analog
                    .sample_index
                    .checked_add(step)
                    .and_then(|index| rows.get(index))
                else {
                    continue;
                };
                if start.close <= 0.0 || !start.close.is_finite() {
                    continue;
                }
                let scale = source_price / start.close * analog.weight;
                open += candle.open * scale;
                high += candle.high * scale;
                low += candle.low * scale;
                close += candle.close * scale;
                weight_sum += analog.weight;
            }
            if weight_sum <= 0.0 {
                return None;
            }
            let scale = 1.0 / weight_sum;
            Some(ProjectedCandle {
                step,
                time_ms: rows[source_index]
                    .time_ms
                    .saturating_add((step as i64).saturating_mul(timeframe_seconds as i64 * 1_000)),
                open: open * scale,
                high: high * scale,
                low: low * scale,
                close: close * scale,
            })
        })
        .collect()
}

fn window_candle(
    row: &CandleRow,
    signal: CascadeSignal,
    window: Option<MovementWindow>,
) -> WindowCandle {
    WindowCandle {
        time_ms: row.time_ms,
        open: row.open,
        high: row.high,
        low: row.low,
        close: row.close,
        mrc: row.mrc,
        cascade_direction: signal.dir,
        cascade_phase: signal.phase.as_str(),
        cascade_strength: signal.strength,
        window_low: window.map(|value| value.low),
        window_high: window.map(|value| value.high),
        window_width_pct: window.map(|value| value.width_pct),
        window_direction_changes: window.map(|value| value.direction_changes),
        window_price_location: window.map(|value| value.price_location.as_str()),
    }
}

pub(crate) fn analyze(
    symbol_key: &str,
    timeframe_seconds: u64,
    requested_history: usize,
    analysis_offset_minutes: usize,
    rows: &[CandleRow],
) -> ResearchReport {
    let visible_start = rows.len().saturating_sub(requested_history);
    let training_start = MRC_WARMUP_BARS.min(rows.len());
    let mut cascade_state = CascadeState::default();
    let mut all_signals = Vec::with_capacity(rows.len());
    let mut all_windows = Vec::with_capacity(rows.len());
    let mut all_features = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let signal = cascade::update_from_candle_at(
            &mut cascade_state,
            &Candle {
                open: row.open,
                high: row.high,
                low: row.low,
                close: row.close,
            },
            row.time_ms.max(0) as u64,
        );
        let window = movement_window::detect(&cascade_state, row.close);
        all_features.push(make_feature(index, rows, signal, window));
        all_signals.push(signal);
        all_windows.push(window);
    }

    let visible_rows = &rows[visible_start..];
    let forecasts = (visible_start..rows.len())
        .map(|index| {
            predict(
                index,
                rows,
                &all_features,
                all_signals[index],
                timeframe_seconds,
                training_start,
            )
        })
        .collect::<Vec<_>>();
    let offset_bars =
        analysis_offset_minutes.saturating_mul(60) / timeframe_seconds.max(1) as usize;
    let analysis_row_index = rows.len().checked_sub(offset_bars.saturating_add(1));
    let analysis_index = analysis_row_index
        .map(|index| index.saturating_sub(visible_start))
        .filter(|index| *index < visible_rows.len());
    let latest_forecast = analysis_index.and_then(|index| forecasts.get(index).cloned().flatten());
    let latest_projection = latest_forecast
        .as_ref()
        .zip(analysis_row_index)
        .map(|(forecast, source_index)| {
            project_future(rows, source_index, forecast, timeframe_seconds)
        })
        .unwrap_or_default();
    let mut resolved = 0usize;
    let mut direction_matches = 0usize;
    let mut total_absolute_error = 0.0;
    for forecast in forecasts.iter().flatten() {
        let Some(realized_price) = forecast.realized_price else {
            continue;
        };
        resolved += 1;
        let predicted_move = forecast.predicted_price - forecast.close;
        let realized_move = realized_price - forecast.close;
        if predicted_move.signum() == realized_move.signum() {
            direction_matches += 1;
        }
        total_absolute_error +=
            ((forecast.predicted_price - realized_price).abs() / forecast.close * 100.0).abs();
    }
    let forecast_count = forecasts
        .iter()
        .filter(|forecast| forecast.is_some())
        .count();
    let validation = ValidationSummary {
        forecasts: forecast_count,
        resolved_forecasts: resolved,
        direction_accuracy_pct: (resolved > 0)
            .then_some(direction_matches as f64 / resolved as f64 * 100.0),
        mean_absolute_error_pct: (resolved > 0).then_some(total_absolute_error / resolved as f64),
    };
    let candles = visible_rows
        .iter()
        .zip(all_signals[visible_start..].iter().copied())
        .zip(all_windows[visible_start..].iter().copied())
        .map(|((row, signal), window)| window_candle(row, signal, window))
        .collect::<Vec<_>>();

    ResearchReport {
        schema_version: 4,
        symbol_key: symbol_key.to_string(),
        timeframe_seconds,
        history_requested: requested_history,
        history_loaded: visible_rows.len(),
        training_history_loaded: rows.len().saturating_sub(training_start),
        mrc_period: MRC_PERIOD,
        mrc_warmup_bars: MRC_WARMUP_BARS,
        analysis_offset_minutes,
        analysis_index,
        analysis_time_ms: analysis_row_index.map(|index| rows[index].time_ms),
        realized_future_bars: analysis_row_index
            .map(|index| rows.len().saturating_sub(index.saturating_add(1)))
            .unwrap_or_default(),
        source_start_ms: visible_rows.first().map(|row| row.time_ms),
        source_end_ms: visible_rows.last().map(|row| row.time_ms),
        learning: LearningSettings {
            method: "walk_forward_weighted_nearest_analogs_with_cascade_timing",
            target_timing: "Per candle: remaining average completed movement duration (cascade average duration minus current movement elapsed time), rounded up to full candles and floored at one candle.",
            neighbors: NEIGHBOR_COUNT,
            minimum_training_samples: MIN_TRAINING_SAMPLES,
            training_updates: "Each candle forecasts over its own cascade-derived remaining duration; a historical outcome is used only after that candle-specific duration has closed.",
            leakage_control: "Each forecast uses only historical examples whose full forward interval for that forecast duration ended at or before the forecast candle.",
        },
        validation,
        latest_forecast,
        latest_projection,
        candles,
        forecasts,
    }
}
