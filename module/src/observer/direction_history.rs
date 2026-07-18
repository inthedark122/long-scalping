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
}

#[derive(Debug, Clone, Default)]
pub struct DirectionChangeHistory {
    pub current_direction: Option<MarketState>,
    pub current_started_at_ms: u64,
    pub current_min_price: f64,
    pub current_max_price: f64,
    pub last_detected_at_ms: u64,
    pub records: Vec<DirectionChangeRecord>,
}

impl DirectionChangeHistory {
    const MAX_RECORDS: usize = 128;

    pub fn observe(&mut self, direction: MarketState, price: f64, detected_at_ms: u64) -> bool {
        let mut changed = false;

        match self.current_direction {
            None => {
                self.current_direction = Some(direction);
                self.current_started_at_ms = detected_at_ms;
                self.current_min_price = price;
                self.current_max_price = price;
            }
            Some(current) if current == direction => {
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
            }
            Some(current) => {
                let elapsed_ms = detected_at_ms.saturating_sub(self.current_started_at_ms);
                self.records.push(DirectionChangeRecord {
                    from: current,
                    to: direction,
                    detected_at_ms,
                    elapsed_ms,
                    detect_price: price,
                    min_price: self.current_min_price,
                    max_price: self.current_max_price,
                });
                if self.records.len() > Self::MAX_RECORDS {
                    let overflow = self.records.len() - Self::MAX_RECORDS;
                    self.records.drain(0..overflow);
                }
                self.current_direction = Some(direction);
                self.current_started_at_ms = detected_at_ms;
                self.current_min_price = price;
                self.current_max_price = price;
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

    pub fn summary(&self) -> String {
        let current = self
            .current_direction
            .map(|direction| format!("{:?}", direction))
            .unwrap_or_else(|| "na".to_string());
        let last_interval = self
            .last_interval_ms()
            .map(|value| format!("{}ms", value))
            .unwrap_or_else(|| "na".to_string());
        let avg3 = self
            .average_interval_ms(3)
            .map(|value| format!("{:.0}ms", value))
            .unwrap_or_else(|| "na".to_string());
        let avg8 = self
            .average_interval_ms(8)
            .map(|value| format!("{:.0}ms", value))
            .unwrap_or_else(|| "na".to_string());

        match self.records.last() {
            Some(record) => format!(
                "current={} last={} avg3={} avg8={} count={} last_change={:?}->{:?} at={} price={:.4} min={:.4} max={:.4}",
                current,
                last_interval,
                avg3,
                avg8,
                self.records.len(),
                record.from,
                record.to,
                record.detected_at_ms,
                record.detect_price,
                record.min_price,
                record.max_price
            ),
            None => format!(
                "current={} last={} avg3={} avg8={} count=0",
                current, last_interval, avg3, avg8
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

        let first = &history.records[0];
        assert_eq!(first.from, MarketState::Short);
        assert_eq!(first.to, MarketState::Long);
        assert_eq!(first.elapsed_ms, 3_000);
        assert_eq!(first.detect_price, 11.0);
        assert_eq!(first.min_price, 9.0);
        assert_eq!(first.max_price, 10.0);
    }
}
