//! Continuous direction prediction built from incremental MRC-touch state.
//!
//! The staircase classifier describes the structural route.  This module
//! estimates what is happening now around that route: continuation,
//! recovery, or sideways movement.  It deliberately returns probabilities and
//! expected value instead of a binary long/short rule.

use crate::detection::mrc_touch::MrcTouchSignal;
use crate::detection::mrc_touch_learning::{
    MrcTouchLearningSignal, TouchMatchScope, TouchTimeBucket,
};

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
    /// Empirical outcome of previous exact MRC touches in the current 5m
    /// context. This is the only directional authority of this predictor.
    pub touch_learning: MrcTouchLearningSignal,
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
            touch_learning: MrcTouchLearningSignal::default(),
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
    /// Whether the empirical touch history permits this candidate. A missing
    /// or unreliable history leaves this true so the existing route learner
    /// remains the fallback.
    pub candidate_allowed: bool,
    pub state: MrcPredictionState,
    pub candidate_direction: i8,
    pub mean_distance_pct: f64,
    pub structural_bias: f64,
    pub touch_bias: f64,
    pub learned_direction: i8,
    pub learned_samples: u32,
    pub learned_scope: TouchMatchScope,
    pub learned_time_bucket: TouchTimeBucket,
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
            candidate_allowed: true,
            state: MrcPredictionState::Unknown,
            candidate_direction: 0,
            mean_distance_pct: 0.0,
            structural_bias: 0.0,
            touch_bias: 0.0,
            learned_direction: 0,
            learned_samples: 0,
            learned_scope: TouchMatchScope::None,
            learned_time_bucket: TouchTimeBucket::Unknown,
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
        self.learned_samples > 0 && self.learned_scope != TouchMatchScope::None
    }

    pub fn summary(self) -> String {
        format!(
            "state={} learn={} samples={} scope={} tf={} cand={} allowed={} mean={:+.3}%",
            self.state.as_str(),
            direction_label(self.learned_direction),
            self.learned_samples,
            self.learned_scope.as_str(),
            self.learned_time_bucket.as_str(),
            direction_label(self.candidate_direction),
            self.candidate_allowed,
            self.mean_distance_pct,
        )
    }
}

/// Build a continuous prediction from the current MRC-touch signal and the
/// learned route statistics of one candidate.
pub fn predict(signal: MrcTouchSignal, context: MrcDirectionContext) -> MrcDirectionPrediction {
    let mean_distance_pct = finite_or_zero(context.mean_distance_pct);
    let learning = context.touch_learning;
    let direction_score = if learning.reliable {
        learning.learned_direction
    } else {
        0
    } as f64;
    let (up_probability, down_probability, sideways_probability) = match direction_score as i8 {
        1 => (1.0, 0.0, 0.0),
        -1 => (0.0, 1.0, 0.0),
        _ => (0.0, 0.0, 1.0),
    };
    let confidence = u8::from(learning.reliable) as f64;
    let candidate_allowed = !learning.reliable
        || learning.learned_direction == 0
        || learning.learned_direction == context.candidate_direction;
    let state = match learning.learned_direction {
        1 if learning.reliable => MrcPredictionState::ContinuationUp,
        -1 if learning.reliable => MrcPredictionState::ContinuationDown,
        _ if learning.current_line.is_some() => MrcPredictionState::Sideways,
        _ => MrcPredictionState::Unknown,
    };

    MrcDirectionPrediction {
        up_probability,
        down_probability,
        sideways_probability,
        direction_score,
        confidence,
        candidate_edge_pct: 0.0,
        candidate_allowed,
        state,
        candidate_direction: context.candidate_direction,
        mean_distance_pct,
        structural_bias: context.structural_direction as f64,
        touch_bias: signal.direction_score,
        learned_direction: learning.learned_direction,
        learned_samples: learning.learned_samples,
        learned_scope: learning.match_scope,
        learned_time_bucket: learning.learned_time_bucket,
    }
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
