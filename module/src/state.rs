use crate::model::{Config, ExitKind, PositionState};
use lte_strategy_bridge::abi::Direction;
use lte_strategy_bridge::indicator_access::IndicatorHistory;

#[derive(Debug, Clone, Copy, Default)]
pub struct PositionRuntime {
    pub side: PositionState,
    pub entry_price: f64,
    pub peak_price: f64,
    pub trough_price: f64,
    pub take_profit: f64,
    pub stop_loss: f64,
    pub exit_tp_reliable: bool,
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
pub struct SharpReturnState {
    pub active: bool,
    pub direction: i8,
    pub ref_price: f64,
    pub armed_bar: u64,
    pub fail_streak: u32,
    pub last_fail_price: f64,
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
    pub sharp_return: SharpReturnState,
    pub cascade_1m: CascadeState,
    pub cascade_5m: CascadeState,
    pub cascade_15m: CascadeState,
    pub cascade_1h: CascadeState,
    pub last_exit_kind: Option<ExitKind>,
    pub last_exit_side: PositionState,
    pub last_exit_price: f64,
    pub last_exit_tick: u64,
    pub last_exit_tp_reliable: bool,
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
            sharp_return: SharpReturnState::default(),
            cascade_1m: CascadeState::default(),
            cascade_5m: CascadeState::default(),
            cascade_15m: CascadeState::default(),
            cascade_1h: CascadeState::default(),
            last_exit_kind: None,
            last_exit_side: PositionState::Flat,
            last_exit_price: 0.0,
            last_exit_tick: 0,
            last_exit_tp_reliable: false,
        }
    }
}

#[derive(serde::Serialize)]
struct PersistedState {
    ticks: u64,
    bar_1m: u64,
    cfg: Config,
    cfg_loaded: bool,
    trade_budget: f64,
    position: PositionState,
    last_exit_kind: Option<ExitKind>,
    last_exit_side: PositionState,
    last_exit_price: f64,
    last_exit_tick: u64,
    last_exit_tp_reliable: bool,
}

pub fn save(state: &EngineState) -> Option<serde_json::Value> {
    serde_json::to_value(PersistedState {
        ticks: state.ticks,
        bar_1m: state.bar_1m,
        cfg: state.cfg.clone(),
        cfg_loaded: state.cfg_loaded,
        trade_budget: state.trade_budget,
        position: state.position.side,
        last_exit_kind: state.last_exit_kind,
        last_exit_side: state.last_exit_side,
        last_exit_price: state.last_exit_price,
        last_exit_tick: state.last_exit_tick,
        last_exit_tp_reliable: state.last_exit_tp_reliable,
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
