use super::cascade::{self, CascadeState};

// Match the cascade's existing five-movement context for continuation estimates.
const WINDOW_MOVEMENTS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceLocation {
    Inside,
    Above,
    Below,
}

impl PriceLocation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inside => "inside",
            Self::Above => "above",
            Self::Below => "below",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementWindow {
    pub movements: usize,
    pub low: f64,
    pub high: f64,
    pub average_movement_size: f64,
    pub width_pct: f64,
    pub direction_changes: usize,
    pub price_location: PriceLocation,
}

pub fn detect(state: &CascadeState, price: f64) -> Option<MovementWindow> {
    if !price.is_finite() || price <= 0.0 {
        return None;
    }

    let history = cascade::movement_history(state);
    let start = history.len().saturating_sub(WINDOW_MOVEMENTS);
    let movements = &history[start..];
    if movements.is_empty() {
        return None;
    }

    let low = movements
        .iter()
        .map(|movement| movement.low)
        .min_by(f64::total_cmp)?;
    let high = movements
        .iter()
        .map(|movement| movement.high)
        .max_by(f64::total_cmp)?;
    if !low.is_finite() || low <= 0.0 || !high.is_finite() || high < low {
        return None;
    }

    let direction_changes = movements
        .windows(2)
        .filter(|pair| pair[0].dir != pair[1].dir)
        .count();
    let price_location = if price > high {
        PriceLocation::Above
    } else if price < low {
        PriceLocation::Below
    } else {
        PriceLocation::Inside
    };

    Some(MovementWindow {
        movements: movements.len(),
        low,
        high,
        average_movement_size: state.avg_size,
        width_pct: (high - low) / price * 100.0,
        direction_changes,
        price_location,
    })
}
