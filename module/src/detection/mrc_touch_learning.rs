//! Empirical learning from exact MRC-line touches.
//!
//! This module intentionally does not calculate a weighted direction score.
//! It records touch episodes and asks a simple historical question: after
//! this line was touched in the same 5m context, which line was touched next?
//! Matching is exact first, then falls back to the broader line history.

use serde::{Deserialize, Serialize};

pub const MRC_LINE_COUNT: usize = 9;

const EXACT_TOUCH_PCT: f64 = 0.08;
const CONTEXT_SIDE_PCT: f64 = 0.05;
const RELIABLE_SAMPLES: u32 = 3;
const TIME_BUCKET_COUNT: usize = 3;
const CONTEXT_COUNT: usize = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TouchTimeBucket {
    #[default]
    Unknown,
    Fast,
    Normal,
    Slow,
}

impl TouchTimeBucket {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Fast => "fast",
            Self::Normal => "normal",
            Self::Slow => "slow",
        }
    }

    fn index(self) -> Option<usize> {
        match self {
            Self::Fast => Some(0),
            Self::Normal => Some(1),
            Self::Slow => Some(2),
            Self::Unknown => None,
        }
    }

    fn from_elapsed_ms(elapsed_ms: u64) -> Self {
        if elapsed_ms < 30_000 {
            Self::Fast
        } else if elapsed_ms < 180_000 {
            Self::Normal
        } else {
            Self::Slow
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TouchMatchScope {
    #[default]
    None,
    Line,
    Slope,
    Exact,
}

impl TouchMatchScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Line => "line",
            Self::Slope => "slope",
            Self::Exact => "exact",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TouchContext {
    /// Direction of the 5m MRC mean-line slope: -1, 0, or 1.
    pub slope: i8,
    /// Price side of the 5m mean line: -1, 0, or 1.
    pub price_side: i8,
}

impl TouchContext {
    fn index(self) -> usize {
        let slope = (self.slope.clamp(-1, 1) + 1) as usize;
        let side = (self.price_side.clamp(-1, 1) + 1) as usize;
        slope * 3 + side
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TouchRouteStats {
    pub up_count: u32,
    pub down_count: u32,
    pub next_lines: [u32; MRC_LINE_COUNT],
}

impl Default for TouchRouteStats {
    fn default() -> Self {
        Self {
            up_count: 0,
            down_count: 0,
            next_lines: [0; MRC_LINE_COUNT],
        }
    }
}

impl TouchRouteStats {
    fn samples(self) -> u32 {
        self.up_count.saturating_add(self.down_count)
    }

    fn record(&mut self, source: u8, destination: u8) {
        if destination as usize >= MRC_LINE_COUNT || source == destination {
            return;
        }
        self.next_lines[destination as usize] =
            self.next_lines[destination as usize].saturating_add(1);
        if destination > source {
            self.up_count = self.up_count.saturating_add(1);
        } else {
            self.down_count = self.down_count.saturating_add(1);
        }
    }

    fn merge(&mut self, other: Self) {
        self.up_count = self.up_count.saturating_add(other.up_count);
        self.down_count = self.down_count.saturating_add(other.down_count);
        for (left, right) in self.next_lines.iter_mut().zip(other.next_lines) {
            *left = left.saturating_add(right);
        }
    }

    fn best_direction(self) -> i8 {
        if self.up_count > self.down_count {
            1
        } else if self.down_count > self.up_count {
            -1
        } else {
            0
        }
    }

    fn best_destination(self) -> Option<u8> {
        self.next_lines
            .iter()
            .enumerate()
            .filter(|(_, count)| **count > 0)
            .max_by_key(|(_, count)| **count)
            .map(|(line, _)| line as u8)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MrcTouchLearningSignal {
    pub current_line: Option<u8>,
    pub exact_touch: bool,
    pub distance_pct: f64,
    pub context: TouchContext,
    pub learned_direction: i8,
    pub learned_destination: Option<u8>,
    pub learned_samples: u32,
    pub learned_up_count: u32,
    pub learned_down_count: u32,
    pub learned_time_bucket: TouchTimeBucket,
    pub match_scope: TouchMatchScope,
    pub reliable: bool,
    pub now_ms: u64,
}

impl Default for MrcTouchLearningSignal {
    fn default() -> Self {
        Self {
            current_line: None,
            exact_touch: false,
            distance_pct: 0.0,
            context: TouchContext::default(),
            learned_direction: 0,
            learned_destination: None,
            learned_samples: 0,
            learned_up_count: 0,
            learned_down_count: 0,
            learned_time_bucket: TouchTimeBucket::Unknown,
            match_scope: TouchMatchScope::None,
            reliable: false,
            now_ms: 0,
        }
    }
}

impl MrcTouchLearningSignal {
    pub fn summary(self) -> String {
        format!(
            "line={} exact={} ctx={}/{} learn={} next={} samples={} up={} down={} tf={} scope={} reliable={}",
            self.current_line
                .map(line_label)
                .unwrap_or("na"),
            self.exact_touch,
            self.context.slope,
            self.context.price_side,
            direction_label(self.learned_direction),
            self.learned_destination
                .map(line_label)
                .unwrap_or("na"),
            self.learned_samples,
            self.learned_up_count,
            self.learned_down_count,
            self.learned_time_bucket.as_str(),
            self.match_scope.as_str(),
            self.reliable,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MrcTouchLearningState {
    pub signal: MrcTouchLearningSignal,
    last_price: f64,
    last_time_ms: u64,
    last_touch_line: Option<u8>,
    last_touch_time_ms: u64,
    last_context: TouchContext,
    initialized: bool,
    routes: Vec<TouchRouteStats>,
}

impl Default for MrcTouchLearningState {
    fn default() -> Self {
        Self {
            signal: MrcTouchLearningSignal::default(),
            last_price: 0.0,
            last_time_ms: 0,
            last_touch_line: None,
            last_touch_time_ms: 0,
            last_context: TouchContext::default(),
            initialized: false,
            routes: vec![
                TouchRouteStats::default();
                MRC_LINE_COUNT * CONTEXT_COUNT * TIME_BUCKET_COUNT
            ],
        }
    }
}

impl MrcTouchLearningState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn update(
        &mut self,
        lines: [f64; MRC_LINE_COUNT],
        price: f64,
        now_ms: u64,
        slow_lines: [f64; MRC_LINE_COUNT],
        slow_slope: i8,
    ) {
        self.update_with_events(lines, price, now_ms, slow_lines, slow_slope, &[]);
    }

    pub fn update_with_events(
        &mut self,
        lines: [f64; MRC_LINE_COUNT],
        price: f64,
        now_ms: u64,
        slow_lines: [f64; MRC_LINE_COUNT],
        slow_slope: i8,
        touched_lines: &[u8],
    ) {
        if !valid_price(price) || !valid_lines(lines) {
            return;
        }
        let (near_line, distance_pct) = nearest_touch(lines, price);
        let current_line = near_line.or_else(|| {
            touched_lines
                .iter()
                .rev()
                .copied()
                .find(|line| (*line as usize) < MRC_LINE_COUNT)
        });
        let context = context(slow_lines, price, slow_slope);

        if self.initialized {
            if let (Some(source), Some(destination)) = (self.last_touch_line, current_line) {
                if source != destination {
                    let elapsed_ms = now_ms.saturating_sub(self.last_touch_time_ms);
                    self.record(source, destination, self.last_context, elapsed_ms);
                    self.last_touch_line = Some(destination);
                    self.last_touch_time_ms = now_ms;
                    self.last_context = context;
                }
            } else if self.last_touch_line.is_none() && current_line.is_some() {
                self.last_touch_line = current_line;
                self.last_touch_time_ms = now_ms;
                self.last_context = context;
            }
        } else {
            self.initialized = true;
            self.last_touch_line = current_line;
            self.last_touch_time_ms = now_ms;
            self.last_context = context;
        }

        self.signal = self.learn(
            current_line,
            near_line.is_some() || !touched_lines.is_empty(),
            distance_pct,
            context,
            now_ms,
        );
        self.last_price = price;
        self.last_time_ms = now_ms;
    }

    fn record(&mut self, source: u8, destination: u8, context: TouchContext, elapsed_ms: u64) {
        let Some(time_bucket) = TouchTimeBucket::from_elapsed_ms(elapsed_ms).index() else {
            return;
        };
        let index = route_index(source, context, time_bucket);
        if let Some(stats) = self.routes.get_mut(index) {
            stats.record(source, destination);
        }
    }

    fn learn(
        &self,
        current_line: Option<u8>,
        touch_observed: bool,
        distance_pct: f64,
        context: TouchContext,
        now_ms: u64,
    ) -> MrcTouchLearningSignal {
        let Some(line) = current_line else {
            return MrcTouchLearningSignal {
                distance_pct,
                context,
                now_ms,
                ..Default::default()
            };
        };

        let (stats, scope) = self.matching_stats(line, context);
        let time_bucket = self.best_time_bucket(line, context, scope);
        let samples = stats.samples();
        let direction = stats.best_direction();
        MrcTouchLearningSignal {
            current_line: Some(line),
            exact_touch: touch_observed,
            distance_pct,
            context,
            learned_direction: direction,
            learned_destination: stats.best_destination(),
            learned_samples: samples,
            learned_up_count: stats.up_count,
            learned_down_count: stats.down_count,
            learned_time_bucket: time_bucket,
            match_scope: scope,
            reliable: samples >= RELIABLE_SAMPLES && direction != 0,
            now_ms,
        }
    }

    fn matching_stats(
        &self,
        line: u8,
        context: TouchContext,
    ) -> (TouchRouteStats, TouchMatchScope) {
        let exact = self.aggregate(line, |candidate| candidate == context);
        if exact.samples() > 0 {
            return (exact, TouchMatchScope::Exact);
        }
        let slope = self.aggregate(line, |candidate| candidate.slope == context.slope);
        if slope.samples() > 0 {
            return (slope, TouchMatchScope::Slope);
        }
        let line_stats = self.aggregate(line, |_| true);
        if line_stats.samples() > 0 {
            return (line_stats, TouchMatchScope::Line);
        }
        (TouchRouteStats::default(), TouchMatchScope::None)
    }

    fn aggregate<F>(&self, line: u8, matches: F) -> TouchRouteStats
    where
        F: Fn(TouchContext) -> bool,
    {
        let mut aggregate = TouchRouteStats::default();
        for slope in -1..=1 {
            for price_side in -1..=1 {
                let candidate = TouchContext { slope, price_side };
                if !matches(candidate) {
                    continue;
                }
                for bucket in 0..TIME_BUCKET_COUNT {
                    let index = route_index(line, candidate, bucket);
                    if let Some(stats) = self.routes.get(index) {
                        aggregate.merge(*stats);
                    }
                }
            }
        }
        aggregate
    }

    fn best_time_bucket(
        &self,
        line: u8,
        context: TouchContext,
        scope: TouchMatchScope,
    ) -> TouchTimeBucket {
        if matches!(scope, TouchMatchScope::None) {
            return TouchTimeBucket::Unknown;
        }
        let mut best = (0u32, TouchTimeBucket::Unknown);
        for bucket in [
            TouchTimeBucket::Fast,
            TouchTimeBucket::Normal,
            TouchTimeBucket::Slow,
        ] {
            let index = bucket.index().expect("known time bucket");
            let stats = match scope {
                TouchMatchScope::Exact => {
                    self.aggregate_bucket(line, |candidate| candidate == context, index)
                }
                TouchMatchScope::Slope => {
                    self.aggregate_bucket(line, |candidate| candidate.slope == context.slope, index)
                }
                TouchMatchScope::Line => self.aggregate_bucket(line, |_| true, index),
                TouchMatchScope::None => TouchRouteStats::default(),
            };
            if stats.samples() > best.0 {
                best = (stats.samples(), bucket);
            }
        }
        best.1
    }

    fn aggregate_bucket<F>(&self, line: u8, matches: F, bucket: usize) -> TouchRouteStats
    where
        F: Fn(TouchContext) -> bool,
    {
        let mut aggregate = TouchRouteStats::default();
        for slope in -1..=1 {
            for price_side in -1..=1 {
                let candidate = TouchContext { slope, price_side };
                if matches(candidate) {
                    let index = route_index(line, candidate, bucket);
                    if let Some(stats) = self.routes.get(index) {
                        aggregate.merge(*stats);
                    }
                }
            }
        }
        aggregate
    }
}

fn route_index(line: u8, context: TouchContext, bucket: usize) -> usize {
    ((line as usize * CONTEXT_COUNT) + context.index()) * TIME_BUCKET_COUNT + bucket
}

fn nearest_touch(lines: [f64; MRC_LINE_COUNT], price: f64) -> (Option<u8>, f64) {
    let Some((index, line)) = lines.iter().enumerate().min_by(|(_, left), (_, right)| {
        (price - **left)
            .abs()
            .partial_cmp(&(price - **right).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    }) else {
        return (None, 0.0);
    };
    let distance_pct = (price - *line) / *line * 100.0;
    (
        (distance_pct.abs() <= EXACT_TOUCH_PCT).then_some(index as u8),
        distance_pct,
    )
}

fn context(lines: [f64; MRC_LINE_COUNT], price: f64, slow_slope: i8) -> TouchContext {
    let mean = lines[4];
    let distance_pct = if valid_price(mean) {
        (price - mean) / mean * 100.0
    } else {
        0.0
    };
    let price_side = if distance_pct > CONTEXT_SIDE_PCT {
        1
    } else if distance_pct < -CONTEXT_SIDE_PCT {
        -1
    } else {
        0
    };
    TouchContext {
        slope: slow_slope.clamp(-1, 1),
        price_side,
    }
}

fn valid_price(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn valid_lines(lines: [f64; MRC_LINE_COUNT]) -> bool {
    lines.into_iter().all(valid_price)
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

fn direction_label(direction: i8) -> &'static str {
    match direction {
        1 => "long",
        -1 => "short",
        _ => "na",
    }
}
