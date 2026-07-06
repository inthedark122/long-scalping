use crate::model::{Candle, TimeframeSnapshot};
use crate::state::CascadeState;

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
    pub strength: f64,
    pub stretched: bool,
    pub phase: MovementPhase,
    pub completed_size: f64,
    pub average_size: f64,
    pub break_price: f64,
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
            memory.sizes[memory.sizes_pos] = cas_size;
            memory.sizes_pos = (memory.sizes_pos + 1) % memory.sizes.len();
            if memory.sizes_len < memory.sizes.len() {
                memory.sizes_len += 1;
            }
            memory.samples = memory.sizes_len as u32;
            memory.avg_size = memory.sizes[..memory.sizes_len].iter().sum::<f64>()
                / memory.sizes_len as f64;
        }
        memory.last_break = memory.last_close;
        memory.cas_dir = dir;
        memory.cas_high = high.max(memory.cas_high);
        memory.cas_low = low.min(memory.cas_low);
        memory.cas_candles = 1;
        memory.break_n = 0;
        memory.breaks = [0.0; 5];
        true
    }
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

    let dir = if raw_dir == 0 {
        memory.dir
    } else {
        raw_dir
    };

    let phase_hint = if dir == 0 {
        false
    } else {
        update_block(memory, dir, high, low, close)
    };

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
        strength,
        stretched,
        phase,
        completed_size: active_size,
        average_size,
        break_price: memory.breaks[memory.break_n.saturating_sub(1).min(memory.breaks.len() - 1)],
    }
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
    use crate::model::{Candle, MrcBands};

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
        assert!(matches!(second.phase, MovementPhase::Building | MovementPhase::Mature));
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
    }
}
