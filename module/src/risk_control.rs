use crate::model::{MrcBands, SugInfoSnapshot, TimeframeSnapshot};

const ANGLE_ZERO_RATIO: f64 = 0.05;
const INITIAL_WARMUP_MS: u64 = 10 * 60 * 1_000;
const PERSISTENT_ANGLE_MS: u64 = 30 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MrcRiskLevel {
    LowImpulse,
    Normal,
    High,
}

impl MrcRiskLevel {
    fn rank(self) -> u8 {
        match self {
            Self::LowImpulse => 0,
            Self::Normal => 1,
            Self::High => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LowImpulse => "low-impulse",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MrcAngle {
    Falling,
    Flat,
    Rising,
}

impl MrcAngle {
    fn from_slope(slope: f64, zero_tolerance: f64) -> Self {
        if slope.abs() <= zero_tolerance {
            Self::Flat
        } else if slope < 0.0 {
            Self::Falling
        } else {
            Self::Rising
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Falling => "falling",
            Self::Flat => "flat",
            Self::Rising => "rising",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MrcZone {
    LowerLowImpulse,
    LowerNormal,
    MeanLowImpulse,
    UpperLowImpulse,
    UpperNormal,
    BeyondDownOuter,
    BeyondUpOuter,
    Invalid,
}

impl MrcZone {
    fn as_str(self) -> &'static str {
        match self {
            Self::LowerLowImpulse => "lower-low",
            Self::LowerNormal => "lower-normal",
            Self::MeanLowImpulse => "mean-low",
            Self::UpperLowImpulse => "upper-low",
            Self::UpperNormal => "upper-normal",
            Self::BeyondDownOuter => "below-down-outer",
            Self::BeyondUpOuter => "above-up-outer",
            Self::Invalid => "invalid",
        }
    }

    fn is_low_impulse(self) -> bool {
        matches!(
            self,
            Self::LowerLowImpulse | Self::MeanLowImpulse | Self::UpperLowImpulse
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MrcTimeframeRisk {
    pub timeframe: &'static str,
    pub level: MrcRiskLevel,
    pub zone: MrcZone,
    pub angle: MrcAngle,
    pub angle_duration_ms: u64,
    pub reasons: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MrcRiskAssessment {
    pub level: MrcRiskLevel,
    pub timeframes: Vec<MrcTimeframeRisk>,
}

impl MrcRiskAssessment {
    pub fn summary(&self) -> String {
        let details = self
            .timeframes
            .iter()
            .map(|risk| {
                let reasons = if risk.reasons.is_empty() {
                    "none".to_string()
                } else {
                    risk.reasons.join(",")
                };
                format!(
                    "{}:{}:{}:{}:{}",
                    risk.timeframe,
                    risk.level.as_str(),
                    risk.zone.as_str(),
                    risk.angle.as_str(),
                    reasons
                )
            })
            .collect::<Vec<_>>()
            .join("|");
        format!("{} [{}]", self.level.as_str(), details)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct MrcTimeframeHistory {
    initialized_at_ms: Option<u64>,
    angle: Option<MrcAngle>,
    angle_started_at_ms: Option<u64>,
    previous_price: Option<f64>,
}

impl MrcTimeframeHistory {
    fn observe(
        &mut self,
        timeframe: &'static str,
        snapshot: &TimeframeSnapshot,
        price: f64,
        now_ms: u64,
        fast_candle_active: bool,
    ) -> MrcTimeframeRisk {
        let mrc = &snapshot.mrc;
        let zone = mrc_zone(mrc, price);
        if zone == MrcZone::Invalid {
            return MrcTimeframeRisk {
                timeframe,
                level: MrcRiskLevel::Normal,
                zone,
                angle: MrcAngle::Flat,
                angle_duration_ms: 0,
                reasons: Vec::new(),
            };
        }
        let zero_tolerance = mrc_zero_tolerance(mrc);
        let angle = MrcAngle::from_slope(mrc.mean_line_slope, zero_tolerance);
        let initialized_at_ms = *self.initialized_at_ms.get_or_insert(now_ms);
        let angle_started_at_ms = match (self.angle, self.angle_started_at_ms) {
            (Some(previous), Some(started)) if previous == angle => started,
            _ => now_ms,
        };
        self.angle = Some(angle);
        self.angle_started_at_ms = Some(angle_started_at_ms);

        let crossed_outer_or_big = self
            .previous_price
            .is_some_and(|previous| crossed_outer_or_big(mrc, previous, price));
        self.previous_price = Some(price);

        let warmup_complete = now_ms.saturating_sub(initialized_at_ms) >= INITIAL_WARMUP_MS;
        let angle_duration_ms = now_ms.saturating_sub(angle_started_at_ms);
        let high_volume = snapshot.gain_is_late();
        let mut reasons = Vec::new();

        if matches!(zone, MrcZone::BeyondDownOuter | MrcZone::BeyondUpOuter) {
            reasons.push("outside-outer");
        }
        if warmup_complete && angle == MrcAngle::Flat {
            reasons.push("angle-flat");
        }
        if warmup_complete && angle != MrcAngle::Flat && angle_duration_ms >= PERSISTENT_ANGLE_MS {
            reasons.push("angle-persistent");
        }
        if crossed_outer_or_big && (fast_candle_active || high_volume) {
            reasons.push(if fast_candle_active && high_volume {
                "outer-big-cross:spike-volume"
            } else if fast_candle_active {
                "outer-big-cross:spike"
            } else {
                "outer-big-cross:volume"
            });
        }

        let level = if reasons.is_empty() {
            if zone.is_low_impulse() {
                MrcRiskLevel::LowImpulse
            } else {
                MrcRiskLevel::Normal
            }
        } else {
            MrcRiskLevel::High
        };

        MrcTimeframeRisk {
            timeframe,
            level,
            zone,
            angle,
            angle_duration_ms,
            reasons,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MrcRiskManager {
    one_minute: MrcTimeframeHistory,
    five_minute: MrcTimeframeHistory,
    latest: Option<MrcRiskAssessment>,
}

impl MrcRiskManager {
    pub fn observe(
        &mut self,
        snapshot: &crate::model::IndicatorSnapshot,
        sug_snapshot: &SugInfoSnapshot,
        price: f64,
        now_ms: u64,
    ) -> MrcRiskAssessment {
        let fast_candle_active = sug_snapshot
            .fast_candle
            .is_some_and(|fast_candle| fast_candle.active);
        let timeframes = vec![
            self.one_minute
                .observe("1m", &snapshot.tf1m, price, now_ms, fast_candle_active),
            self.five_minute
                .observe("5m", &snapshot.tf5m, price, now_ms, fast_candle_active),
        ];
        let level = timeframes
            .iter()
            .map(|risk| risk.level)
            .max_by_key(|risk| risk.rank())
            .unwrap_or(MrcRiskLevel::Normal);
        let assessment = MrcRiskAssessment { level, timeframes };
        self.latest = Some(assessment.clone());
        assessment
    }

    pub fn latest(&self) -> Option<&MrcRiskAssessment> {
        self.latest.as_ref()
    }
}

fn mrc_zero_tolerance(mrc: &MrcBands) -> f64 {
    let lower_width = (mrc.mean_line - mrc.down_inner).abs();
    let upper_width = (mrc.up_inner - mrc.mean_line).abs();
    lower_width.max(upper_width) * ANGLE_ZERO_RATIO
}

fn mrc_zone(mrc: &MrcBands, price: f64) -> MrcZone {
    if !mrc.ordered() || !price.is_finite() || price <= 0.0 {
        return MrcZone::Invalid;
    }

    if price < mrc.down_outer {
        return MrcZone::BeyondDownOuter;
    }
    if price > mrc.up_outer {
        return MrcZone::BeyondUpOuter;
    }

    let lower_midpoint = mrc.down_inner - (mrc.down_inner - mrc.down_small) / 2.0;
    let upper_midpoint = mrc.up_inner + (mrc.up_small - mrc.up_inner) / 2.0;
    if price >= lower_midpoint && price <= upper_midpoint {
        if price < mrc.mean_line {
            MrcZone::LowerLowImpulse
        } else if price > mrc.mean_line {
            MrcZone::UpperLowImpulse
        } else {
            MrcZone::MeanLowImpulse
        }
    } else if price < mrc.mean_line {
        MrcZone::LowerNormal
    } else {
        MrcZone::UpperNormal
    }
}

fn crossed(previous_price: f64, current_price: f64, level: f64, upward: bool) -> bool {
    if upward {
        previous_price < level && current_price >= level
    } else {
        previous_price > level && current_price <= level
    }
}

fn crossed_outer_or_big(mrc: &MrcBands, previous_price: f64, current_price: f64) -> bool {
    crossed(previous_price, current_price, mrc.up_outer, true)
        || crossed(previous_price, current_price, mrc.up_big, true)
        || crossed(previous_price, current_price, mrc.down_outer, false)
        || crossed(previous_price, current_price, mrc.down_big, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Candle, IndicatorSnapshot, MrcBands, SugInfoSnapshot, TimeframeSnapshot};

    fn mrc() -> MrcBands {
        MrcBands {
            down_big: 80.0,
            down_outer: 85.0,
            down_small: 90.0,
            down_inner: 95.0,
            mean_line: 100.0,
            up_inner: 105.0,
            up_small: 110.0,
            up_outer: 115.0,
            up_big: 120.0,
            current_cross: 0.0,
            mean_line_slope: 1.0,
        }
    }

    fn snapshot(mrc: MrcBands) -> IndicatorSnapshot {
        let tf = TimeframeSnapshot {
            candle: Candle {
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.0,
            },
            mrc,
            volume_gain: 1.0,
            volume_gain_min: 1.0,
            volume_gain_max: 10.0,
            ..TimeframeSnapshot::default()
        };
        IndicatorSnapshot {
            tf1m: tf,
            tf5m: tf,
            tf15m: tf,
            tf1h: tf,
        }
    }

    #[test]
    fn classifies_mean_midpoint_as_low_impulse() {
        let mut manager = MrcRiskManager::default();
        let assessment = manager.observe(&snapshot(mrc()), &SugInfoSnapshot::default(), 107.0, 0);
        assert_eq!(assessment.level, MrcRiskLevel::LowImpulse);
        assert_eq!(assessment.timeframes[0].zone, MrcZone::UpperLowImpulse);
    }

    #[test]
    fn outside_outer_is_high() {
        let mut manager = MrcRiskManager::default();
        let assessment = manager.observe(&snapshot(mrc()), &SugInfoSnapshot::default(), 116.0, 0);
        assert_eq!(assessment.level, MrcRiskLevel::High);
        assert!(assessment.timeframes[0].reasons.contains(&"outside-outer"));
    }

    #[test]
    fn flat_angle_waits_for_warmup() {
        let mut bands = mrc();
        bands.mean_line_slope = 0.1;
        let mut manager = MrcRiskManager::default();
        let first = manager.observe(&snapshot(bands), &SugInfoSnapshot::default(), 100.0, 0);
        assert_eq!(first.level, MrcRiskLevel::LowImpulse);

        bands.mean_line_slope = 0.0;
        let later = manager.observe(
            &snapshot(bands),
            &SugInfoSnapshot::default(),
            100.0,
            INITIAL_WARMUP_MS,
        );
        assert_eq!(later.level, MrcRiskLevel::High);
        assert!(later.timeframes[0].reasons.contains(&"angle-flat"));
    }

    #[test]
    fn persistent_angle_is_high_after_thirty_minutes() {
        let mut manager = MrcRiskManager::default();
        let initial = manager.observe(&snapshot(mrc()), &SugInfoSnapshot::default(), 100.0, 0);
        assert_eq!(initial.level, MrcRiskLevel::LowImpulse);
        let later = manager.observe(
            &snapshot(mrc()),
            &SugInfoSnapshot::default(),
            100.0,
            PERSISTENT_ANGLE_MS,
        );
        assert_eq!(later.level, MrcRiskLevel::High);
        assert!(later.timeframes[0].reasons.contains(&"angle-persistent"));
    }

    #[test]
    fn outer_cross_requires_spike_or_volume() {
        let mut manager = MrcRiskManager::default();
        manager.observe(&snapshot(mrc()), &SugInfoSnapshot::default(), 114.0, 0);
        let mut high_gain = mrc();
        high_gain.mean_line_slope = 0.0;
        let mut next = snapshot(high_gain);
        next.tf1m.volume_gain = 9.0;
        next.tf1m.volume_gain_max = 10.0;
        let assessment = manager.observe(&next, &SugInfoSnapshot::default(), 116.0, 1_000);
        assert!(assessment.timeframes[0]
            .reasons
            .contains(&"outer-big-cross:volume"));
    }
}
