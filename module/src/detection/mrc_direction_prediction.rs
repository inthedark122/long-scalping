//! Continuous direction prediction built from incremental MRC-touch state.
//!
//! The staircase classifier describes the structural route.  This module
//! estimates what is happening now around that route: continuation,
//! recovery, or sideways movement.  It deliberately returns probabilities and
//! expected value instead of a binary long/short rule.

use crate::detection::mrc_touch::MrcTouchSignal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MrcPredictionState {
    Unknown,
    Sideways,
    RecoveryUp,
    RecoveryDown,
    ContinuationUp,
    ContinuationDown,
    Conflict,
}

impl Default for MrcPredictionState {
    fn default() -> Self {
        Self::Unknown
    }
}

impl MrcPredictionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Sideways => "sideway",
            Self::RecoveryUp => "recovery-up",
            Self::RecoveryDown => "recovery-down",
            Self::ContinuationUp => "continuation-up",
            Self::ContinuationDown => "continuation-down",
            Self::Conflict => "conflict",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MrcDirectionContext {
    /// Structural direction supplied by the current staircase candidate.
    pub structural_direction: i8,
    /// Candidate direction being evaluated.  Zero produces a general market
    /// prediction without an entry-specific expected value.
    pub candidate_direction: i8,
    /// Signed EMA distance from price to the mean line, in percent.
    pub mean_distance_pct: f64,
    /// Learned route outcome from the existing MRC crossing statistics.
    pub historical_expected_pct: Option<f64>,
    pub historical_success_rate: Option<f64>,
    pub pattern_samples: u32,
    pub target_samples: u32,
    /// Current candidate reward and risk distances, in percent.
    pub target_distance_pct: f64,
    pub stop_distance_pct: f64,
}

impl Default for MrcDirectionContext {
    fn default() -> Self {
        Self {
            structural_direction: 0,
            candidate_direction: 0,
            mean_distance_pct: 0.0,
            historical_expected_pct: None,
            historical_success_rate: None,
            pattern_samples: 0,
            target_samples: 0,
            target_distance_pct: 0.0,
            stop_distance_pct: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MrcDirectionPrediction {
    pub up_probability: f64,
    pub down_probability: f64,
    pub sideways_probability: f64,
    /// Positive values support upward movement; negative values support
    /// downward movement.
    pub direction_score: f64,
    /// Confidence is continuous and includes both current touch activity and
    /// the amount of learned route evidence.
    pub confidence: f64,
    /// Expected value for the candidate direction supplied in the context.
    pub candidate_edge_pct: f64,
    pub state: MrcPredictionState,
    pub candidate_direction: i8,
    pub mean_distance_pct: f64,
    pub structural_bias: f64,
    pub touch_bias: f64,
}

impl Default for MrcDirectionPrediction {
    fn default() -> Self {
        Self {
            up_probability: 0.0,
            down_probability: 0.0,
            sideways_probability: 1.0,
            direction_score: 0.0,
            confidence: 0.0,
            candidate_edge_pct: 0.0,
            state: MrcPredictionState::Unknown,
            candidate_direction: 0,
            mean_distance_pct: 0.0,
            structural_bias: 0.0,
            touch_bias: 0.0,
        }
    }
}

impl MrcDirectionPrediction {
    pub fn probability(self, direction: i8) -> f64 {
        if direction > 0 {
            self.up_probability
        } else if direction < 0 {
            self.down_probability
        } else {
            self.sideways_probability
        }
    }

    pub fn has_evidence(self) -> bool {
        self.confidence > 0.0 || self.up_probability > 0.0 || self.down_probability > 0.0
    }

    pub fn summary(self) -> String {
        format!(
            "state={} up={:.3} down={:.3} side={:.3} score={:.3} conf={:.3} cand={} edge={:+.3}% mean={:+.3}% touch={:+.3} structural={:+.3}",
            self.state.as_str(),
            self.up_probability,
            self.down_probability,
            self.sideways_probability,
            self.direction_score,
            self.confidence,
            direction_label(self.candidate_direction),
            self.candidate_edge_pct,
            self.mean_distance_pct,
            self.touch_bias,
            self.structural_bias,
        )
    }
}

/// Build a continuous prediction from the current MRC-touch signal and the
/// learned route statistics of one candidate.
pub fn predict(signal: MrcTouchSignal, context: MrcDirectionContext) -> MrcDirectionPrediction {
    let mean_distance_pct = finite_or_zero(context.mean_distance_pct);
    let price_angle = (signal.price_angle_deg / 45.0).tanh();
    let mean_reversion = (-mean_distance_pct / 1.5).clamp(-1.0, 1.0);

    // The touch layer is already EMA-smoothed.  Keep the current movement as
    // the largest input, then use angle and location to identify recovery from
    // one side of the mean without turning either into a hard rule.
    let touch_bias = (signal.direction_score * 0.45
        + signal.stairs_pressure * 0.25
        + price_angle * 0.15
        + mean_reversion * 0.15)
        .clamp(-1.0, 1.0);

    let route_confidence = sample_confidence(context.pattern_samples, context.target_samples);
    let historical_bias = historical_bias(
        context.structural_direction,
        context.historical_expected_pct,
        context.historical_success_rate,
        route_confidence,
    );
    // Current touch information should be able to describe a recovery against
    // an old staircase, while learned route results still provide a prior.
    let direction_score = (touch_bias * 0.70 + historical_bias * 0.30).clamp(-1.0, 1.0);

    let sideways_base = signal.sideways_score.clamp(0.0, 1.0);
    let directional_mass = (1.0 - sideways_base * 0.75).clamp(0.20, 1.0);
    let up_probability = (directional_mass * (0.5 + direction_score * 0.5)).clamp(0.0, 1.0);
    let down_probability = (directional_mass * (0.5 - direction_score * 0.5)).clamp(0.0, 1.0);
    let sideways_probability = (1.0 - directional_mass).clamp(0.0, 1.0);
    let confidence =
        ((direction_score.abs() * 0.65 + signal.interest * 0.20 + route_confidence * 0.15)
            * (1.0 - sideways_probability))
            .clamp(0.0, 1.0);

    let candidate_edge_pct = candidate_edge(
        context.candidate_direction,
        up_probability,
        down_probability,
        context.target_distance_pct,
        context.stop_distance_pct,
        context.historical_expected_pct,
        route_confidence,
        confidence,
    );
    let state = prediction_state(
        signal,
        context,
        direction_score,
        sideways_probability,
        confidence,
    );

    MrcDirectionPrediction {
        up_probability,
        down_probability,
        sideways_probability,
        direction_score,
        confidence,
        candidate_edge_pct,
        state,
        candidate_direction: context.candidate_direction,
        mean_distance_pct,
        structural_bias: historical_bias,
        touch_bias,
    }
}

fn candidate_edge(
    direction: i8,
    up_probability: f64,
    down_probability: f64,
    target_distance_pct: f64,
    stop_distance_pct: f64,
    historical_expected_pct: Option<f64>,
    route_confidence: f64,
    confidence: f64,
) -> f64 {
    if !matches!(direction, 1 | -1) {
        return 0.0;
    }
    let probability = if direction > 0 {
        up_probability
    } else {
        down_probability
    };
    let target = finite_or_zero(target_distance_pct).max(0.0);
    let stop = finite_or_zero(stop_distance_pct).max(0.0);
    let geometric_edge = probability * target - (1.0 - probability) * stop;
    let learned_edge = historical_expected_pct
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
        * route_confidence;
    (geometric_edge + learned_edge * 0.25) * (0.50 + confidence * 0.50)
}

fn historical_bias(
    structural_direction: i8,
    expected_pct: Option<f64>,
    success_rate: Option<f64>,
    route_confidence: f64,
) -> f64 {
    if !matches!(structural_direction, 1 | -1) || route_confidence <= 0.0 {
        return 0.0;
    }
    let expected = expected_pct
        .filter(|value| value.is_finite())
        .map(|value| (value / 1.0).tanh())
        .unwrap_or(0.0);
    let success = success_rate
        .filter(|value| value.is_finite())
        .map(|value| (value.clamp(0.0, 1.0) - 0.5) * 2.0)
        .unwrap_or(0.0);
    structural_direction as f64 * (expected * 0.60 + success * 0.40) * route_confidence
}

fn prediction_state(
    signal: MrcTouchSignal,
    context: MrcDirectionContext,
    direction_score: f64,
    sideways_probability: f64,
    confidence: f64,
) -> MrcPredictionState {
    if !signal.now_ms.gt(&0) && signal.nearest_line.is_none() {
        return MrcPredictionState::Unknown;
    }
    if sideways_probability >= 0.35 || confidence < 0.10 {
        return MrcPredictionState::Sideways;
    }
    if context.candidate_direction != 0
        && direction_score * (context.candidate_direction as f64) < -0.15
    {
        return MrcPredictionState::Conflict;
    }

    let recovery = signal
        .line(4)
        .map(|line| line.distance_ema_pct.abs() > 0.15)
        .unwrap_or(false)
        && signal.direction_score.abs() > 0.05
        && signal.direction_score.signum() == (-context.mean_distance_pct).signum();
    if recovery {
        if direction_score > 0.0 {
            MrcPredictionState::RecoveryUp
        } else {
            MrcPredictionState::RecoveryDown
        }
    } else if direction_score > 0.0 {
        MrcPredictionState::ContinuationUp
    } else if direction_score < 0.0 {
        MrcPredictionState::ContinuationDown
    } else {
        MrcPredictionState::Sideways
    }
}

fn sample_confidence(pattern_samples: u32, target_samples: u32) -> f64 {
    let pattern = pattern_samples as f64 / (pattern_samples as f64 + 4.0);
    let target = target_samples as f64 / (target_samples as f64 + 3.0);
    (pattern * target).clamp(0.0, 1.0)
}

fn finite_or_zero(value: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

fn direction_label(direction: i8) -> &'static str {
    if direction > 0 {
        "up"
    } else if direction < 0 {
        "down"
    } else {
        "na"
    }
}
