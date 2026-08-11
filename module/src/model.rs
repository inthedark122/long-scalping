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
pub enum StrategyMode {
    Stairs,
    MrcReclaim,
}

impl StrategyMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stairs => "stairs",
            Self::MrcReclaim => "mrc-reclaim",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExitType {
    LevelExit,
    VirtualTrailingExit { ticks: u32 },
}

pub const STAIRS_EXIT_TYPE: ExitType = ExitType::LevelExit;
pub const MRC_RECLAIM_EXIT_TYPE: ExitType = ExitType::VirtualTrailingExit { ticks: 2 };

pub fn exit_type_for_strategy(strategy_mode: StrategyMode) -> ExitType {
    match strategy_mode {
        StrategyMode::Stairs => STAIRS_EXIT_TYPE,
        StrategyMode::MrcReclaim => MRC_RECLAIM_EXIT_TYPE,
    }
}

impl Default for StrategyMode {
    fn default() -> Self {
        Self::Stairs
    }
}

impl Default for ExitType {
    fn default() -> Self {
        STAIRS_EXIT_TYPE
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
    pub strategy_mode: StrategyMode,
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
            min_tp_percent: 0.0,
            strategy_mode: StrategyMode::default(),
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
pub struct SmcSnapshot {
    pub direction: i8,
    pub direction_swing: i8,
    pub fp: Option<f64>,
    pub high_price: Option<f64>,
    pub low_price: Option<f64>,
    pub break_price: Option<f64>,
    pub ob_direction: Option<i8>,
    pub ob_high: Option<f64>,
    pub ob_low: Option<f64>,
    pub sbp: Option<f64>,
}

impl SmcSnapshot {
    pub fn allows_long(&self) -> bool {
        self.direction >= 0
    }

    pub fn allows_short(&self) -> bool {
        self.direction <= 0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TimeframeSnapshot {
    pub candle: Candle,
    pub mrc: MrcBands,
    #[serde(default)]
    pub mrc_signal: MrcSignal,
    pub zz_dir: i8,
    pub zz_buy_price: Option<f64>,
    pub zz_sell_price: Option<f64>,
    pub zz_break_price: f64,
    #[serde(default)]
    pub smc: SmcSnapshot,
    pub volume_gain: f64,
    pub volume_gain_min: f64,
    pub volume_gain_max: f64,
    pub buy_sell_gain: f64,
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
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SugInfoSnapshot {
    pub fast_candle: Option<FastCandleSnapshot>,
    pub sug_candle: Option<Candle>,
    pub signal: SignalIndicatorSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EntryReason {
    SugInfoScalp {
        price: f64,
    },
    SugInfoMovement {
        price: f64,
    },
    SidewayScalp {
        price: f64,
        block_low: f64,
        block_high: f64,
        source: String,
    },
    Structural {
        price: f64,
    },
    MrcReclaim {
        price: f64,
        target_price: f64,
        stop_price: f64,
        source_line: u8,
        target_line: u8,
        probability: f64,
        estimated_time_ms: f64,
    },
}

impl EntryReason {
    pub fn price(&self) -> f64 {
        match self {
            Self::SugInfoScalp { price, .. } => *price,
            Self::SugInfoMovement { price, .. } => *price,
            Self::SidewayScalp { price, .. } => *price,
            Self::Structural { price } => *price,
            Self::MrcReclaim { price, .. } => *price,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::SugInfoScalp { .. } => "sug-info-scalp",
            Self::SugInfoMovement { .. } => "sug-info-movement",
            Self::SidewayScalp { .. } => "sideway-scalp",
            Self::Structural { .. } => "structural",
            Self::MrcReclaim { .. } => "mrc-reclaim",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExitMode {
    Structural,
    TrailAfterVirtualSl,
}

impl Default for ExitMode {
    fn default() -> Self {
        Self::Structural
    }
}

impl ExitMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Structural => "structural",
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

    #[test]
    fn strategy_mode_defaults_to_stairs_and_round_trips() {
        assert_eq!(Config::default().strategy_mode, StrategyMode::Stairs);
        let config: Config = serde_json::from_value(serde_json::json!({
            "strategy_mode": "MrcReclaim"
        }))
        .expect("strategy mode should deserialize from Start state");
        assert_eq!(config.strategy_mode, StrategyMode::MrcReclaim);
        assert_eq!(config.strategy_mode.as_str(), "mrc-reclaim");
    }

    #[test]
    fn strategy_exit_type_is_hardcoded_per_strategy() {
        assert_eq!(
            exit_type_for_strategy(StrategyMode::Stairs),
            ExitType::LevelExit
        );
        assert_eq!(
            exit_type_for_strategy(StrategyMode::MrcReclaim),
            ExitType::VirtualTrailingExit { ticks: 2 }
        );
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct RegimeSnapshot {
    pub primary: MarketState,
    pub fallback: MarketState,
    pub confirmed: MarketState,
    pub fallback_used: bool,
    pub context_1h: MarketState,
    pub blocked_by_context: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EntryWaitSolution {
    WaitForSamples {
        required: u32,
        current: u32,
    },
    WaitForDirectionBreak {
        source: String,
        direction: MarketState,
        break_price: f64,
    },
    WaitForConfirmation {
        source: String,
        direction: MarketState,
        armed_tick: u64,
        price: f64,
    },
    ReclaimLevel {
        source: String,
        level: f64,
        band_low: f64,
        band_high: f64,
    },
    FastCandle {
        phase: FastCandlePhase,
        fast_candle_pct: f64,
        candle_pct: f64,
    },
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
