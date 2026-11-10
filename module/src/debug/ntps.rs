use std::cmp::Reverse;
use std::collections::BinaryHeap;

const DAY_MS: i64 = 86_400_000;
const BANGKOK_OFFSET_MS: i64 = 7 * 60 * 60 * 1_000;

#[derive(Default)]
pub(crate) struct NtpsDailyStats {
    local_day: Option<i64>,
    count: u64,
    sum: i128,
    min: Option<i64>,
    max: Option<i64>,
    lower: BinaryHeap<i64>,
    upper: BinaryHeap<Reverse<i64>>,
}

impl NtpsDailyStats {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn observe(&mut self, time_ms: i64, ntp_5s: i64) {
        if time_ms < 0 || ntp_5s < 0 {
            return;
        }

        let local_day = time_ms.saturating_add(BANGKOK_OFFSET_MS).div_euclid(DAY_MS);
        if self.local_day != Some(local_day) {
            self.reset();
            self.local_day = Some(local_day);
        }

        self.count += 1;
        self.sum += i128::from(ntp_5s);
        self.min = Some(self.min.map_or(ntp_5s, |value| value.min(ntp_5s)));
        self.max = Some(self.max.map_or(ntp_5s, |value| value.max(ntp_5s)));

        if self.lower.peek().map_or(true, |value| ntp_5s <= *value) {
            self.lower.push(ntp_5s);
        } else {
            self.upper.push(Reverse(ntp_5s));
        }

        let p90_rank = self.count - self.count / 10;
        while self.lower.len() as u64 > p90_rank {
            if let Some(value) = self.lower.pop() {
                self.upper.push(Reverse(value));
            }
        }
        while (self.lower.len() as u64) < p90_rank {
            if let Some(Reverse(value)) = self.upper.pop() {
                self.lower.push(value);
            }
        }
    }

    pub(crate) fn summary(&self) -> String {
        match (self.count, self.min, self.max, self.lower.peek()) {
            (0, _, _, _) => "count=0".to_string(),
            (count, Some(min), Some(max), Some(p90)) => format!(
                "count={count} min={min} max={max} average={:.2} p90={p90}",
                self.sum as f64 / count as f64
            ),
            _ => "count=0".to_string(),
        }
    }
}
