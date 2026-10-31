use super::catalog::PatternKind;
use super::matcher::{self, PATTERN_BARS, PREDICTION_HORIZON_BARS};
use lte_strategy_bridge::abi::ModuleKline;

#[derive(Debug, Clone, Copy)]
pub struct Outcome {
    pub return_pct: f64,
    pub up_excursion_pct: f64,
    pub down_excursion_pct: f64,
}

pub fn matching_outcomes(rows: &[&ModuleKline], kind: PatternKind) -> Vec<Outcome> {
    let current_start = rows.len().saturating_sub(PATTERN_BARS);
    let required = PATTERN_BARS + PREDICTION_HORIZON_BARS;
    let Some(last_start) = current_start.checked_sub(required) else {
        return Vec::new();
    };

    let mut outcomes = Vec::new();
    let mut start = 0;
    while start <= last_start {
        let pattern_end = start + PATTERN_BARS;
        let outcome_end = pattern_end + PREDICTION_HORIZON_BARS;
        let window = &rows[start..pattern_end];
        let matches = matcher::current(window).is_some_and(|pattern| pattern.kind == kind);
        if matches {
            if let Some(outcome) = outcome(rows[pattern_end - 1], &rows[pattern_end..outcome_end]) {
                outcomes.push(outcome);
            }
            start = outcome_end;
        } else {
            start += 1;
        }
    }
    outcomes
}

fn outcome(base: &ModuleKline, future: &[&ModuleKline]) -> Option<Outcome> {
    let last = future.last()?;
    if !base.close.is_finite() || base.close <= 0.0 {
        return None;
    }
    let high = future
        .iter()
        .map(|row| row.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let low = future
        .iter()
        .map(|row| row.low)
        .fold(f64::INFINITY, f64::min);
    Some(Outcome {
        return_pct: (last.close - base.close) / base.close * 100.0,
        up_excursion_pct: (high - base.close) / base.close * 100.0,
        down_excursion_pct: (base.close - low) / base.close * 100.0,
    })
}
