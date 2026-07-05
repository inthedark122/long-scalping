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
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct IndicatorSnapshot {
    pub tf1m: TimeframeSnapshot,
    pub tf5m: TimeframeSnapshot,
    pub tf15m: TimeframeSnapshot,
    pub tf1h: TimeframeSnapshot,
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
