use crate::model::TimeframeSnapshot;
use crate::state::CascadeState;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CascadeSignal {
    pub dir: i8,
    pub strength: f64,
    pub stretched: bool,
}

pub fn update(memory: &mut CascadeState, tf: &TimeframeSnapshot) -> CascadeSignal {
    let close = tf.candle.close;
    if close <= 0.0 {
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
    } else if tf.candle.close > tf.candle.open {
        1
    } else if tf.candle.close < tf.candle.open {
        -1
    } else {
        0
    };

    let agreed_dir = if tf.dp_dir == tf.zz_dir && tf.dp_dir != 0 {
        tf.dp_dir
    } else {
        raw_dir
    };

    let step = if memory.last_close > 0.0 {
        (close - memory.last_close).abs()
    } else {
        (tf.candle.close - tf.candle.open).abs()
    };

    if memory.dir == 0 || agreed_dir == memory.dir || agreed_dir == 0 {
        memory.dir = if agreed_dir == 0 {
            memory.dir
        } else {
            agreed_dir
        };
        memory.run_extreme = if memory.run_extreme == 0.0 {
            close
        } else if memory.dir >= 0 {
            memory.run_extreme.max(close)
        } else {
            memory.run_extreme.min(close).max(f64::MIN)
        };
        memory.run_size += step;
    } else {
        if memory.run_size > 0.0 {
            memory.avg_size = if memory.samples == 0 {
                memory.run_size
            } else {
                (memory.avg_size * memory.samples as f64 + memory.run_size)
                    / (memory.samples as f64 + 1.0)
            };
            memory.samples += 1;
            memory.last_break = memory.last_close;
        }
        memory.dir = agreed_dir;
        memory.run_size = step;
        memory.run_extreme = close;
    }

    memory.last_close = close;
    let strength = if memory.avg_size > 0.0 {
        memory.run_size / memory.avg_size
    } else if step > 0.0 {
        1.0
    } else {
        0.0
    };

    CascadeSignal {
        dir: memory.dir,
        strength,
        stretched: strength >= 1.5,
    }
}
