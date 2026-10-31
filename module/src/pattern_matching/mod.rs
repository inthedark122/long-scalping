mod catalog;
mod decision;
mod history;
mod matcher;
mod predictor;

use lte_strategy_bridge::abi::ModuleKline;
use std::collections::VecDeque;

pub use catalog::{PatternKind, DEFINITIONS};
pub use decision::extend_target_to_forecast;
pub use matcher::{PATTERN_BARS, PREDICTION_HORIZON_BARS};
pub use predictor::MIN_MATCHES;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurrentPattern {
    pub kind: PatternKind,
    pub direction: i8,
    pub confirmed_new_high: bool,
    pub confirmed_new_low: bool,
    pub high_extension_pct: f64,
    pub low_extension_pct: f64,
    pub spike_direction: Option<i8>,
    pub mean_cross_direction: Option<i8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Forecast {
    pub direction: i8,
    pub matches: u32,
    pub agreement: f64,
    pub expected_return_pct: f64,
    pub favorable_move_pct: f64,
    pub adverse_move_pct: f64,
}

impl Forecast {
    pub fn supports(self, direction: i8) -> bool {
        self.matches >= MIN_MATCHES
            && self.direction == direction
            && self.agreement >= predictor::MIN_DIRECTION_AGREEMENT
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub current: Option<CurrentPattern>,
    pub forecast: Option<Forecast>,
    pub summary: String,
}

pub fn evaluate(rows: Option<&VecDeque<ModuleKline>>, now_ms: u64) -> Evaluation {
    let bars = matcher::closed_rows(rows, now_ms);
    let Some(current) = matcher::current(&bars) else {
        return Evaluation {
            current: None,
            forecast: None,
            summary: format!("candle_pattern=waiting bars={}", bars.len()),
        };
    };
    let outcomes = history::matching_outcomes(&bars, current.kind);
    let forecast = predictor::predict(&outcomes);
    let summary = predictor::summary(current, outcomes.len(), forecast);
    Evaluation {
        current: Some(current),
        forecast,
        summary,
    }
}
