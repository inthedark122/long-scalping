use crate::builder::BuiltStrategy;
use crate::martingale::LadderProfile;
use crate::model::MarketState;
use crate::model::{Config, EntryReason, ExitMemory, ExitMode, IndicatorSnapshot, PositionState};
use lte_strategy_bridge::abi::Direction;
use lte_strategy_bridge::indicator_access::IndicatorHistory;

#[derive(Debug, Clone, Default)]
pub struct MartingaleRuntime {
    pub profile: LadderProfile,
    pub base_price: f64,
    pub base_amount: f64,
    pub last_filled_mark: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PositionRuntime {
    pub side: PositionState,
    pub entry_price: f64,
    pub peak_price: f64,
    pub trough_price: f64,
    pub take_profit: f64,
    pub stop_loss: f64,
    pub exit_tp_reliable: bool,
    pub exit_mode: ExitMode,
    pub tp_mark: String,
    pub entry_reason: Option<EntryReason>,
    pub martingale: Option<MartingaleRuntime>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct VSideState {
    pub count: u8,
    pub last_power: f64,
    pub prev_power: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct VLineState {
    pub was_above: Option<bool>,
    pub peak: Option<f64>,
    pub trough: Option<f64>,
    pub long_side: VSideState,
    pub short_side: VSideState,
    pub last_cross_tick: u64,
    pub last_cross_dir: i8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CascadeState {
    pub last_close: f64,
    pub dir: i8,
    pub run_extreme: f64,
    pub run_size: f64,
    pub avg_size: f64,
    pub samples: u32,
    pub last_break: f64,
    pub cas_dir: i8,
    pub cas_high: f64,
    pub cas_low: f64,
    pub cas_candles: u32,
    pub sizes: [f64; 30],
    pub sizes_pos: usize,
    pub sizes_len: usize,
    pub breaks: [f64; 5],
    pub break_n: usize,
}

#[derive(Debug, Clone)]
pub struct DirectionLockState {
    pub direction: MarketState,
    pub break_price: f64,
    pub last_tick: u64,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct PendingEntryState {
    pub direction: MarketState,
    pub armed_tick: u64,
    pub price: f64,
    pub reason: String,
}

impl Default for PendingEntryState {
    fn default() -> Self {
        Self {
            direction: MarketState::Sideways,
            armed_tick: 0,
            price: 0.0,
            reason: "na".to_string(),
        }
    }
}

impl Default for DirectionLockState {
    fn default() -> Self {
        Self {
            direction: MarketState::Sideways,
            break_price: 0.0,
            last_tick: 0,
            reason: "na".to_string(),
        }
    }
}

impl DirectionLockState {
    pub fn label(&self) -> String {
        if self.direction == MarketState::Sideways {
            return "na".to_string();
        }
        format!(
            "{:?}:{:.4}:{}",
            self.direction, self.break_price, self.reason
        )
    }
}

pub struct EngineState {
    pub ticks: u64,
    pub bar_1m: u64,
    pub cfg: Config,
    pub cfg_loaded: bool,
    pub history: IndicatorHistory,
    pub position: PositionRuntime,
    pub trade_budget: f64,
    pub vlines: [VLineState; 9],
    pub direction_lock: DirectionLockState,
    pub movement_entry_pending: Option<PendingEntryState>,
    pub cascade_tf: CascadeState,
    pub cascade_sug: CascadeState,
    pub exit_memory: ExitMemory,
    pub strategy_builder: Option<BuiltStrategy>,
    pub last_error: Option<String>,
    pub last_snapshot: Option<IndicatorSnapshot>,
}

impl Default for EngineState {
    fn default() -> Self {
        Self {
            ticks: 0,
            bar_1m: 0,
            cfg: Config::default(),
            cfg_loaded: false,
            history: IndicatorHistory::default(),
            position: PositionRuntime::default(),
            trade_budget: 0.0,
            vlines: [VLineState::default(); 9],
            direction_lock: DirectionLockState::default(),
            movement_entry_pending: None,
            cascade_tf: CascadeState::default(),
            cascade_sug: CascadeState::default(),
            exit_memory: ExitMemory::default(),
            strategy_builder: None,
            last_error: None,
            last_snapshot: None,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedState {
    ticks: u64,
    bar_1m: u64,
    cfg: Config,
    cfg_loaded: bool,
    trade_budget: f64,
    position: PositionState,
    exit_memory: ExitMemory,
}

pub fn save(state: &EngineState) -> Option<serde_json::Value> {
    serde_json::to_value(PersistedState {
        ticks: state.ticks,
        bar_1m: state.bar_1m,
        cfg: state.cfg.clone(),
        cfg_loaded: state.cfg_loaded,
        trade_budget: state.trade_budget,
        position: state.position.side,
        exit_memory: state.exit_memory,
    })
    .ok()
}

pub fn sync_position_state(
    state: &mut EngineState,
    long: Option<(f64, f64)>,
    short: Option<(f64, f64)>,
    price: f64,
) {
    match (long, short) {
        (Some((entry, _)), None) => {
            if state.position.side != PositionState::Long
                || (state.position.entry_price - entry).abs() > f64::EPSILON
            {
                state.position = PositionRuntime {
                    side: PositionState::Long,
                    entry_price: entry,
                    peak_price: price.max(entry),
                    trough_price: price.min(entry),
                    take_profit: 0.0,
                    stop_loss: 0.0,
                    exit_tp_reliable: false,
                    exit_mode: ExitMode::Structural,
                    tp_mark: String::new(),
                    entry_reason: state.position.entry_reason.clone(),
                    martingale: state.position.martingale.clone(),
                };
            } else {
                state.position.peak_price = state.position.peak_price.max(price);
                state.position.trough_price = state.position.trough_price.min(price);
            }
        }
        (None, Some((entry, _))) => {
            if state.position.side != PositionState::Short
                || (state.position.entry_price - entry).abs() > f64::EPSILON
            {
                state.position = PositionRuntime {
                    side: PositionState::Short,
                    entry_price: entry,
                    peak_price: price.max(entry),
                    trough_price: price.min(entry),
                    take_profit: 0.0,
                    stop_loss: 0.0,
                    exit_tp_reliable: false,
                    exit_mode: ExitMode::Structural,
                    tp_mark: String::new(),
                    entry_reason: state.position.entry_reason.clone(),
                    martingale: state.position.martingale.clone(),
                };
            } else {
                state.position.peak_price = state.position.peak_price.max(price);
                state.position.trough_price = state.position.trough_price.min(price);
            }
        }
        _ => {
            state.position = PositionRuntime::default();
        }
    }
}

pub fn dir_to_position(direction: Direction) -> PositionState {
    match direction {
        Direction::Long => PositionState::Long,
        Direction::Short => PositionState::Short,
        Direction::Both => PositionState::Flat,
    }
}
