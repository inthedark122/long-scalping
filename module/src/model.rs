use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MarketState {
    Long,
    Short,
    #[default]
    Sideways,
}

impl MarketState {
    pub fn from_dir_pair(dp_dir: i8, zz_dir: i8) -> Self {
        match (dp_dir, zz_dir) {
            (1, 1) => Self::Long,
            (-1, -1) => Self::Short,
            _ => Self::Sideways,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PositionState {
    #[default]
    Flat,
    Long,
    Short,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_cap_init")]
    pub cap_init: f64,
    #[serde(default = "default_trading_amount")]
    pub trading_amount: f64,
}

fn default_cap_init() -> f64 {
    2.0
}

fn default_trading_amount() -> f64 {
    50.0
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cap_init: default_cap_init(),
            trading_amount: default_trading_amount(),
        }
    }
}

impl Config {
    pub fn cap_ratio(&self) -> f64 {
        (self.cap_init / 100.0).clamp(0.0, 1.0)
    }

    pub fn trading_ratio(&self) -> f64 {
        (self.trading_amount / 100.0).clamp(0.10, 1.0)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Candle {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct MrcBands {
    pub down_big: f64,
    pub down_outer: f64,
    pub down_small: f64,
    pub down_inner: f64,
    pub mean_line: f64,
    pub up_inner: f64,
    pub up_small: f64,
    pub up_outer: f64,
    pub up_big: f64,
    pub current_cross: f64,
    pub mean_line_slope: f64,
}

impl MrcBands {
    pub fn ordered(&self) -> bool {
        self.down_big < self.down_outer
            && self.down_outer < self.down_small
            && self.down_small < self.down_inner
            && self.down_inner < self.mean_line
            && self.mean_line < self.up_inner
            && self.up_inner < self.up_small
            && self.up_small < self.up_outer
            && self.up_outer < self.up_big
    }

    pub fn range(&self) -> f64 {
        self.up_big - self.down_big
    }

    pub fn lines(&self) -> [f64; 9] {
        [
            self.down_big,
            self.down_outer,
            self.down_small,
            self.down_inner,
            self.mean_line,
            self.up_inner,
            self.up_small,
            self.up_outer,
            self.up_big,
        ]
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TimeframeSnapshot {
    pub candle: Candle,
    pub mrc: MrcBands,
    pub dp_dir: i8,
    pub zz_dir: i8,
    pub dp_zee: f64,
    pub dp_zee1: f64,
    pub zz_buy_price: f64,
    pub zz_sell_price: f64,
    pub zz_break_price: f64,
    pub volume_gain: f64,
    pub volume_gain_min: f64,
    pub volume_gain_max: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GainZone {
    Weak,
    Building,
    Late,
}

impl Default for GainZone {
    fn default() -> Self {
        Self::Weak
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct FastCandleSnapshot {
    pub candle: Candle,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FastCandlePhase {
    Absent,
    Cooling,
    Building,
    Tiring,
}

impl Default for FastCandlePhase {
    fn default() -> Self {
        Self::Absent
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SignalIndicatorSnapshot {
    pub ntps: i64,
    pub asset_01: f64,
    pub asset_diff_01: f64,
}

impl TimeframeSnapshot {
    pub fn gain_zone(&self) -> GainZone {
        if self.volume_gain < 1.0 {
            return GainZone::Weak;
        }
        if self.volume_gain_max > 0.0 && self.volume_gain >= self.volume_gain_max * 0.8 {
            GainZone::Late
        } else {
            GainZone::Building
        }
    }

    pub fn gain_progress_pct(&self) -> f64 {
        if self.volume_gain_max <= self.volume_gain_min {
            return 0.0;
        }
        ((self.volume_gain - self.volume_gain_min)
            / (self.volume_gain_max - self.volume_gain_min))
            .clamp(0.0, 1.0)
    }

    pub fn gain_is_building(&self) -> bool {
        matches!(self.gain_zone(), GainZone::Building)
    }

    pub fn gain_is_late(&self) -> bool {
        matches!(self.gain_zone(), GainZone::Late)
    }
}

impl FastCandleSnapshot {
    pub fn range_pct(&self) -> f64 {
        let price = self.candle.close.max(0.0);
        if price <= 0.0 {
            return 0.0;
        }
        ((self.candle.high - self.candle.low).abs() / price) * 100.0
    }

    pub fn phase(&self, current_candle: &Candle) -> FastCandlePhase {
        if !self.active {
            return if self.range_pct() > 0.0 {
                FastCandlePhase::Cooling
            } else {
                FastCandlePhase::Absent
            };
        }

        let fc_pct = self.range_pct();
        let candle_pct = if current_candle.close > 0.0 {
            ((current_candle.high - current_candle.low).abs() / current_candle.close) * 100.0
        } else {
            0.0
        };

        if fc_pct <= 0.0 {
            FastCandlePhase::Absent
        } else if candle_pct <= 0.40 && fc_pct >= candle_pct * 1.5 {
            FastCandlePhase::Tiring
        } else if fc_pct >= candle_pct {
            FastCandlePhase::Building
        } else {
            FastCandlePhase::Cooling
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct IndicatorSnapshot {
    pub tf1m: TimeframeSnapshot,
    pub tf5m: TimeframeSnapshot,
    pub tf15m: TimeframeSnapshot,
    pub tf1h: TimeframeSnapshot,
    pub fast_candle: Option<FastCandleSnapshot>,
    pub signal: SignalIndicatorSnapshot,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_candle_phase_follows_doc_semantics() {
        let active = FastCandleSnapshot {
            candle: Candle {
                open: 100.0,
                high: 106.0,
                low: 100.0,
                close: 105.0,
            },
            active: true,
        };
        let growing = active.phase(&Candle {
            open: 100.0,
            high: 105.5,
            low: 99.5,
            close: 105.0,
        });
        assert_eq!(growing, FastCandlePhase::Building);

        let tiring = active.phase(&Candle {
            open: 105.0,
            high: 105.3,
            low: 104.9,
            close: 105.05,
        });
        assert_eq!(tiring, FastCandlePhase::Tiring);

        let cooling = FastCandleSnapshot {
            active: false,
            ..active
        }
        .phase(&Candle {
            open: 105.0,
            high: 105.3,
            low: 104.9,
            close: 105.05,
        });
        assert_eq!(cooling, FastCandlePhase::Cooling);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct RegimeSnapshot {
    pub primary: MarketState,
    pub confirmed: MarketState,
    pub fallback_used: bool,
    pub context_1h: MarketState,
    pub blocked_by_context: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct RiskSnapshot {
    pub risk_high: bool,
    pub fail_streak: u32,
    pub active: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TradeDecision {
    pub market_state: MarketState,
    pub allow_entry: bool,
    pub reason: String,
}
