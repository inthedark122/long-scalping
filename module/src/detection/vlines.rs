use crate::state::{MrcCrossingState, VLineState, VSideState};

fn close_v(side: &mut VSideState, power: f64) {
    side.prev_power = side.last_power;
    side.last_power = power;
    side.count = side.count.saturating_add(1);
}

fn record_cross(
    vlines: &mut [VLineState; 9],
    crossing: &mut MrcCrossingState,
    line_idx: usize,
    line: f64,
    price: f64,
    direction: i8,
    tick: u64,
) {
    let tracker = &mut vlines[line_idx];
    tracker.last_cross_tick = tick;
    tracker.last_cross_dir = direction;
    tracker.cross_count = tracker.cross_count.saturating_add(1);
    tracker.last_cross_price = line;

    let pulled_back = crossing.last_line == Some(line_idx as u8);
    if pulled_back {
        crossing.reverted_line = crossing.last_line;
        crossing.no_pullback_count = 0;
    } else {
        crossing.no_pullback_count = crossing.no_pullback_count.saturating_add(1);
    }

    if crossing.sequence_len == crossing.sequence.len() {
        crossing.sequence.copy_within(1.., 0);
        crossing.sequence_len -= 1;
    }
    crossing.sequence[crossing.sequence_len] = line_idx as u8;
    crossing.sequence_len += 1;

    crossing.last_line = Some(line_idx as u8);
    crossing.last_direction = direction;
    crossing.highest_line = Some(
        crossing
            .highest_line
            .map_or(line_idx as u8, |value| value.max(line_idx as u8)),
    );
    crossing.lowest_line = Some(
        crossing
            .lowest_line
            .map_or(line_idx as u8, |value| value.min(line_idx as u8)),
    );
    crossing.movement_start = line;
    crossing.movement_extreme = price;
    crossing.movement_direction = direction;
}

pub fn update(
    vlines: &mut [VLineState; 9],
    crossing: &mut MrcCrossingState,
    lines: [f64; 9],
    price: f64,
    tick: u64,
) {
    if !price.is_finite() || price <= 0.0 {
        return;
    }

    let previous_price = crossing.last_price.replace(price);
    let Some(previous_price) = previous_price else {
        for (tracker, line) in vlines.iter_mut().zip(lines) {
            if line > 0.0 && line.is_finite() {
                tracker.was_above = Some(price >= line);
            }
        }
        return;
    };

    let upward = price > previous_price;
    let downward = price < previous_price;
    let mut crossed = Vec::new();
    for (idx, line) in lines.iter().copied().enumerate() {
        if !line.is_finite() || line <= 0.0 {
            continue;
        }
        let tracker = &mut vlines[idx];
        let is_above = price >= line;
        if tracker.was_above != Some(is_above) {
            let crossed_now = (upward && previous_price < line && price >= line)
                || (downward && previous_price >= line && price < line);
            if crossed_now {
                crossed.push((idx, line));
            }
        }
        tracker.was_above = Some(is_above);
    }

    crossed.sort_by(|(left_idx, left_line), (right_idx, right_line)| {
        if upward {
            left_line
                .partial_cmp(right_line)
                .unwrap_or(left_idx.cmp(right_idx))
        } else {
            right_line
                .partial_cmp(left_line)
                .unwrap_or(right_idx.cmp(left_idx))
        }
    });

    for (line_idx, line) in crossed {
        let direction = if upward { 1 } else { -1 };
        record_cross(vlines, crossing, line_idx, line, price, direction, tick);
    }

    if crossing.movement_direction > 0 {
        crossing.movement_extreme = crossing.movement_extreme.max(price);
    } else if crossing.movement_direction < 0 {
        crossing.movement_extreme = if crossing.movement_extreme <= 0.0 {
            price
        } else {
            crossing.movement_extreme.min(price)
        };
    }

    // Preserve the existing excursion statistics for compatibility with the
    // current diagnostics; the new crossing state above is event-based.
    for (idx, line) in lines.iter().copied().enumerate() {
        if line <= 0.0 || !line.is_finite() {
            continue;
        }
        let tracker = &mut vlines[idx];
        if price >= line {
            if let Some(trough) = tracker.trough.take() {
                let power = ((line - trough) / line.abs().max(f64::EPSILON) * 100.0).max(0.0);
                close_v(&mut tracker.short_side, power);
            }
            tracker.peak = Some(tracker.peak.unwrap_or(price).max(price));
        } else {
            if let Some(peak) = tracker.peak.take() {
                let power = ((peak - line) / line.abs().max(f64::EPSILON) * 100.0).max(0.0);
                close_v(&mut tracker.long_side, power);
            }
            tracker.trough = Some(tracker.trough.unwrap_or(price).min(price));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines() -> [f64; 9] {
        [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0]
    }

    #[test]
    fn realtime_crosses_record_direction_and_repeat_count() {
        let mut vlines = [VLineState::default(); 9];
        let mut crossing = MrcCrossingState::default();

        update(&mut vlines, &mut crossing, lines(), 99.0, 1);
        update(&mut vlines, &mut crossing, lines(), 101.0, 2);
        update(&mut vlines, &mut crossing, lines(), 99.5, 3);

        assert_eq!(vlines[4].last_cross_dir, -1);
        assert_eq!(vlines[4].cross_count, 2);
        assert_eq!(crossing.sequence_len, 2);
        assert_eq!(&crossing.sequence[..2], &[4, 4]);
        assert_eq!(crossing.last_direction, -1);
    }

    #[test]
    fn multiple_lines_are_recorded_in_traversal_order() {
        let mut vlines = [VLineState::default(); 9];
        let mut crossing = MrcCrossingState::default();

        update(&mut vlines, &mut crossing, lines(), 99.0, 1);
        update(&mut vlines, &mut crossing, lines(), 105.0, 2);

        assert_eq!(&crossing.sequence[..2], &[4, 5]);
        assert_eq!(crossing.no_pullback_count, 2);
        assert_eq!(crossing.highest_line, Some(5));
        assert_eq!(crossing.lowest_line, Some(4));
    }

    #[test]
    fn pullback_records_reverted_line_and_resets_no_pullback_count() {
        let mut vlines = [VLineState::default(); 9];
        let mut crossing = MrcCrossingState::default();

        update(&mut vlines, &mut crossing, lines(), 99.0, 1);
        update(&mut vlines, &mut crossing, lines(), 105.0, 2);
        update(&mut vlines, &mut crossing, lines(), 103.0, 3);

        assert_eq!(crossing.reverted_line, Some(5));
        assert_eq!(crossing.no_pullback_count, 0);
        assert_eq!(crossing.movement_direction, -1);
    }

    #[test]
    fn movement_tracks_favorable_extreme_and_resets_on_next_cross() {
        let mut vlines = [VLineState::default(); 9];
        let mut crossing = MrcCrossingState::default();

        update(&mut vlines, &mut crossing, lines(), 99.0, 1);
        update(&mut vlines, &mut crossing, lines(), 101.0, 2);
        update(&mut vlines, &mut crossing, lines(), 102.0, 3);
        assert_eq!(crossing.movement_start, 100.0);
        assert_eq!(crossing.movement_extreme, 102.0);

        update(&mut vlines, &mut crossing, lines(), 105.0, 4);
        assert_eq!(crossing.movement_start, 104.0);
        assert_eq!(crossing.movement_extreme, 105.0);
    }
}
