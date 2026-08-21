use serde::{Deserialize, Serialize};

use crate::detection::mrc::MrcSignal;
use crate::format::fmt_price;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MarketState {
    Long,
    Short,
    #[default]
    Sideways,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PositionState {
    #[default]
    Flat,
    Long,
    Short,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryType {
    Market,
    Limit,
}

impl EntryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Market => "market",
            Self::Limit => "limit",
        }
    }
}

impl Default for EntryType {
    fn default() -> Self {
        Self::Limit
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExitType {
    LevelExit,
    VirtualTrailingExit { ticks: u32 },
}

pub const MRC_CROSSING_EXIT_TYPE: ExitType = ExitType::VirtualTrailingExit { ticks: 2 };

impl Default for ExitType {
    fn default() -> Self {
        MRC_CROSSING_EXIT_TYPE
    }
}

impl ExitType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LevelExit => "level-exit",
            Self::VirtualTrailingExit { .. } => "virtual-trailing-exit",
        }
    }

    pub fn ticks(self) -> u32 {
        match self {
            Self::LevelExit => 0,
            Self::VirtualTrailingExit { ticks } => ticks.max(1),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_cap_init")]
    pub cap_init: f64,
    #[serde(default = "default_trading_amount")]
    pub trading_amount: f64,
    #[serde(default)]
    pub min_tp_percent: f64,
    #[serde(default)]
    pub min_sl_percent: f64,
    #[serde(default)]
    pub risk_target_percent: f64,
    #[serde(default)]
    pub entry_type: EntryType,
    #[serde(default = "default_required_samples")]
    pub required_samples: u32,
    #[serde(default = "default_decision_components")]
    pub decision_components: Vec<String>,
}

fn default_cap_init() -> f64 {
    50.0
}

fn default_trading_amount() -> f64 {
    50.0
}

fn default_required_samples() -> u32 {
    3
}

fn default_decision_components() -> Vec<String> {
    [
        "primary-stairs",
        "support-fast-break-return",
        "support-pullback",
        "support-volume",
        "support-extreme",
        "support-trend",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cap_init: default_cap_init(),
            trading_amount: default_trading_amount(),
            min_tp_percent: 0.0,
            min_sl_percent: 0.0,
            risk_target_percent: 0.0,
            entry_type: EntryType::default(),
            required_samples: default_required_samples(),
            decision_components: default_decision_components(),
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

    pub fn min_tp_percent(&self) -> f64 {
        if self.min_tp_percent.is_finite() {
            self.min_tp_percent.max(0.0)
        } else {
            0.0
        }
    }

    pub fn min_sl_percent(&self) -> f64 {
        if self.min_sl_percent.is_finite() {
            self.min_sl_percent.max(0.0)
        } else {
            0.0
        }
    }

    pub fn risk_target_percent(&self) -> f64 {
        if self.risk_target_percent.is_finite() {
            self.risk_target_percent.max(0.0)
        } else {
            0.0
        }
    }

    pub fn required_samples(&self) -> u32 {
        self.required_samples
    }

    pub fn entry_type(&self) -> EntryType {
        self.entry_type
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
    #[serde(default)]
    pub mrc_signal: MrcSignal,
    pub volume_gain: f64,
    pub volume_gain_min: f64,
    pub volume_gain_max: f64,
    pub buy_sell_gain: f64,
    /// Average trades per second-window from the NTPS indicator.
    #[serde(default)]
    pub ntps: i64,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EntryReason {
    MrcCrossing {
        price: f64,
        market_price: f64,
        mrc_min: f64,
        mrc_max: f64,
        target_price: f64,
        stop_price: f64,
        source_line: u8,
        target_line: u8,
        pattern_kind: String,
        pattern: Vec<u8>,
        pattern_elapsed_ms: Vec<u64>,
        pattern_samples: u32,
        context_samples: u32,
        no_pullback_count: u32,
        reverted_line: Option<u8>,
        target_samples: u32,
        movement_samples: u32,
        next_line_crosses: u32,
        support_resistance_moves: u32,
        price_only_moves: u32,
        successful_moves: u32,
        failed_moves: u32,
        movement_success_rate: f64,
        expected_value_pct: f64,
        expected_price_low: f64,
        expected_price_high: f64,
        estimated_time_ms: f64,
        favorable_move_pct: f64,
        adverse_move_pct: f64,
    },
}

impl EntryReason {
    pub fn price(&self) -> f64 {
        match self {
            Self::MrcCrossing { price, .. } => *price,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::MrcCrossing { .. } => "mrc-crossing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExitMode {
    Fixed,
    TrailAfterVirtualSl,
}

impl Default for ExitMode {
    fn default() -> Self {
        Self::Fixed
    }
}

impl ExitMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::TrailAfterVirtualSl => "trail-after-virtual-sl",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ExitMemory {
    pub side: PositionState,
    pub kind: Option<ExitKind>,
    pub price: f64,
    pub take_profit: f64,
    pub stop_loss: f64,
    pub reliable: bool,
    pub tick: u64,
}

impl ExitMemory {
    pub fn summary(&self) -> String {
        match self.kind {
            Some(kind) if self.price > 0.0 => format!(
                "side={:?} kind={:?} price={} tp={} sl={} reliable={} tick={}",
                self.side,
                kind,
                fmt_price(self.price),
                fmt_price(self.take_profit),
                fmt_price(self.stop_loss),
                self.reliable,
                if self.tick == 0 {
                    "na".to_string()
                } else {
                    self.tick.to_string()
                }
            ),
            _ => "exit-mem=na".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExitKind {
    TakeProfit,
    StopLoss,
}

impl Default for ExitKind {
    fn default() -> Self {
        Self::TakeProfit
    }
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
        ((self.volume_gain - self.volume_gain_min) / (self.volume_gain_max - self.volume_gain_min))
            .clamp(0.0, 1.0)
    }

    pub fn gain_is_building(&self) -> bool {
        matches!(self.gain_zone(), GainZone::Building)
    }

    pub fn gain_is_late(&self) -> bool {
        matches!(self.gain_zone(), GainZone::Late)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct IndicatorSnapshot {
    pub tf1m: TimeframeSnapshot,
    pub tf5m: TimeframeSnapshot,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_uses_pattern_engine_defaults() {
        assert_eq!(Config::default().entry_type(), EntryType::Limit);
        let config: Config = serde_json::from_value(serde_json::json!({
            "entry_type": "Market"
        }))
        .expect("entry setting should deserialize");
        assert_eq!(config.entry_type(), EntryType::Market);
    }

    #[test]
    fn required_samples_defaults_and_allows_zero() {
        assert_eq!(Config::default().required_samples(), 3);
        let config: Config = serde_json::from_value(serde_json::json!({
            "required_samples": 0
        }))
        .expect("sample setting should deserialize");
        assert_eq!(config.required_samples(), 0);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EntryWaitSolution {
    MrcCrossing,
    MrcDirectionalTarget,
    WaitForSamples { required: u32, current: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryWait {
    pub reason: String,
    pub solutions: Vec<EntryWaitSolution>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EntryGate {
    Open {
        notes: Vec<String>,
        reason: EntryOpenReason,
    },
    Wait(EntryWait),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EntryOpenReason {
    Entry(EntryReason),
}

impl EntryOpenReason {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Entry(reason) => reason.label(),
        }
    }

    pub fn price(&self) -> Option<f64> {
        match self {
            Self::Entry(reason) => Some(reason.price()),
        }
    }

    pub fn as_entry_reason(&self) -> Option<&EntryReason> {
        match self {
            Self::Entry(reason) => Some(reason),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeDecision {
    pub market_state: MarketState,
    pub entry: EntryGate,
}
