use super::cascade::{self, CascadeState};
use super::movement_window::{MovementWindow, PriceLocation};

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
struct RangeBounds {
    low: f64,
    high: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StableRangeState {
    active: Option<RangeBounds>,
    last_movement_start_ms: u64,
    closed_at_movement_start_ms: u64,
}

impl StableRangeState {
    pub fn update(
        &mut self,
        cascade: &CascadeState,
        candidate: Option<MovementWindow>,
        price: f64,
    ) -> Option<MovementWindow> {
        let movement_start_ms = cascade.movement_start_ms;
        if movement_start_ms == 0 {
            return None;
        }

        if let Some(range) = self.active {
            if movement_start_ms != self.last_movement_start_ms {
                self.last_movement_start_ms = movement_start_ms;
                if completed_movement_broke(cascade, range) {
                    self.active = None;
                    self.closed_at_movement_start_ms = movement_start_ms;
                    return None;
                }
            }
            return MovementWindow::from_bounds(range.low, range.high, cascade.avg_size, price);
        }

        if self.closed_at_movement_start_ms == movement_start_ms {
            return None;
        }

        let candidate = candidate.filter(|window| {
            window.price_location == PriceLocation::Inside && window.high > window.low
        })?;
        let range = RangeBounds {
            low: candidate.low,
            high: candidate.high,
        };
        self.active = Some(range);
        self.last_movement_start_ms = movement_start_ms;
        MovementWindow::from_bounds(range.low, range.high, cascade.avg_size, price)
    }

    pub fn bounds(&self) -> Option<(f64, f64)> {
        self.active.map(|range| (range.low, range.high))
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

fn completed_movement_broke(state: &CascadeState, range: RangeBounds) -> bool {
    let Some(completed) = cascade::movement_history(state).last().copied() else {
        return false;
    };
    if !state.last_break.is_finite() || state.last_break <= 0.0 {
        return false;
    }

    match completed.dir {
        1 => state.last_break > range.high,
        -1 => state.last_break < range.low,
        _ => false,
    }
}
