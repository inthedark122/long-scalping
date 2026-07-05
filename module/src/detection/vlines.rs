use crate::state::{VLineState, VSideState};

fn close_v(side: &mut VSideState, power: f64) {
    side.prev_power = side.last_power;
    side.last_power = power;
    side.count = side.count.saturating_add(1);
}

pub fn update(vlines: &mut [VLineState; 9], lines: [f64; 9], price: f64, tick: u64) {
    for (idx, line) in lines.iter().enumerate() {
        if *line <= 0.0 {
            continue;
        }
        let tracker = &mut vlines[idx];
        let is_above = price >= *line;

        match tracker.was_above {
            None => tracker.was_above = Some(is_above),
            Some(prev) if prev != is_above => {
                tracker.last_cross_tick = tick;
                tracker.last_cross_dir = if is_above { 1 } else { -1 };

                if is_above {
                    if let Some(trough) = tracker.trough.take() {
                        let power =
                            ((*line - trough) / line.abs().max(f64::EPSILON) * 100.0).max(0.0);
                        close_v(&mut tracker.short_side, power);
                    }
                    tracker.peak = Some(price);
                } else {
                    if let Some(peak) = tracker.peak.take() {
                        let power =
                            ((peak - *line) / line.abs().max(f64::EPSILON) * 100.0).max(0.0);
                        close_v(&mut tracker.long_side, power);
                    }
                    tracker.trough = Some(price);
                }

                tracker.was_above = Some(is_above);
            }
            Some(true) => {
                tracker.peak = Some(tracker.peak.unwrap_or(price).max(price));
            }
            Some(false) => {
                tracker.trough = Some(tracker.trough.unwrap_or(price).min(price));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vline_updates_only_from_price_and_line_inputs() {
        let mut vlines = [VLineState::default(); 9];
        let lines = [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0];

        update(&mut vlines, lines, 99.0, 1);
        update(&mut vlines, lines, 101.0, 2);
        update(&mut vlines, lines, 99.5, 3);

        assert_eq!(vlines[4].last_cross_tick, 3);
        assert_eq!(vlines[4].last_cross_dir, -1);
        assert_eq!(vlines[4].long_side.count, 1);
        assert!(vlines[4].long_side.last_power >= 1.0);
    }
}
