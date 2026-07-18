use crate::format::{fmt_duration_ms, fmt_duration_ms_f64, fmt_price_tick};
use crate::model::MarketState;

#[derive(Debug, Clone)]
pub struct DirectionChangeRecord {
    pub from: MarketState,
    pub to: MarketState,
    pub detected_at_ms: u64,
    pub elapsed_ms: u64,
    pub detect_price: f64,
    pub min_price: f64,
    pub max_price: f64,
    pub adverse_ms: u64,
    pub last_cross_at_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct DirectionChangeHistory {
    pub current_direction: Option<MarketState>,
    pub current_started_at_ms: u64,
    pub current_min_price: f64,
    pub current_max_price: f64,
    pub current_reference_price: f64,
    pub current_adverse_ms: u64,
    pub current_last_cross_at_ms: u64,
    pub current_last_observed_at_ms: u64,
    pub current_last_price: f64,
    pub last_detected_at_ms: u64,
    pub records: Vec<DirectionChangeRecord>,
}

impl DirectionChangeHistory {
    const MAX_RECORDS: usize = 128;

    fn is_adverse(direction: MarketState, price: f64, reference_price: f64) -> bool {
        let primary = if direction == MarketState::Sideways {
            MarketState::Long
        } else {
            direction
        };
        match primary {
            MarketState::Long => price < reference_price,
            MarketState::Short => price > reference_price,
            MarketState::Sideways => false,
        }
    }

    fn crossed(reference_price: f64, previous_price: f64, current_price: f64) -> bool {
        (previous_price < reference_price && current_price >= reference_price)
            || (previous_price > reference_price && current_price <= reference_price)
    }

    fn advance_current(&mut self, price: f64, detected_at_ms: u64) {
        let elapsed_ms = detected_at_ms.saturating_sub(self.current_last_observed_at_ms);
        if Self::is_adverse(
            self.current_direction.unwrap_or(MarketState::Sideways),
            self.current_last_price,
            self.current_reference_price,
        ) {
            self.current_adverse_ms = self.current_adverse_ms.saturating_add(elapsed_ms);
        }
        if Self::crossed(self.current_reference_price, self.current_last_price, price) {
            self.current_last_cross_at_ms = detected_at_ms;
        }
        self.current_min_price = if self.current_min_price > 0.0 {
            self.current_min_price.min(price)
        } else {
            price
        };
        self.current_max_price = if self.current_max_price > 0.0 {
            self.current_max_price.max(price)
        } else {
            price
        };
        self.current_last_price = price;
        self.current_last_observed_at_ms = detected_at_ms;
    }

    fn start_current(&mut self, direction: MarketState, price: f64, detected_at_ms: u64) {
        self.current_direction = Some(direction);
        self.current_started_at_ms = detected_at_ms;
        self.current_min_price = price;
        self.current_max_price = price;
        self.current_reference_price = price;
        self.current_adverse_ms = 0;
        self.current_last_cross_at_ms = detected_at_ms;
        self.current_last_observed_at_ms = detected_at_ms;
        self.current_last_price = price;
    }

    pub fn observe(&mut self, direction: MarketState, price: f64, detected_at_ms: u64) -> bool {
        let mut changed = false;

        match self.current_direction {
            None => {
                self.start_current(direction, price, detected_at_ms);
            }
            Some(current) if current == direction => {
                self.advance_current(price, detected_at_ms);
            }
            Some(current) => {
                let segment_min_price = self.current_min_price;
                let segment_max_price = self.current_max_price;
                self.advance_current(price, detected_at_ms);
                let elapsed_ms = detected_at_ms.saturating_sub(self.current_started_at_ms);
                self.records.push(DirectionChangeRecord {
                    from: current,
                    to: direction,
                    detected_at_ms,
                    elapsed_ms,
                    detect_price: price,
                    min_price: segment_min_price,
                    max_price: segment_max_price,
                    adverse_ms: self.current_adverse_ms,
                    last_cross_at_ms: self.current_last_cross_at_ms,
                });
                if self.records.len() > Self::MAX_RECORDS {
                    let overflow = self.records.len() - Self::MAX_RECORDS;
                    self.records.drain(0..overflow);
                }
                self.start_current(direction, price, detected_at_ms);
                self.last_detected_at_ms = detected_at_ms;
                changed = true;
            }
        }

        if self.current_direction == Some(direction) && self.last_detected_at_ms != detected_at_ms {
            self.last_detected_at_ms = detected_at_ms;
        }

        changed
    }

    pub fn last_interval_ms(&self) -> Option<u64> {
        self.records.last().map(|record| record.elapsed_ms)
    }

    pub fn average_interval_ms(&self, count: usize) -> Option<f64> {
        if count == 0 || self.records.is_empty() {
            return None;
        }
        let mut total = 0.0;
        let mut samples = 0usize;
        for record in self.records.iter().rev().take(count) {
            total += record.elapsed_ms as f64;
            samples += 1;
        }
        (samples > 0).then_some(total / samples as f64)
    }

    pub fn average_window_pct(&self, count: usize) -> Option<f64> {
        if count == 0 || self.records.is_empty() {
            return None;
        }
        let mut total = 0.0;
        let mut samples = 0usize;
        for record in self.records.iter().rev().take(count) {
            if record.detect_price > 0.0 {
                total += (record.max_price - record.min_price).abs() / record.detect_price * 100.0;
                samples += 1;
            }
        }
        (samples > 0).then_some(total / samples as f64)
    }

    pub fn current_elapsed_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.current_started_at_ms)
    }

    pub fn current_adverse_pct(&self, now_ms: u64) -> f64 {
        let elapsed_ms = self.current_elapsed_ms(now_ms);
        if elapsed_ms == 0 {
            0.0
        } else {
            self.current_adverse_ms as f64 / elapsed_ms as f64 * 100.0
        }
    }

    pub fn current_score_pct(&self, now_ms: u64) -> f64 {
        100.0 - self.current_adverse_pct(now_ms)
    }

    pub fn average_adverse_pct(&self, count: usize) -> Option<f64> {
        if count == 0 || self.records.is_empty() {
            return None;
        }
        let mut total = 0.0;
        let mut samples = 0usize;
        for record in self.records.iter().rev().take(count) {
            if record.elapsed_ms > 0 {
                total += record.adverse_ms as f64 / record.elapsed_ms as f64 * 100.0;
                samples += 1;
            }
        }
        (samples > 0).then_some(total / samples as f64)
    }

    pub fn summary(&self, tick_size: f64) -> String {
        let current = self
            .current_direction
            .map(|direction| format!("{:?}", direction))
            .unwrap_or_else(|| "na".to_string());
        let last_interval = self
            .last_interval_ms()
            .map(fmt_duration_ms)
            .unwrap_or_else(|| "na".to_string());
        let avg3 = self
            .average_interval_ms(3)
            .map(fmt_duration_ms_f64)
            .unwrap_or_else(|| "na".to_string());
        let avg8 = self
            .average_interval_ms(8)
            .map(fmt_duration_ms_f64)
            .unwrap_or_else(|| "na".to_string());
        let avg3_window = self
            .average_window_pct(3)
            .map(|value| format!("{value:.2}%"))
            .unwrap_or_else(|| "na".to_string());
        let avg8_window = self
            .average_window_pct(8)
            .map(|value| format!("{value:.2}%"))
            .unwrap_or_else(|| "na".to_string());

        match self.records.last() {
            Some(record) => format!(
                "current={} entry={} score={:.1}% out={:.1}% since_cross={} last={} avg3={}:range={:.2}%:out={:.1}% avg8={}:range={:.2}%:out={:.1}% count={} last_change={:?}->{:?} since_cross={} at={} price={} min={} max={} window={:.2}%",
                current,
                fmt_price_tick(self.current_reference_price, tick_size),
                self.current_score_pct(self.last_detected_at_ms),
                self.current_adverse_pct(self.last_detected_at_ms),
                fmt_duration_ms(
                    self.last_detected_at_ms
                        .saturating_sub(self.current_last_cross_at_ms),
                ),
                last_interval,
                avg3,
                avg3_window,
                self.average_adverse_pct(3).unwrap_or(0.0),
                avg8,
                avg8_window,
                self.average_adverse_pct(8).unwrap_or(0.0),
                self.records.len(),
                record.from,
                record.to,
                fmt_duration_ms(record.detected_at_ms.saturating_sub(record.last_cross_at_ms)),
                record.detected_at_ms,
                fmt_price_tick(record.detect_price, tick_size),
                fmt_price_tick(record.min_price, tick_size),
                fmt_price_tick(record.max_price, tick_size),
                (record.max_price - record.min_price).abs() / record.detect_price * 100.0
            ),
            None => format!(
                "current={} entry={} score=na out=na since_cross=na last={} avg3={}:{} avg8={}:{} count=na",
                current,
                fmt_price_tick(self.current_reference_price, tick_size),
                last_interval,
                avg3,
                avg3_window,
                avg8,
                avg8_window
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_history_tracks_runs_and_averages() {
        let mut history = DirectionChangeHistory::default();

        assert!(!history.observe(MarketState::Short, 10.0, 1_000));
        assert!(!history.observe(MarketState::Short, 9.0, 2_000));
        assert!(history.observe(MarketState::Long, 11.0, 4_000));
        assert!(!history.observe(MarketState::Long, 12.0, 7_000));
        assert!(history.observe(MarketState::Sideways, 8.0, 9_500));

        assert_eq!(history.records.len(), 2);
        assert_eq!(history.last_interval_ms(), Some(5_500));
        assert_eq!(history.average_interval_ms(3), Some(4_250.0));
        assert_eq!(history.average_interval_ms(8), Some(4_250.0));

        let summary = history.summary(0.01);
        assert!(summary.contains("last=5.5s"));
        assert!(summary.contains("avg3=4.2s:range="));
        assert!(summary.contains("avg8=4.2s:range="));

        let first = &history.records[0];
        assert_eq!(first.from, MarketState::Short);
        assert_eq!(first.to, MarketState::Long);
        assert_eq!(first.elapsed_ms, 3_000);
        assert_eq!(first.detect_price, 11.0);
        assert_eq!(first.min_price, 9.0);
        assert_eq!(first.max_price, 10.0);
    }

    #[test]
    fn direction_history_scores_time_outside_long_direction() {
        let mut history = DirectionChangeHistory::default();

        history.observe(MarketState::Long, 100.0, 1_000);
        history.observe(MarketState::Long, 99.0, 2_000);
        history.observe(MarketState::Long, 98.0, 4_000);
        history.observe(MarketState::Long, 101.0, 5_000);
        assert!(history.observe(MarketState::Sideways, 101.0, 6_000));

        let record = &history.records[0];
        assert_eq!(record.adverse_ms, 3_000);
        assert_eq!(record.last_cross_at_ms, 5_000);
        assert_eq!(history.average_adverse_pct(3), Some(60.0));
    }
}
