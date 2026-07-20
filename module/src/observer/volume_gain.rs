use crate::format::fmt_metric;

pub const VOLUME_GAIN_THRESHOLD: f64 = 2.0;

#[derive(Debug, Clone, Copy)]
pub struct VolumeGainRecord {
    pub candle: u64,
    pub gain: f64,
    pub buy_sell_gain: f64,
    pub mrc_zone: i8,
    pub mrc_cross_mask: u16,
    pub distance: Option<u64>,
    pub low_gain_before: u32,
    pub post_gain_low: f64,
    pub post_gain_high: f64,
}

#[derive(Debug, Clone, Default)]
pub struct VolumeGainTracker {
    pub records: Vec<VolumeGainRecord>,
    pub distances: Vec<u64>,
    pub low_gain_candles: u32,
    last_big_candle: Option<u64>,
}

impl VolumeGainTracker {
    pub const MAX_RECORDS: usize = 3;
    pub const MAX_DISTANCES: usize = 8;

    pub fn observe(
        &mut self,
        candle: u64,
        gain: f64,
        buy_sell_gain: f64,
        mrc_zone: i8,
        mrc_cross_mask: u16,
        candle_high: f64,
        candle_low: f64,
        candle_close: f64,
    ) {
        if !gain.is_finite()
            || !buy_sell_gain.is_finite()
            || !candle_high.is_finite()
            || !candle_low.is_finite()
            || !candle_close.is_finite()
        {
            return;
        }
        if gain <= VOLUME_GAIN_THRESHOLD {
            if let Some(record) = self.records.last_mut() {
                if record.candle < candle {
                    record.post_gain_low = record.post_gain_low.min(candle_low);
                    record.post_gain_high = record.post_gain_high.max(candle_high);
                }
            }
            self.low_gain_candles = self.low_gain_candles.saturating_add(1);
        } else {
            let distance = self.last_big_candle.map(|last| candle.saturating_sub(last));
            if let Some(distance) = distance {
                self.distances.push(distance);
                if self.distances.len() > Self::MAX_DISTANCES {
                    self.distances.remove(0);
                }
            }
            self.records.push(VolumeGainRecord {
                candle,
                gain,
                buy_sell_gain,
                mrc_zone,
                mrc_cross_mask,
                distance,
                low_gain_before: self.low_gain_candles,
                post_gain_low: candle_close,
                post_gain_high: candle_close,
            });
            if self.records.len() > Self::MAX_RECORDS {
                self.records.remove(0);
            }
            self.last_big_candle = Some(candle);
            self.low_gain_candles = 0;
        }
    }

    pub fn risk(&self) -> &'static str {
        match self.low_gain_candles {
            0..=7 => "normal",
            8..=15 => "building",
            _ => "huge-move",
        }
    }

    pub fn average_distance(&self, limit: usize) -> Option<f64> {
        let values = self.distances.iter().rev().take(limit).copied();
        let mut count = 0usize;
        let mut total = 0u64;
        for value in values {
            count += 1;
            total = total.saturating_add(value);
        }
        (count > 0).then_some(total as f64 / count as f64)
    }

    pub fn summary(&self) -> String {
        let records = self
            .records
            .iter()
            .rev()
            .map(|record| {
                format!(
                    "c={} g={} bsg={} mrc=z{}:{} liq={} d={} low={} post={:.4}..{:.4}",
                    record.candle,
                    fmt_metric(record.gain, 2),
                    fmt_metric(record.buy_sell_gain, 2),
                    record.mrc_zone,
                    format_mrc_lines(record.mrc_cross_mask),
                    is_liquidation_cross(record.mrc_cross_mask),
                    record
                        .distance
                        .map(|distance| distance.to_string())
                        .unwrap_or_else(|| "na".to_string()),
                    record.low_gain_before,
                    record.post_gain_low,
                    record.post_gain_high
                )
            })
            .collect::<Vec<_>>()
            .join(";");
        format!(
            "[{}] dist=avg3:{} avg8:{} low={} risk={}",
            if records.is_empty() { "na" } else { &records },
            format_average(self.average_distance(3)),
            format_average(self.average_distance(8)),
            self.low_gain_candles,
            self.risk()
        )
    }
}

fn format_average(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.1}c"))
        .unwrap_or_else(|| "na".to_string())
}

fn format_mrc_lines(mask: u16) -> String {
    let lines = ["db", "do", "ds", "di", "m", "ui", "us", "uo", "ub"];
    let crossed = lines
        .iter()
        .enumerate()
        .filter(|(index, _)| mask & (1 << index) != 0)
        .map(|(_, line)| *line)
        .collect::<Vec<_>>();
    if crossed.is_empty() {
        "na".to_string()
    } else {
        crossed.join(",")
    }
}

fn is_liquidation_cross(mask: u16) -> bool {
    mask & ((1 << 0) | (1 << 1) | (1 << 7) | (1 << 8)) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_last_three_volume_gains_and_low_gain_risk() {
        let mut tracker = VolumeGainTracker::default();
        tracker.observe(1, 2.1, -0.4, -1, 1 << 0, 102.0, 99.0, 101.0);
        tracker.observe(2, 1.0, 0.1, 0, 0, 103.0, 98.0, 100.0);
        tracker.observe(5, 2.5, 0.8, 1, (1 << 7) | (1 << 8), 104.0, 97.0, 102.0);

        assert_eq!(tracker.records.len(), 2);
        assert_eq!(tracker.records[1].distance, Some(4));
        assert_eq!(tracker.records[1].buy_sell_gain, 0.8);
        assert_eq!(tracker.records[0].post_gain_low, 98.0);
        assert_eq!(tracker.records[0].post_gain_high, 103.0);
        assert_eq!(tracker.average_distance(3), Some(4.0));
        assert_eq!(tracker.low_gain_candles, 0);
    }

    #[test]
    fn low_gain_candles_raise_huge_move_risk() {
        let mut tracker = VolumeGainTracker::default();
        for candle in 1..=16 {
            tracker.observe(candle, 1.9, 0.0, 0, 0, 101.0, 99.0, 100.0);
        }
        assert_eq!(tracker.low_gain_candles, 16);
        assert_eq!(tracker.risk(), "huge-move");
    }
}
