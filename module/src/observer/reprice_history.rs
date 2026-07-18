use crate::format::{fmt_duration_ms, fmt_price_tick};
use crate::model::ExitMode;

#[derive(Debug, Clone)]
pub struct TpRepriceRecord {
    pub elapsed_ms: u64,
    pub price: f64,
    pub previous_tp: f64,
    pub new_tp: f64,
    pub reason: String,
    pub source: String,
    pub exit_mode: ExitMode,
    pub virtual_sl_hit: bool,
    pub extra: String,
}

#[derive(Debug, Clone, Default)]
pub struct TpRepriceHistory {
    pub total_count: u64,
    pub last_detected_at_ms: u64,
    pub records: Vec<TpRepriceRecord>,
}

impl TpRepriceHistory {
    const MAX_RECORDS: usize = 32;

    pub fn observe(
        &mut self,
        detected_at_ms: u64,
        price: f64,
        previous_tp: f64,
        new_tp: f64,
        reason: impl Into<String>,
        source: impl Into<String>,
        exit_mode: ExitMode,
        virtual_sl_hit: bool,
        extra: impl Into<String>,
    ) {
        let elapsed_ms = if self.last_detected_at_ms == 0 {
            0
        } else {
            detected_at_ms.saturating_sub(self.last_detected_at_ms)
        };

        self.total_count = self.total_count.saturating_add(1);
        self.last_detected_at_ms = detected_at_ms;
        self.records.push(TpRepriceRecord {
            elapsed_ms,
            price,
            previous_tp,
            new_tp,
            reason: reason.into(),
            source: source.into(),
            exit_mode,
            virtual_sl_hit,
            extra: extra.into(),
        });
        if self.records.len() > Self::MAX_RECORDS {
            let overflow = self.records.len() - Self::MAX_RECORDS;
            self.records.drain(0..overflow);
        }
    }

    pub fn summary(&self, tick_size: f64) -> Option<String> {
        let last = self.records.last()?;
        let last_delay = if last.elapsed_ms == 0 {
            "na".to_string()
        } else {
            fmt_duration_ms(last.elapsed_ms)
        };
        let recent = self
            .records
            .iter()
            .rev()
            .take(3)
            .map(|record| {
                let delay = if record.elapsed_ms == 0 {
                    "na".to_string()
                } else {
                    fmt_duration_ms(record.elapsed_ms)
                };
                format!(
                    "{}:{}:{}->{}@{}:{}:{}",
                    delay,
                    record.reason,
                    fmt_price_tick(record.previous_tp, tick_size),
                    fmt_price_tick(record.new_tp, tick_size),
                    fmt_price_tick(record.price, tick_size),
                    record.source,
                    record.extra
                )
            })
            .collect::<Vec<_>>()
            .join(";");

        Some(format!(
            "count={} last_delay={} last_mode={:?} last_vsl={} last3=[{}]",
            self.total_count, last_delay, last.exit_mode, last.virtual_sl_hit, recent
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_recent_reprices_and_delays() {
        let mut history = TpRepriceHistory::default();

        history.observe(
            1_000,
            100.0,
            99.0,
            100.5,
            "tp-reprice",
            "structural",
            ExitMode::Structural,
            false,
            "source=mean".to_string(),
        );
        history.observe(
            2_500,
            101.0,
            100.5,
            101.2,
            "tp-reprice",
            "structural",
            ExitMode::TrailAfterVirtualSl,
            true,
            "source=bb".to_string(),
        );
        history.observe(
            5_000,
            100.8,
            101.2,
            100.9,
            "tp-advance",
            "virtual-sl",
            ExitMode::TrailAfterVirtualSl,
            true,
            "mode=martingale".to_string(),
        );

        assert_eq!(history.total_count, 3);
        let summary = history.summary(0.1).expect("summary");
        assert!(summary.contains("count=3"));
        assert!(summary.contains("last_delay=2.5s"));
        assert!(summary.contains("tp-reprice"));
        assert!(summary.contains("tp-advance"));
        assert!(summary.contains("last3=["));
    }
}
