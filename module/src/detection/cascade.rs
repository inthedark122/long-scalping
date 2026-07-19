use crate::model::{Candle, TimeframeSnapshot};
use crate::state::{CascadeMovement, CascadeState};

const CASCADE_SAMPLE_WINDOW: usize = 30;
const CASCADE_TP_PERCENTILE: f64 = 0.90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementPhase {
    Starting,
    Building,
    Mature,
    CounterMove,
    Reversal,
    Unknown,
}

impl Default for MovementPhase {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CascadeSignal {
    pub dir: i8,
    pub prev_dir: i8,
    pub strength: f64,
    pub stretched: bool,
    pub phase: MovementPhase,
    pub completed_size: f64,
    pub average_size: f64,
    pub samples: u32,
    pub break_price: f64,
}

pub fn average_size_percent(average_size: f64, price: f64) -> Option<f64> {
    if !average_size.is_finite() || average_size <= 0.0 || !price.is_finite() || price <= 0.0 {
        return None;
    }

    let percent = average_size / price * 100.0;
    (percent.is_finite() && percent > 0.0).then_some(percent)
}

fn update_block(memory: &mut CascadeState, dir: i8, high: f64, low: f64, close: f64) -> bool {
    if memory.cas_dir == 0 {
        memory.cas_dir = dir;
        memory.cas_high = high;
        memory.cas_low = low;
        memory.cas_candles = 1;
        memory.dir = dir;
        memory.run_extreme = if dir >= 0 { high } else { low };
        memory.run_size = high - low;
        memory.last_close = close;
        return false;
    }

    if dir == memory.cas_dir {
        if high > memory.cas_high {
            memory.cas_high = high;
        }
        if low < memory.cas_low {
            memory.cas_low = low;
        }
        memory.cas_candles = memory.cas_candles.saturating_add(1);
        return false;
    }

    let completed_size = (memory.cas_high - memory.cas_low).abs();
    let threshold = if memory.avg_size > 0.0 {
        memory.avg_size * 0.40
    } else {
        f64::MAX
    };
    let is_counter = memory.dir != memory.cas_dir;
    let absorb = is_counter && completed_size < threshold;

    if absorb {
        let break_price = if memory.cas_dir == -1 {
            memory.cas_low
        } else {
            memory.cas_high
        };
        if memory.break_n >= memory.breaks.len() {
            memory.breaks.rotate_left(1);
            if let Some(last) = memory.breaks.last_mut() {
                *last = break_price;
            }
        } else {
            memory.breaks[memory.break_n] = break_price;
            memory.break_n += 1;
        }
        memory.cas_high = memory.cas_high.max(high);
        memory.cas_low = memory.cas_low.min(low);
        if dir == memory.cas_dir {
            memory.cas_candles = memory.cas_candles.saturating_add(1);
        }
        false
    } else {
        let cas_size = (memory.cas_high - memory.cas_low).abs();
        if cas_size > 0.0 {
            memory.movement_history[memory.movement_history_pos] = CascadeMovement {
                high: memory.cas_high,
                low: memory.cas_low,
            };
            memory.movement_history_pos =
                (memory.movement_history_pos + 1) % memory.movement_history.len();
            memory.movement_history_len = memory
                .movement_history_len
                .saturating_add(1)
                .min(memory.movement_history.len());
            memory.sizes[memory.sizes_pos] = cas_size;
            memory.sizes_pos = (memory.sizes_pos + 1) % memory.sizes.len();
            if memory.sizes_len < memory.sizes.len() {
                memory.sizes_len += 1;
            }
            memory.samples = memory.sizes_len as u32;
            memory.avg_size = rolling_average(memory);
        }
        memory.last_break = memory.last_close;
        memory.cas_dir = dir;
        memory.cas_high = high;
        memory.cas_low = low;
        memory.cas_candles = 1;
        memory.break_n = 0;
        memory.breaks = [0.0; 5];
        true
    }
}

fn rolling_average(memory: &CascadeState) -> f64 {
    if memory.sizes_len == 0 {
        return 0.0;
    }
    let len = memory.sizes_len.min(CASCADE_SAMPLE_WINDOW);
    let mut total = 0.0;
    if memory.sizes_len < memory.sizes.len() {
        for idx in 0..len {
            total += memory.sizes[idx];
        }
    } else {
        let start = memory.sizes_pos % memory.sizes.len();
        for idx in 0..len {
            total += memory.sizes[(start + idx) % memory.sizes.len()];
        }
    }
    total / len as f64
}

#[allow(dead_code)]
pub fn sample_window(memory: &CascadeState) -> Vec<f64> {
    if memory.sizes_len == 0 {
        return Vec::new();
    }
    let len = memory.sizes_len.min(CASCADE_SAMPLE_WINDOW);
    let mut out = Vec::with_capacity(len);
    if memory.sizes_len < memory.sizes.len() {
        out.extend_from_slice(&memory.sizes[..len]);
    } else {
        let start = memory.sizes_pos % memory.sizes.len();
        for idx in 0..len {
            out.push(memory.sizes[(start + idx) % memory.sizes.len()]);
        }
    }
    out
}

pub fn percentile_window(memory: &CascadeState, percentile: f64) -> Option<f64> {
    if memory.sizes_len == 0 {
        return None;
    }

    let mut values = sample_window(memory);
    if values.is_empty() {
        return None;
    }

    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pct = percentile.clamp(0.0, 1.0);
    let rank = ((values.len() as f64) * pct).ceil() as usize;
    let idx = rank.saturating_sub(1).min(values.len().saturating_sub(1));
    values.get(idx).copied()
}

pub fn cascade_take_profit_distance(memory: &CascadeState) -> Option<f64> {
    percentile_window(memory, CASCADE_TP_PERCENTILE)
}

pub fn update_from_candle(memory: &mut CascadeState, candle: &Candle) -> CascadeSignal {
    let high = candle.high;
    let low = candle.low;
    let close = candle.close;
    if high <= 0.0 || low <= 0.0 || close <= 0.0 {
        return CascadeSignal::default();
    }

    let raw_dir = if memory.last_close > 0.0 {
        if close > memory.last_close {
            1
        } else if close < memory.last_close {
            -1
        } else {
            0
        }
    } else if candle.close > candle.open {
        1
    } else if candle.close < candle.open {
        -1
    } else {
        0
    };

    let dir = if raw_dir == 0 { memory.dir } else { raw_dir };

    let phase_hint = if dir == 0 {
        false
    } else {
        update_block(memory, dir, high, low, close)
    };

    let prev_dir = memory.dir;
    memory.dir = if dir == 0 { memory.dir } else { dir };
    memory.last_close = close;
    memory.run_extreme = if memory.dir >= 0 {
        if memory.run_extreme == 0.0 {
            high
        } else {
            memory.run_extreme.max(high)
        }
    } else if memory.run_extreme == 0.0 {
        low
    } else {
        memory.run_extreme.min(low)
    };
    memory.run_size = (memory.cas_high - memory.cas_low).abs();

    let active_size = (memory.cas_high - memory.cas_low).abs();
    let average_size = memory.avg_size;
    let strength = if average_size > 0.0 {
        active_size / average_size
    } else if active_size > 0.0 {
        1.0
    } else {
        0.0
    };
    let stretched = strength >= 1.5;
    let phase = if phase_hint {
        MovementPhase::Reversal
    } else if memory.cas_candles <= 1 {
        MovementPhase::Starting
    } else if strength >= 1.2 {
        MovementPhase::Mature
    } else if memory.break_n > 0 {
        MovementPhase::CounterMove
    } else {
        MovementPhase::Building
    };

    CascadeSignal {
        dir: memory.dir,
        prev_dir,
        strength,
        stretched,
        phase,
        completed_size: active_size,
        average_size,
        samples: memory.samples,
        break_price: memory.breaks[memory
            .break_n
            .saturating_sub(1)
            .min(memory.breaks.len() - 1)],
    }
}

pub fn movement_history(memory: &CascadeState) -> Vec<CascadeMovement> {
    let len = memory
        .movement_history_len
        .min(memory.movement_history.len());
    if len == 0 {
        return Vec::new();
    }

    let start = if memory.movement_history_len < memory.movement_history.len() {
        0
    } else {
        memory.movement_history_pos
    };
    (0..len)
        .map(|offset| memory.movement_history[(start + offset) % memory.movement_history.len()])
        .collect()
}

pub fn preview_from_candle(memory: &CascadeState, candle: &Candle) -> CascadeSignal {
    let mut preview = *memory;
    update_from_candle(&mut preview, candle)
}

pub fn update(memory: &mut CascadeState, tf: &TimeframeSnapshot) -> CascadeSignal {
    update_from_candle(memory, &tf.candle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Candle;

    fn candle(open: f64, high: f64, low: f64, close: f64) -> Candle {
        Candle {
            open,
            high,
            low,
            close,
        }
    }

    #[test]
    fn movement_starts_then_builds() {
        let mut state = CascadeState::default();
        let first = update_from_candle(&mut state, &candle(100.0, 101.0, 99.5, 100.8));
        assert_eq!(first.phase, MovementPhase::Starting);

        let second = update_from_candle(&mut state, &candle(100.8, 103.0, 100.4, 102.5));
        assert!(matches!(
            second.phase,
            MovementPhase::Building | MovementPhase::Mature
        ));
        assert!(second.dir > 0);
    }

    #[test]
    fn counter_move_and_reversal_are_flagged() {
        let mut state = CascadeState::default();
        let _ = update_from_candle(&mut state, &candle(100.0, 101.0, 99.5, 100.8));
        let _ = update_from_candle(&mut state, &candle(100.8, 103.0, 100.4, 102.5));
        let reversal = update_from_candle(&mut state, &candle(102.5, 102.8, 99.2, 99.8));
        assert_eq!(reversal.phase, MovementPhase::Reversal);
        assert!(reversal.completed_size > 0.0);

        let history = movement_history(&state);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].high, 103.0);
        assert_eq!(history[0].low, 99.5);
    }

    #[test]
    fn rolling_average_tracks_the_last_ten_completed_blocks() {
        let mut state = CascadeState::default();
        for size in 1..=10 {
            state.sizes[state.sizes_pos] = size as f64;
            state.sizes_pos = (state.sizes_pos + 1) % state.sizes.len();
            state.sizes_len += 1;
        }
        state.avg_size = rolling_average(&state);
        assert_eq!(
            sample_window(&state),
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]
        );
        assert!((state.avg_size - 5.5).abs() < 1e-9);

        state.sizes[state.sizes_pos] = 100.0;
        state.sizes_pos = (state.sizes_pos + 1) % state.sizes.len();
        state.sizes_len += 1;
        state.sizes[state.sizes_pos] = 110.0;
        state.sizes_pos = (state.sizes_pos + 1) % state.sizes.len();
        state.sizes_len += 1;
        state.avg_size = rolling_average(&state);

        assert_eq!(
            sample_window(&state),
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 100.0, 110.0]
        );
        assert!((state.avg_size - 22.083333333333332).abs() < 1e-9);
    }

    #[test]
    fn new_block_starts_from_current_candle_after_reversal() {
        let mut state = CascadeState::default();
        let _ = update_from_candle(&mut state, &candle(100.0, 101.0, 99.0, 100.5));
        let _ = update_from_candle(&mut state, &candle(100.5, 106.0, 100.0, 105.5));
        let _ = update_from_candle(&mut state, &candle(105.5, 105.8, 104.8, 104.9));

        assert!(state.cas_high <= 105.8 + f64::EPSILON);
        assert!(state.cas_low >= 104.8 - f64::EPSILON);
        assert!((state.cas_high - state.cas_low).abs() <= 1.1);
    }
}
