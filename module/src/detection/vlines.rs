use crate::state::{MrcCrossingState, VLineState};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MrcCrossEvent {
    pub line_idx: u8,
    pub line: f64,
    pub direction: i8,
    pub tick: u64,
    pub now_ms: u64,
}

fn record_cross(
    vlines: &mut [VLineState; 9],
    crossing: &mut MrcCrossingState,
    line_idx: usize,
    line: f64,
    next_line: Option<f64>,
    price: f64,
    direction: i8,
    tick: u64,
    now_ms: u64,
) -> Option<MrcCrossEvent> {
    // Ignore repeated touches of the current line. A line becomes a new
    // sequence event only after another line has been crossed, which keeps
    // A -> B -> A meaningful.
    if crossing.last_line == Some(line_idx as u8) {
        return None;
    }

    let tracker = &mut vlines[line_idx];
    tracker.last_cross_tick = tick;
    tracker.last_cross_dir = direction;
    tracker.cross_count = tracker.cross_count.saturating_add(1);
    tracker.last_cross_price = line;

    let pulled_back = crossing.sequence_len >= 2
        && crossing.sequence[crossing.sequence_len - 2] == line_idx as u8;
    if pulled_back {
        if direction > 0 {
            crossing.reverted_long_line = crossing.last_line;
        } else {
            crossing.reverted_short_line = crossing.last_line;
        }
        crossing.no_pullback_count = 0;
    } else {
        crossing.no_pullback_count = crossing.no_pullback_count.saturating_add(1);
    }

    if crossing.sequence_len == crossing.sequence.len() {
        crossing.sequence.copy_within(1.., 0);
        crossing.sequence_times_ms.copy_within(1.., 0);
        crossing.sequence_len -= 1;
    }
    crossing.sequence[crossing.sequence_len] = line_idx as u8;
    crossing.sequence_times_ms[crossing.sequence_len] = now_ms;
    crossing.sequence_len += 1;

    crossing.last_line = Some(line_idx as u8);
    crossing.candle_cross_mask |= 1u16 << line_idx;
    crossing.last_direction = direction;
    crossing.movement_start = line;
    crossing.movement_target = next_line.unwrap_or(0.0);
    crossing.movement_high = price;
    crossing.movement_low = price;
    crossing.movement_extreme = price;
    crossing.movement_direction = direction;
    Some(MrcCrossEvent {
        line_idx: line_idx as u8,
        line,
        direction,
        tick,
        now_ms,
    })
}

pub fn update(
    vlines: &mut [VLineState; 9],
    crossing: &mut MrcCrossingState,
    lines: [f64; 9],
    price: f64,
    tick: u64,
    now_ms: u64,
) {
    let _ = update_with_events(vlines, crossing, lines, price, tick, now_ms);
}

pub fn update_with_events(
    vlines: &mut [VLineState; 9],
    crossing: &mut MrcCrossingState,
    lines: [f64; 9],
    price: f64,
    tick: u64,
    now_ms: u64,
) -> Vec<MrcCrossEvent> {
    if !price.is_finite() || price <= 0.0 {
        return Vec::new();
    }

    let previous_price = crossing.last_price.replace(price);
    let Some(previous_price) = previous_price else {
        for (tracker, line) in vlines.iter_mut().zip(lines) {
            if line > 0.0 && line.is_finite() {
                tracker.was_above = Some(price >= line);
            }
        }
        return Vec::new();
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

    let mut events = Vec::new();
    for (line_idx, line) in crossed {
        let direction = if upward { 1 } else { -1 };
        let next_line = if direction > 0 {
            lines.get(line_idx + 1).copied()
        } else {
            line_idx
                .checked_sub(1)
                .and_then(|idx| lines.get(idx).copied())
        };
        if let Some(event) = record_cross(
            vlines,
            crossing,
            line_idx,
            line,
            next_line.filter(|value| value.is_finite() && *value > 0.0),
            price,
            direction,
            tick,
            now_ms,
        ) {
            events.push(event);
        }
    }

    if crossing.movement_direction > 0 {
        crossing.movement_high = crossing.movement_high.max(price);
        crossing.movement_low = crossing.movement_low.min(price);
        crossing.movement_extreme = crossing.movement_extreme.max(price);
    } else if crossing.movement_direction < 0 {
        crossing.movement_high = if crossing.movement_high <= 0.0 {
            price
        } else {
            crossing.movement_high.max(price)
        };
        crossing.movement_low = if crossing.movement_low <= 0.0 {
            price
        } else {
            crossing.movement_low.min(price)
        };
        crossing.movement_extreme = if crossing.movement_extreme <= 0.0 {
            price
        } else {
            crossing.movement_extreme.min(price)
        };
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines() -> [f64; 9] {
        [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0]
    }

    #[test]
    fn realtime_crosses_record_direction_and_repeat_count() {
        let mut crossing = MrcCrossingState::default();

        let mut vlines = [VLineState::default(); 9];
        update(&mut vlines, &mut crossing, lines(), 99.0, 1, 1_000);
        update(&mut vlines, &mut crossing, lines(), 101.0, 2, 2_000);
        update(&mut vlines, &mut crossing, lines(), 99.5, 3, 3_000);

        assert_eq!(vlines[4].last_cross_dir, 1);
        assert_eq!(vlines[4].cross_count, 1);
        assert_eq!(crossing.sequence_len, 1);
        assert_eq!(&crossing.sequence[..1], &[4]);
        assert_eq!(crossing.last_direction, 1);
    }

    #[test]
    fn multiple_lines_are_recorded_in_traversal_order() {
        let mut vlines = [VLineState::default(); 9];
        let mut crossing = MrcCrossingState::default();

        update(&mut vlines, &mut crossing, lines(), 99.0, 1, 1_000);
        update(&mut vlines, &mut crossing, lines(), 105.0, 2, 2_000);

        assert_eq!(&crossing.sequence[..2], &[4, 5]);
        assert_eq!(crossing.no_pullback_count, 2);
    }

    #[test]
    fn pullback_records_reverted_line_and_resets_no_pullback_count() {
        let mut vlines = [VLineState::default(); 9];
        let mut crossing = MrcCrossingState::default();

        update(&mut vlines, &mut crossing, lines(), 99.0, 1, 1_000);
        update(&mut vlines, &mut crossing, lines(), 105.0, 2, 62_000);
        update(&mut vlines, &mut crossing, lines(), 99.0, 3, 92_000);

        assert_eq!(crossing.reverted_short_line, Some(5));
        assert_eq!(crossing.reverted_long_line, None);
        assert_eq!(crossing.no_pullback_count, 0);
        assert_eq!(crossing.movement_direction, -1);
        assert_eq!(&crossing.sequence[..3], &[4, 5, 4]);
    }

    #[test]
    fn pullbacks_track_long_and_short_lines_independently() {
        let mut vlines = [VLineState::default(); 9];
        let mut crossing = MrcCrossingState::default();

        update(&mut vlines, &mut crossing, lines(), 99.0, 1, 1_000);
        update(&mut vlines, &mut crossing, lines(), 105.0, 2, 2_000);
        update(&mut vlines, &mut crossing, lines(), 99.0, 3, 3_000);
        update(&mut vlines, &mut crossing, lines(), 105.0, 4, 4_000);

        assert_eq!(crossing.reverted_short_line, Some(5));
        assert_eq!(crossing.reverted_long_line, Some(4));
    }

    #[test]
    fn repeated_touch_of_current_line_is_not_a_new_sequence_event() {
        let mut vlines = [VLineState::default(); 9];
        let mut crossing = MrcCrossingState::default();

        update(&mut vlines, &mut crossing, lines(), 99.0, 1, 1_000);
        update(&mut vlines, &mut crossing, lines(), 101.0, 2, 2_000);
        update(&mut vlines, &mut crossing, lines(), 99.5, 3, 3_000);
        update(&mut vlines, &mut crossing, lines(), 100.5, 4, 4_000);

        assert_eq!(&crossing.sequence[..1], &[4]);
        assert_eq!(vlines[4].cross_count, 1);
    }

    #[test]
    fn movement_tracks_favorable_extreme_and_resets_on_next_cross() {
        let mut vlines = [VLineState::default(); 9];
        let mut crossing = MrcCrossingState::default();

        update(&mut vlines, &mut crossing, lines(), 99.0, 1, 1_000);
        update(&mut vlines, &mut crossing, lines(), 101.0, 2, 2_000);
        update(&mut vlines, &mut crossing, lines(), 102.0, 3, 3_000);
        update(&mut vlines, &mut crossing, lines(), 99.0, 4, 4_000);
        assert_eq!(crossing.movement_start, 100.0);
        assert_eq!(crossing.movement_target, 104.0);
        assert_eq!(crossing.movement_high, 102.0);
        assert_eq!(crossing.movement_low, 99.0);
        assert_eq!(crossing.movement_extreme, 102.0);

        update(&mut vlines, &mut crossing, lines(), 105.0, 5, 5_000);
        assert_eq!(crossing.movement_start, 104.0);
        assert_eq!(crossing.movement_extreme, 105.0);
    }
}
