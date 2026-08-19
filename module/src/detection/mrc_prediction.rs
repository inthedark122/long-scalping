//! Per-line movement information for the MRC route.
//!
//! This module deliberately contains no probabilities and no special outer-
//! band forecast. Every line gets a route role, movement distance, stop
//! distance, and the best available time estimate. A line without an exact
//! history sample is still useful: its time is scaled from a nearby observed
//! route or estimated from the current cascade speed.

use crate::state::MrcForecastStats;

pub const MRC_LINE_COUNT: usize = 9;

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MrcPredictionContext {
    pub ntps: i64,
    pub ntps_fast_time_ms: Option<i64>,
    pub cascade_direction: i8,
    pub cascade_speed_pct_per_min: Option<f64>,
    pub cascade_strength: f64,
    pub cascade_stretched: bool,
}

impl MrcPredictionContext {
    pub fn fast(self) -> bool {
        self.ntps_fast_time_ms.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MrcLineRole {
    Current,
    FutureRoute,
    PlannedTarget,
    CoveredByTarget { target_line: u8 },
    BeyondTarget,
    OppositeDirection,
    NoDirection,
}

impl Default for MrcLineRole {
    fn default() -> Self {
        Self::NoDirection
    }
}

impl MrcLineRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::FutureRoute => "future-route",
            Self::PlannedTarget => "planned-target",
            Self::CoveredByTarget { .. } => "covered-by-target",
            Self::BeyondTarget => "beyond-target",
            Self::OppositeDirection => "opposite-direction",
            Self::NoDirection => "no-direction",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MrcLineTimeSource {
    Current,
    CascadeSpeed,
    HistoricalExact,
    HistoricalScaled,
    Unavailable,
}

impl Default for MrcLineTimeSource {
    fn default() -> Self {
        Self::Unavailable
    }
}

impl MrcLineTimeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::CascadeSpeed => "cascade-speed",
            Self::HistoricalExact => "historical-exact",
            Self::HistoricalScaled => "historical-scaled",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MrcLinePlan {
    pub line_idx: u8,
    pub line_price: f64,
    pub direction: i8,
    /// Movement measured in the active direction: positive is favorable,
    /// negative is adverse, and zero is the current line.
    pub move_pct: f64,
    /// Absolute price distance from the entry to this line.
    pub distance_pct: f64,
    /// Absolute distance from the entry to the stop line.
    pub stop_loss_pct: f64,
    pub stop_loss_set: bool,
    /// Exact or scaled historical route estimate. Zero means that no
    /// historical estimate exists; `estimated_time_ms` carries the selected
    /// fallback when cascade speed is available.
    pub historical_time_ms: f64,
    pub historical_samples: u32,
    pub cascade_time_ms: f64,
    pub estimated_time_ms: f64,
    pub time_source: MrcLineTimeSource,
    pub route_role: MrcLineRole,
    pub ntps: i64,
    pub ntps_fast_time_ms: Option<i64>,
    pub cascade_direction: i8,
    pub cascade_speed_pct_per_min: Option<f64>,
}

fn valid_price(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn price_delta_pct(entry_price: f64, target_price: f64) -> f64 {
    if !valid_price(entry_price) || !valid_price(target_price) {
        return 0.0;
    }
    let value = (target_price - entry_price) / entry_price * 100.0;
    value.is_finite().then_some(value).unwrap_or(0.0)
}

fn route_steps(source_line: u8, line_idx: u8) -> usize {
    (line_idx as i16 - source_line as i16).unsigned_abs() as usize
}

fn route_role(
    source_line: u8,
    line_idx: u8,
    direction: i8,
    target_line: Option<u8>,
) -> MrcLineRole {
    if line_idx == source_line {
        return MrcLineRole::Current;
    }
    if !matches!(direction, 1 | -1) {
        return MrcLineRole::NoDirection;
    }
    let is_forward = if direction > 0 {
        line_idx > source_line
    } else {
        line_idx < source_line
    };
    if !is_forward {
        return MrcLineRole::OppositeDirection;
    }
    let Some(target_line) = target_line else {
        return MrcLineRole::FutureRoute;
    };
    if line_idx == target_line {
        MrcLineRole::PlannedTarget
    } else if (direction > 0 && line_idx < target_line) || (direction < 0 && line_idx > target_line)
    {
        MrcLineRole::CoveredByTarget { target_line }
    } else {
        MrcLineRole::BeyondTarget
    }
}

fn historical_estimate(
    source_line: u8,
    line_idx: u8,
    direction: i8,
    stats: [MrcForecastStats; MRC_LINE_COUNT],
) -> (f64, u32, MrcLineTimeSource) {
    if line_idx == source_line {
        return (0.0, 0, MrcLineTimeSource::Current);
    }
    let is_forward = if direction > 0 {
        line_idx > source_line
    } else {
        line_idx < source_line
    };
    if !is_forward {
        return (0.0, 0, MrcLineTimeSource::Unavailable);
    }
    if let Some(value) = stats[line_idx as usize].average_duration_ms() {
        return (
            value,
            stats[line_idx as usize].count,
            MrcLineTimeSource::HistoricalExact,
        );
    }

    let target_steps = route_steps(source_line, line_idx);
    let nearest = stats
        .iter()
        .enumerate()
        .filter(|(index, value)| {
            let observed_line = *index as u8;
            observed_line != source_line
                && if direction > 0 {
                    observed_line > source_line
                } else {
                    observed_line < source_line
                }
                && value.count > 0
                && value.average_duration_ms().is_some()
        })
        .min_by_key(|(index, _)| route_steps(source_line, *index as u8));
    let Some((index, observed)) = nearest else {
        return (0.0, 0, MrcLineTimeSource::Unavailable);
    };
    let observed_steps = route_steps(source_line, index as u8).max(1);
    let time =
        observed.average_duration_ms().unwrap_or(0.0) / observed_steps as f64 * target_steps as f64;
    (time, observed.count, MrcLineTimeSource::HistoricalScaled)
}

fn cascade_time_ms(distance_pct: f64, context: MrcPredictionContext) -> Option<f64> {
    let speed = context
        .cascade_speed_pct_per_min
        .filter(|value| value.is_finite() && *value > 0.0)?;
    (distance_pct > 0.0).then_some(distance_pct / speed * 60_000.0)
}

/// Build a complete information plan for every MRC line.
pub fn build_line_plans(
    lines: [f64; MRC_LINE_COUNT],
    source_line: u8,
    entry_price: f64,
    direction: i8,
    stop_price: Option<f64>,
    target_line: Option<u8>,
    stats: [MrcForecastStats; MRC_LINE_COUNT],
    context: MrcPredictionContext,
) -> [MrcLinePlan; MRC_LINE_COUNT] {
    std::array::from_fn(|index| {
        let line_idx = index as u8;
        let raw_move_pct = price_delta_pct(entry_price, lines[index]);
        let move_pct = if matches!(direction, 1 | -1) {
            raw_move_pct * direction as f64
        } else {
            raw_move_pct
        };
        let distance_pct = raw_move_pct.abs();
        let stop_loss_pct = stop_price
            .map(|stop| price_delta_pct(entry_price, stop).abs())
            .unwrap_or(0.0);
        let role = route_role(source_line, line_idx, direction, target_line);
        let (historical_time_ms, historical_samples, historical_source) =
            historical_estimate(source_line, line_idx, direction, stats);
        let cascade_time_ms = cascade_time_ms(distance_pct, context).unwrap_or(0.0);
        let (estimated_time_ms, time_source) = match historical_source {
            MrcLineTimeSource::Current => (0.0, MrcLineTimeSource::Current),
            _ if cascade_time_ms > 0.0 => (cascade_time_ms, MrcLineTimeSource::CascadeSpeed),
            _ => (historical_time_ms, historical_source),
        };

        MrcLinePlan {
            line_idx,
            line_price: lines[index],
            direction,
            move_pct,
            distance_pct,
            stop_loss_pct,
            stop_loss_set: stop_price.is_some_and(valid_price),
            historical_time_ms,
            historical_samples,
            cascade_time_ms,
            estimated_time_ms,
            time_source,
            route_role: role,
            ntps: context.ntps,
            ntps_fast_time_ms: context.ntps_fast_time_ms,
            cascade_direction: context.cascade_direction,
            cascade_speed_pct_per_min: context.cascade_speed_pct_per_min,
        }
    })
}
