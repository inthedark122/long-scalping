//! Incremental MRC touch and pressure measurements.
//!
//! Discrete V-line crossings and candle cascade statistics already exist in
//! neighboring modules. This module fills the gap between them: it observes
//! every realtime price update, keeps time-decayed per-line state, and
//! reports debounce, side pressure, price speed/angle, and MRC-band geometry.
//! It does not create orders or replace the structural pattern classifier.

use std::f64::consts::PI;

pub const MRC_LINE_COUNT: usize = 9;

const EMA_TAU_MS: f64 = 90_000.0;
const DEBOUNCE_TAU_MS: f64 = 45_000.0;
const NEAR_LINE_PCT: f64 = 0.25;
const SPEED_SCALE_PCT_PER_MIN: f64 = 0.50;
const ANGLE_SLOPE_SCALE_DEG_PER_MIN: f64 = 20.0;
const WIDTH_SLOPE_SCALE_PCT_PER_MIN: f64 = 0.25;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MrcLineTouchEma {
    /// Time-decayed side of price relative to this line: -1 below, +1 above.
    pub side_ema: f64,
    /// Time-decayed signed distance from this line in percent.
    pub distance_ema_pct: f64,
    /// Time-decayed proximity to the line, where 1 means touching.
    pub near_ema: f64,
    /// Time-decayed frequency of near-line side changes.
    pub debounce_ema: f64,
    pub last_side: i8,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MrcTouchRegime {
    #[default]
    Unknown,
    Sideways,
    LongTransition,
    Long,
    ShortTransition,
    Short,
}

impl MrcTouchRegime {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Sideways => "sideway",
            Self::LongTransition => "sideway_to_long",
            Self::Long => "long",
            Self::ShortTransition => "sideway_to_short",
            Self::Short => "short",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MrcTouchSignal {
    pub lines: [MrcLineTouchEma; MRC_LINE_COUNT],
    pub nearest_line: Option<u8>,
    pub price_speed_pct_per_min: f64,
    pub price_angle_deg: f64,
    pub angle_slope_deg_per_min: f64,
    pub band_width_pct: f64,
    pub band_width_slope_pct_per_min: f64,
    pub direction_score: f64,
    /// Directional score after adding angle and contracting-band pressure.
    /// Positive values support upward staircase continuation.
    pub stairs_pressure: f64,
    /// Current activity/attention level, independent of direction.
    pub interest: f64,
    pub sideways_score: f64,
    pub regime: MrcTouchRegime,
    pub now_ms: u64,
    pub elapsed_ms: u64,
}

impl Default for MrcTouchSignal {
    fn default() -> Self {
        Self {
            lines: [MrcLineTouchEma::default(); MRC_LINE_COUNT],
            nearest_line: None,
            price_speed_pct_per_min: 0.0,
            price_angle_deg: 0.0,
            angle_slope_deg_per_min: 0.0,
            band_width_pct: 0.0,
            band_width_slope_pct_per_min: 0.0,
            direction_score: 0.0,
            stairs_pressure: 0.0,
            interest: 0.0,
            sideways_score: 0.0,
            regime: MrcTouchRegime::Unknown,
            now_ms: 0,
            elapsed_ms: 0,
        }
    }
}

impl MrcTouchSignal {
    pub fn line(self, line: u8) -> Option<MrcLineTouchEma> {
        self.lines.get(line as usize).copied()
    }

    /// Return a 0..1 directional support score for a candidate staircase.
    pub fn stairs_score(self, direction: i8) -> f64 {
        if !matches!(direction, 1 | -1) {
            return 0.0;
        }
        let directional = (self.stairs_pressure * direction as f64).clamp(-1.0, 1.0);
        ((directional + 1.0) * 0.5 * (0.5 + self.interest * 0.5)).clamp(0.0, 1.0)
    }

    pub fn summary(self) -> String {
        let (near, debounce) = self
            .nearest_line
            .and_then(|line| self.line(line))
            .map(|line| (line.near_ema, line.debounce_ema))
            .unwrap_or((0.0, 0.0));
        format!(
            "line={} near={:.2} debounce={:.2} regime={} speed={:.3}%/min angle={:.2}deg angle_slope={:.2}deg/min width={:.3}% width_slope={:.3}%/min dir={:.3} stairs={:.3} interest={:.3} sideway={:.3}",
            self.nearest_line
                .map(line_label)
                .unwrap_or("na"),
            near,
            debounce,
            self.regime.as_str(),
            self.price_speed_pct_per_min,
            self.price_angle_deg,
            self.angle_slope_deg_per_min,
            self.band_width_pct,
            self.band_width_slope_pct_per_min,
            self.direction_score,
            self.stairs_pressure,
            self.interest,
            self.sideways_score,
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MrcTouchState {
    pub signal: MrcTouchSignal,
    last_price: f64,
    last_time_ms: u64,
    previous_angle_deg: f64,
    previous_band_width_pct: f64,
    initialized: bool,
}

impl MrcTouchState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn update(&mut self, lines: [f64; MRC_LINE_COUNT], price: f64, now_ms: u64) {
        if !valid_price(price) || !valid_lines(lines) {
            return;
        }

        let width_pct = band_width_pct(lines);
        let nearest_line = nearest_line(lines, price);
        if !self.initialized {
            self.initialized = true;
            self.last_price = price;
            self.last_time_ms = now_ms;
            self.previous_band_width_pct = width_pct;
            self.signal.nearest_line = nearest_line;
            self.signal.band_width_pct = width_pct;
            self.signal.now_ms = now_ms;
            for (index, line) in lines.into_iter().enumerate() {
                let distance_pct = signed_distance_pct(price, line);
                self.signal.lines[index] = MrcLineTouchEma {
                    side_ema: side(distance_pct) as f64,
                    distance_ema_pct: distance_pct,
                    near_ema: proximity(distance_pct),
                    debounce_ema: 0.0,
                    last_side: side(distance_pct),
                };
            }
            return;
        }

        let elapsed_ms = now_ms.saturating_sub(self.last_time_ms);
        let elapsed_min = elapsed_ms as f64 / 60_000.0;
        let alpha = ema_alpha(elapsed_ms, EMA_TAU_MS);
        let debounce_alpha = ema_alpha(elapsed_ms, DEBOUNCE_TAU_MS);
        let raw_speed = if elapsed_min > 0.0 && self.last_price > 0.0 {
            (price - self.last_price) / self.last_price * 100.0 / elapsed_min
        } else {
            0.0
        };
        let previous_speed = self.signal.price_speed_pct_per_min;
        let speed = lerp(previous_speed, raw_speed, alpha);
        let angle = (speed / SPEED_SCALE_PCT_PER_MIN).atan() * 180.0 / PI;
        let raw_angle_slope = if elapsed_min > 0.0 {
            (angle - self.previous_angle_deg) / elapsed_min
        } else {
            0.0
        };
        let angle_slope = lerp(self.signal.angle_slope_deg_per_min, raw_angle_slope, alpha);
        let raw_width_slope = if elapsed_min > 0.0 {
            (width_pct - self.previous_band_width_pct) / elapsed_min
        } else {
            0.0
        };
        let width_slope = lerp(
            self.signal.band_width_slope_pct_per_min,
            raw_width_slope,
            alpha,
        );

        for (index, line) in lines.into_iter().enumerate() {
            let distance_pct = signed_distance_pct(price, line);
            let current_side = side(distance_pct);
            let near = proximity(distance_pct);
            let previous = self.signal.lines[index];
            let crossed_nearby = current_side != 0
                && previous.last_side != 0
                && current_side != previous.last_side
                && (near > 0.10 || previous.near_ema > 0.10);
            self.signal.lines[index] = MrcLineTouchEma {
                side_ema: lerp(previous.side_ema, current_side as f64, alpha),
                distance_ema_pct: lerp(previous.distance_ema_pct, distance_pct, alpha),
                near_ema: lerp(previous.near_ema, near, alpha),
                debounce_ema: lerp(
                    previous.debounce_ema,
                    if crossed_nearby { 1.0 } else { 0.0 },
                    debounce_alpha,
                ),
                last_side: current_side,
            };
        }

        let speed_norm = (speed / SPEED_SCALE_PCT_PER_MIN).tanh();
        let near_total: f64 = self.signal.lines.iter().map(|line| line.near_ema).sum();
        let reversion = if near_total > 0.0 {
            self.signal
                .lines
                .iter()
                .map(|line| -line.side_ema * speed_norm * line.near_ema)
                .sum::<f64>()
                / near_total
        } else {
            0.0
        };
        let direction_score = (speed_norm * 0.65 + reversion * 0.35).clamp(-1.0, 1.0);
        let angle_score = (angle_slope / ANGLE_SLOPE_SCALE_DEG_PER_MIN).tanh();
        let contraction_score = (-width_slope / WIDTH_SLOPE_SCALE_PCT_PER_MIN).clamp(-1.0, 1.0);
        let stairs_pressure = (direction_score * 0.60
            + angle_score * 0.25
            + contraction_score * direction_score * 0.15)
            .clamp(-1.0, 1.0);
        let max_debounce = self
            .signal
            .lines
            .iter()
            .map(|line| line.debounce_ema)
            .fold(0.0, f64::max);
        let max_near = self
            .signal
            .lines
            .iter()
            .map(|line| line.near_ema)
            .fold(0.0, f64::max);
        let speed_interest = speed.abs().tanh();
        let geometry_interest =
            angle_slope.abs().min(ANGLE_SLOPE_SCALE_DEG_PER_MIN) / ANGLE_SLOPE_SCALE_DEG_PER_MIN;
        let band_interest =
            width_slope.abs().min(WIDTH_SLOPE_SCALE_PCT_PER_MIN) / WIDTH_SLOPE_SCALE_PCT_PER_MIN;
        let interest = (speed_interest * 0.40
            + max_debounce * 0.30
            + geometry_interest * 0.20
            + band_interest * 0.10)
            .clamp(0.0, 1.0);
        let sideways_score =
            (max_debounce * 0.60 + max_near * 0.20 + (1.0 - direction_score.abs()) * 0.20)
                .clamp(0.0, 1.0);
        let regime = regime(self.signal.regime, direction_score, sideways_score);

        self.signal = MrcTouchSignal {
            nearest_line,
            price_speed_pct_per_min: speed,
            price_angle_deg: angle,
            angle_slope_deg_per_min: angle_slope,
            band_width_pct: width_pct,
            band_width_slope_pct_per_min: width_slope,
            direction_score,
            stairs_pressure,
            interest,
            sideways_score,
            regime,
            now_ms,
            elapsed_ms,
            ..self.signal
        };
        self.last_price = price;
        self.last_time_ms = now_ms;
        self.previous_angle_deg = angle;
        self.previous_band_width_pct = width_pct;
    }
}

fn valid_price(price: f64) -> bool {
    price.is_finite() && price > 0.0
}

fn valid_lines(lines: [f64; MRC_LINE_COUNT]) -> bool {
    lines.into_iter().all(valid_price)
}

fn signed_distance_pct(price: f64, line: f64) -> f64 {
    (price - line) / line * 100.0
}

fn side(distance_pct: f64) -> i8 {
    if distance_pct > 0.0 {
        1
    } else if distance_pct < 0.0 {
        -1
    } else {
        0
    }
}

fn proximity(distance_pct: f64) -> f64 {
    (-distance_pct.abs() / NEAR_LINE_PCT).exp().clamp(0.0, 1.0)
}

fn ema_alpha(elapsed_ms: u64, tau_ms: f64) -> f64 {
    if elapsed_ms == 0 || !tau_ms.is_finite() || tau_ms <= 0.0 {
        0.0
    } else {
        (1.0 - (-(elapsed_ms as f64) / tau_ms).exp()).clamp(0.0, 1.0)
    }
}

fn lerp(previous: f64, current: f64, alpha: f64) -> f64 {
    previous + (current - previous) * alpha
}

fn band_width_pct(lines: [f64; MRC_LINE_COUNT]) -> f64 {
    let mean = lines[4];
    ((lines[8] - lines[0]).abs() / mean * 100.0).max(0.0)
}

fn nearest_line(lines: [f64; MRC_LINE_COUNT], price: f64) -> Option<u8> {
    lines
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            (price - **left)
                .abs()
                .partial_cmp(&(price - **right).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index as u8)
}

fn regime(previous: MrcTouchRegime, direction_score: f64, sideways_score: f64) -> MrcTouchRegime {
    if sideways_score >= 0.55 && direction_score.abs() < 0.35 {
        return MrcTouchRegime::Sideways;
    }
    if direction_score >= 0.35 {
        return if matches!(previous, MrcTouchRegime::Sideways) {
            MrcTouchRegime::LongTransition
        } else {
            MrcTouchRegime::Long
        };
    }
    if direction_score <= -0.35 {
        return if matches!(previous, MrcTouchRegime::Sideways) {
            MrcTouchRegime::ShortTransition
        } else {
            MrcTouchRegime::Short
        };
    }
    previous
}

fn line_label(index: u8) -> &'static str {
    match index {
        0 => "db",
        1 => "do",
        2 => "ds",
        3 => "di",
        4 => "m",
        5 => "ui",
        6 => "us",
        7 => "uo",
        8 => "ub",
        _ => "na",
    }
}
