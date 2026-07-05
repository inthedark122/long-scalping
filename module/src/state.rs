use crate::model::{Config, PositionState};
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
}

pub fn save(state: &EngineState) -> Option<serde_json::Value> {
    serde_json::to_value(PersistedState {
        ticks: state.ticks,
        bar_1m: state.bar_1m,
        cfg: state.cfg.clone(),
        cfg_loaded: state.cfg_loaded,
        trade_budget: state.trade_budget,
        position: state.position.side,
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
