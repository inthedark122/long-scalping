//! Stateful candle cascade detector used as movement-speed context.
//!
//! The cascade does not choose a side by itself. It measures the active
//! movement and supplies direction, speed, and stretch to the MRC line plan.

use crate::model::{Candle, TimeframeSnapshot};

const CASCADE_SAMPLE_WINDOW: usize = 30;
const CASCADE_TP_PERCENTILE: f64 = 0.90;
const MIN_SAMPLES_FOR_ABSORPTION: u32 = 10;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CascadeMovement {
    pub high: f64,
    pub low: f64,
    pub dir: i8,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CascadeState {
    pub last_close: f64,
    pub dir: i8,
    pub run_extreme: f64,
    pub run_size: f64,
    pub avg_size: f64,
    pub samples: u32,
    pub last_break: f64,
    pub cas_dir: i8,
    pub cas_high: f64,
    pub cas_low: f64,
    pub cas_candles: u32,
    pub sizes: [f64; CASCADE_SAMPLE_WINDOW],
    pub sizes_pos: usize,
    pub sizes_len: usize,
    pub breaks: [f64; 5],
    pub break_n: usize,
    pub movement_history: [CascadeMovement; CASCADE_SAMPLE_WINDOW],
    pub movement_history_pos: usize,
    pub movement_history_len: usize,
    pub movement_start_ms: u64,
    pub last_time_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

impl MovementPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Building => "building",
            Self::Mature => "mature",
            Self::CounterMove => "counter_move",
            Self::Reversal => "reversal",
            Self::Unknown => "unknown",
        }
    }
}

/// Why the cascade would or would not authorize a strict directional entry.
///
/// `NotConfigured` is retained for research/unit-test callers that do not
/// provide live cascade context. Staircase execution records this state as
/// context; the structural stairs gate remains the live entry authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CascadeTradeState {
    NotConfigured,
    Ready,
    DirectionMismatch,
    CounterMove,
    Reversal,
    SpeedUnavailable,
    PotentialTooSmall,
    AdverseMoveTooLarge,
}

impl CascadeTradeState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::Ready => "ready",
            Self::DirectionMismatch => "direction_mismatch",
            Self::CounterMove => "counter_move",
            Self::Reversal => "reversal",
            Self::SpeedUnavailable => "speed_unavailable",
            Self::PotentialTooSmall => "potential_too_small",
            Self::AdverseMoveTooLarge => "adverse_move_too_large",
        }
    }

    pub const fn allows_entry(self) -> bool {
        matches!(self, Self::NotConfigured | Self::Ready)
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
    /// Current close position inside the active movement, as a ratio.
    /// Bearish movement is measured from the high downwards.
    pub movement_position: f64,
    pub average_size: f64,
    pub samples: u32,
    pub break_price: f64,
    pub elapsed_ms: u64,
    pub speed_pct_per_min: Option<f64>,
    /// Maximum continuation range available from the active movement or the
    /// recent same-direction movement envelope, expressed as a percentage of
    /// the current price.
    pub potential_move_pct: Option<f64>,
    /// Largest recent completed movement in the opposite direction, expressed
    /// as a positive percentage. This is the expected adverse excursion used
    /// to validate a stop and place staged entries.
    pub adverse_move_pct: Option<f64>,
}

impl CascadeSignal {
    pub fn trade_state(
        self,
        enabled: bool,
        direction: i8,
        target_move_pct: f64,
        stop_move_pct: f64,
    ) -> CascadeTradeState {
        if !enabled {
            return CascadeTradeState::NotConfigured;
        }
        if !matches!(direction, 1 | -1) || self.dir != direction {
            return CascadeTradeState::DirectionMismatch;
        }
        match self.phase {
            MovementPhase::CounterMove => return CascadeTradeState::CounterMove,
            MovementPhase::Reversal => return CascadeTradeState::Reversal,
            MovementPhase::Unknown => return CascadeTradeState::DirectionMismatch,
            MovementPhase::Starting | MovementPhase::Building | MovementPhase::Mature => {}
        }
        if !self
            .speed_pct_per_min
            .is_some_and(|speed| speed.is_finite() && speed > 0.0)
        {
            return CascadeTradeState::SpeedUnavailable;
        }
        if target_move_pct.is_finite()
            && target_move_pct > 0.0
            && !self
                .potential_move_pct
                .is_some_and(|potential| potential + f64::EPSILON >= target_move_pct)
        {
            return CascadeTradeState::PotentialTooSmall;
        }
        if stop_move_pct.is_finite()
            && stop_move_pct > 0.0
            && self
                .adverse_move_pct
                .is_some_and(|adverse| adverse > stop_move_pct + f64::EPSILON)
        {
            return CascadeTradeState::AdverseMoveTooLarge;
        }
        CascadeTradeState::Ready
    }

    pub fn multi_order_ready(self) -> bool {
        self.dir != 0
            && self
                .speed_pct_per_min
                .is_some_and(|speed| speed.is_finite() && speed > 0.0)
            && self
                .potential_move_pct
                .is_some_and(|value| value.is_finite() && value > 0.0)
    }
}

pub fn average_size_percent(average_size: f64, price: f64) -> Option<f64> {
    if !average_size.is_finite() || average_size <= 0.0 || !price.is_finite() || price <= 0.0 {
        return None;
    }

    let percent = average_size / price * 100.0;
    (percent.is_finite() && percent > 0.0).then_some(percent)
}

fn update_block(
    memory: &mut CascadeState,
    dir: i8,
    high: f64,
    low: f64,
    close: f64,
    now_ms: u64,
) -> bool {
    if memory.cas_dir == 0 {
        memory.cas_dir = dir;
        memory.cas_high = high;
        memory.cas_low = low;
        memory.cas_candles = 1;
        memory.dir = dir;
        memory.run_extreme = if dir >= 0 { high } else { low };
        memory.run_size = high - low;
        memory.last_close = close;
        memory.movement_start_ms = now_ms;
        return false;
    }

    if dir == memory.cas_dir {
        memory.cas_high = memory.cas_high.max(high);
        memory.cas_low = memory.cas_low.min(low);
        memory.cas_candles = memory.cas_candles.saturating_add(1);
        return false;
    }

    let completed_size = (memory.cas_high - memory.cas_low).abs();
    let threshold = if memory.samples >= MIN_SAMPLES_FOR_ABSORPTION && memory.avg_size > 0.0 {
        memory.avg_size * 0.40
    } else {
        0.0
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
        false
    } else {
        let cas_size = (memory.cas_high - memory.cas_low).abs();
        if cas_size > 0.0 {
            memory.movement_history[memory.movement_history_pos] = CascadeMovement {
                high: memory.cas_high,
                low: memory.cas_low,
                dir: memory.cas_dir,
            };
            memory.movement_history_pos =
                (memory.movement_history_pos + 1) % memory.movement_history.len();
            memory.movement_history_len = memory
                .movement_history_len
                .saturating_add(1)
                .min(memory.movement_history.len());
            memory.sizes[memory.sizes_pos] = cas_size;
            memory.sizes_pos = (memory.sizes_pos + 1) % memory.sizes.len();
            memory.sizes_len = (memory.sizes_len + 1).min(memory.sizes.len());
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
        memory.movement_start_ms = now_ms;
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
    update_from_candle_at(memory, candle, 0)
}

pub fn update_from_candle_at(
    memory: &mut CascadeState,
    candle: &Candle,
    now_ms: u64,
) -> CascadeSignal {
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
        update_block(memory, dir, high, low, close, now_ms)
    };

    let prev_dir = memory.dir;
    memory.dir = if dir == 0 { memory.dir } else { dir };
    memory.last_close = close;
    if now_ms > 0 {
        memory.last_time_ms = now_ms;
    }
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
    let movement_position = if active_size > 0.0 {
        let raw_position = if memory.dir >= 0 {
            (close - memory.cas_low) / active_size
        } else {
            (memory.cas_high - close) / active_size
        };
        raw_position.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let average_size = memory.avg_size;
    let strength = if average_size > 0.0 {
        active_size / average_size
    } else if active_size > 0.0 {
        1.0
    } else {
        0.0
    };
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

    let elapsed_ms = if now_ms > 0 && memory.movement_start_ms > 0 {
        now_ms.saturating_sub(memory.movement_start_ms)
    } else {
        0
    };
    let speed_pct_per_min = (elapsed_ms > 0 && close > 0.0 && active_size > 0.0)
        .then(|| active_size / close * 100.0 / (elapsed_ms as f64 / 60_000.0));
    let potential_move_pct = if close > 0.0 && active_size > 0.0 {
        let recent_same_direction =
            recent_directional_max_size(memory, 5, memory.dir).unwrap_or(0.0);
        Some(active_size.max(recent_same_direction) / close * 100.0)
    } else {
        None
    };
    let adverse_move_pct = if close > 0.0 && matches!(memory.dir, 1 | -1) {
        recent_directional_max_size(memory, 5, -memory.dir).map(|size| size / close * 100.0)
    } else {
        None
    };

    CascadeSignal {
        dir: memory.dir,
        prev_dir,
        strength,
        stretched: strength >= 1.5,
        phase,
        completed_size: active_size,
        movement_position,
        average_size,
        samples: memory.samples,
        break_price: memory.breaks[memory
            .break_n
            .saturating_sub(1)
            .min(memory.breaks.len() - 1)],
        elapsed_ms,
        speed_pct_per_min,
        potential_move_pct,
        adverse_move_pct,
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

/// Largest completed movement in `direction` among the latest `count`
/// completed movements. Directional filtering is intentional: the same
/// cascade supplies continuation potential and adverse excursion separately.
pub fn recent_directional_max_size(
    memory: &CascadeState,
    count: usize,
    direction: i8,
) -> Option<f64> {
    if count == 0 || !matches!(direction, 1 | -1) {
        return None;
    }
    let history = movement_history(memory);
    history
        .iter()
        .rev()
        .filter(|movement| movement.dir == direction)
        .take(count)
        .map(|movement| (movement.high - movement.low).abs())
        .filter(|size| size.is_finite() && *size > 0.0)
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
}

pub fn recent_directional_max_size_percent(
    memory: &CascadeState,
    count: usize,
    direction: i8,
    price: f64,
) -> Option<f64> {
    (price.is_finite() && price > 0.0)
        .then_some(recent_directional_max_size(memory, count, direction)? / price * 100.0)
}

/// Price envelope covered by the latest completed movements.
pub fn recent_movement_distance(memory: &CascadeState, count: usize) -> Option<f64> {
    if count == 0 {
        return None;
    }
    let history = movement_history(memory);
    let recent = history.iter().rev().take(count);
    let mut high = f64::NEG_INFINITY;
    let mut low = f64::INFINITY;
    let mut found = false;
    for movement in recent {
        high = high.max(movement.high);
        low = low.min(movement.low);
        found = true;
    }
    found.then_some((high - low).abs())
}

pub fn recent_movement_distance_percent(
    memory: &CascadeState,
    count: usize,
    price: f64,
) -> Option<f64> {
    let distance = recent_movement_distance(memory, count)?;
    (price.is_finite() && price > 0.0).then_some(distance / price * 100.0)
}

pub fn recent_average_size(memory: &CascadeState, count: usize) -> Option<f64> {
    if count == 0 {
        return None;
    }
    let history = movement_history(memory);
    let recent = history.iter().rev().take(count).collect::<Vec<_>>();
    (recent.len() == count).then_some(
        recent
            .iter()
            .map(|movement| (movement.high - movement.low).abs())
            .sum::<f64>()
            / count as f64,
    )
}

pub fn recent_average_size_percent(memory: &CascadeState, count: usize, price: f64) -> Option<f64> {
    (price.is_finite() && price > 0.0)
        .then_some(recent_average_size(memory, count)? / price * 100.0)
}

pub fn recent_total_size(memory: &CascadeState, count: usize) -> Option<f64> {
    if count == 0 {
        return None;
    }
    let history = movement_history(memory);
    let recent = history.iter().rev().take(count).collect::<Vec<_>>();
    (recent.len() == count).then_some(
        recent
            .iter()
            .map(|movement| (movement.high - movement.low).abs())
            .sum(),
    )
}

pub fn recent_total_size_percent(memory: &CascadeState, count: usize, price: f64) -> Option<f64> {
    (price.is_finite() && price > 0.0).then_some(recent_total_size(memory, count)? / price * 100.0)
}

pub fn recent_signed_total_size(memory: &CascadeState, count: usize) -> Option<f64> {
    if count == 0 {
        return None;
    }
    let history = movement_history(memory);
    let recent = history.iter().rev().take(count).collect::<Vec<_>>();
    (recent.len() == count).then_some(
        recent
            .iter()
            .map(|movement| {
                let size = (movement.high - movement.low).abs();
                if movement.dir < 0 {
                    -size
                } else {
                    size
                }
            })
            .sum(),
    )
}

pub fn recent_signed_total_size_percent(
    memory: &CascadeState,
    count: usize,
    price: f64,
) -> Option<f64> {
    (price.is_finite() && price > 0.0)
        .then_some(recent_signed_total_size(memory, count)? / price * 100.0)
}

pub fn preview_from_candle(memory: &CascadeState, candle: &Candle) -> CascadeSignal {
    let mut preview = *memory;
    update_from_candle(&mut preview, candle)
}

pub fn update(memory: &mut CascadeState, tf: &TimeframeSnapshot) -> CascadeSignal {
    update_from_candle(memory, &tf.candle)
}

pub fn update_at(memory: &mut CascadeState, tf: &TimeframeSnapshot, now_ms: u64) -> CascadeSignal {
    update_from_candle_at(memory, &tf.candle, now_ms)
}
