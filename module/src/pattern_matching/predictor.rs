use super::history::Outcome;
use super::{CurrentPattern, Forecast};

pub const MIN_MATCHES: u32 = 3;
pub const MIN_DIRECTION_AGREEMENT: f64 = 0.60;

pub fn predict(outcomes: &[Outcome]) -> Option<Forecast> {
    if outcomes.len() < MIN_MATCHES as usize {
        return None;
    }
    let rising = outcomes.iter().filter(|item| item.return_pct > 0.0).count();
    let falling = outcomes.iter().filter(|item| item.return_pct < 0.0).count();
    let direction = match rising.cmp(&falling) {
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
    };
    if direction == 0 {
        return None;
    }
    let matching = outcomes
        .iter()
        .filter(|item| item.return_pct * direction as f64 > 0.0)
        .collect::<Vec<_>>();
    let agreement = matching.len() as f64 / outcomes.len() as f64;
    if agreement < MIN_DIRECTION_AGREEMENT {
        return None;
    }

    Some(Forecast {
        direction,
        matches: outcomes.len() as u32,
        agreement,
        expected_return_pct: median(matching.iter().map(|item| item.return_pct).collect()),
        favorable_move_pct: mean(
            matching
                .iter()
                .map(|item| {
                    if direction > 0 {
                        item.up_excursion_pct
                    } else {
                        item.down_excursion_pct
                    }
                })
                .collect(),
        ),
        adverse_move_pct: mean(
            matching
                .iter()
                .map(|item| {
                    if direction > 0 {
                        item.down_excursion_pct
                    } else {
                        item.up_excursion_pct
                    }
                })
                .collect(),
        ),
    })
}

pub fn summary(
    pattern: CurrentPattern,
    outcome_count: usize,
    forecast: Option<Forecast>,
) -> String {
    let definition = pattern.kind.description();
    match forecast {
        Some(forecast) => format!(
            "pattern={} description={} direction={} breakout_high={} breakout_low={} extension_high={:.4}% extension_low={:.4}% spike={:?} historical_matches={} predicted_direction={} agreement={:.2} expected_15m_return={:+.4}% favorable={:.4}% adverse={:.4}%",
            pattern.kind.as_str(),
            definition,
            pattern.direction,
            pattern.confirmed_new_high,
            pattern.confirmed_new_low,
            pattern.high_extension_pct,
            pattern.low_extension_pct,
            pattern.spike_direction,
            forecast.matches,
            forecast.direction,
            forecast.agreement,
            forecast.expected_return_pct,
            forecast.favorable_move_pct,
            forecast.adverse_move_pct
        ),
        None => format!(
            "pattern={} description={} direction={} breakout_high={} breakout_low={} extension_high={:.4}% extension_low={:.4}% spike={:?} historical_matches={} forecast=insufficient-or-mixed",
            pattern.kind.as_str(),
            definition,
            pattern.direction,
            pattern.confirmed_new_high,
            pattern.confirmed_new_low,
            pattern.high_extension_pct,
            pattern.low_extension_pct,
            pattern.spike_direction,
            outcome_count
        ),
    }
}

pub fn mean(values: Vec<f64>) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}
