use crate::builder::BuiltStrategy;
use crate::detection::levels::LevelPoint;
use crate::model::MarketState;
use crate::model::{
    exit_type_for_strategy, Config, EntryReason, ExitMemory, ExitMode, ExitType, IndicatorSnapshot,
    PositionState,
};
use crate::observer::{DirectionChangeHistory, TpRepriceHistory, VolumeGainTracker};
use crate::risk_control::MrcRiskManager;
use lte_strategy_bridge::abi::Direction;
use lte_strategy_bridge::indicator_access::IndicatorHistory;

#[derive(Debug, Clone, Default)]
pub struct PositionRuntime {
    pub side: PositionState,
    pub entry_price: f64,
    pub peak_price: f64,
    pub trough_price: f64,
    pub take_profit: f64,
    pub stop_loss: f64,
    pub stop_loss_reason: String,
    pub exit_tp_reliable: bool,
    pub exit_mode: ExitMode,
    pub exit_type: ExitType,
    pub entry_reason: Option<EntryReason>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct VLineState {
    pub was_above: Option<bool>,
    pub last_cross_tick: u64,
    pub last_cross_dir: i8,
    pub cross_count: u32,
    pub last_cross_price: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct MrcCrossingState {
    pub last_price: Option<f64>,
    pub candle_cross_mask: u16,
    pub sequence: [u8; 8],
    pub sequence_times_ms: [u64; 8],
    pub sequence_len: usize,
    pub no_pullback_count: u32,
    pub last_line: Option<u8>,
    pub last_direction: i8,
    pub reverted_long_line: Option<u8>,
    pub reverted_short_line: Option<u8>,
    pub movement_start: f64,
    pub movement_target: f64,
    pub movement_high: f64,
    pub movement_low: f64,
    pub movement_extreme: f64,
    pub movement_direction: i8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MrcTransitionStats {
    pub count: u32,
    pub duration_sum_ms: u64,
}

impl MrcTransitionStats {
    pub fn average_duration_ms(self) -> Option<f64> {
        (self.count > 0).then_some(self.duration_sum_ms as f64 / self.count as f64)
    }
}

#[derive(Debug, Clone)]
pub struct MrcReclaimState {
    pub first_seen_ms: [u64; 9],
    pub last_cross_ms: [u64; 9],
    pub source_counts: [u32; 9],
    pub transitions: [[MrcTransitionStats; 9]; 9],
    pub scenario_source_counts: [u32; 9],
    pub scenario_transitions: [[MrcTransitionStats; 9]; 9],
    pub recent_lines: [u8; 32],
    pub recent_line_times_ms: [u64; 32],
    pub recent_len: usize,
    pub last_event_small_ntps: bool,
    pub last_event_frequent_nearest: bool,
    pub last_line: Option<u8>,
    pub last_direction: i8,
    pub last_event_ms: u64,
    pub last_price: Option<f64>,
    pub wait_direction: i8,
    pub pending_entry: Option<EntryReason>,
}

impl Default for MrcReclaimState {
    fn default() -> Self {
        Self {
            first_seen_ms: [0; 9],
            last_cross_ms: [0; 9],
            source_counts: [0; 9],
            transitions: [[MrcTransitionStats::default(); 9]; 9],
            scenario_source_counts: [0; 9],
            scenario_transitions: [[MrcTransitionStats::default(); 9]; 9],
            recent_lines: [0; 32],
            recent_line_times_ms: [0; 32],
            recent_len: 0,
            last_event_small_ntps: false,
            last_event_frequent_nearest: false,
            last_line: None,
            last_direction: 0,
            last_event_ms: 0,
            last_price: None,
            wait_direction: 0,
            pending_entry: None,
        }
    }
}

impl Default for MrcCrossingState {
    fn default() -> Self {
        Self {
            last_price: None,
            candle_cross_mask: 0,
            sequence: [0; 8],
            sequence_times_ms: [0; 8],
            sequence_len: 0,
            no_pullback_count: 0,
            last_line: None,
            last_direction: 0,
            reverted_long_line: None,
            reverted_short_line: None,
            movement_start: 0.0,
            movement_target: 0.0,
            movement_high: 0.0,
            movement_low: 0.0,
            movement_extreme: 0.0,
            movement_direction: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CascadeMovement {
    pub high: f64,
    pub low: f64,
    pub dir: i8,
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
    pub movement_history: [CascadeMovement; 30],
    pub movement_history_pos: usize,
    pub movement_history_len: usize,
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

pub struct EngineState {
    pub ticks: u64,
    pub bar_1m: u64,
    pub cfg: Config,
    pub cfg_loaded: bool,
    /// Direction preference received by the start signal. Entry guard only.
    pub allowed_direction: Direction,
    /// Last 1m bar for which the active-position TP was recalculated.
    pub last_active_tp_recalc_bar: Option<u64>,
    pub history: IndicatorHistory,
    pub position: PositionRuntime,
    pub trade_budget: f64,
    pub vlines: [VLineState; 9],
    pub mrc_crossing: MrcCrossingState,
    pub mrc_reclaim: MrcReclaimState,
    pub structural_stop_loss: Option<LevelPoint>,
    pub processed_structural_stop_losses: Vec<LevelPoint>,
    pub volume_gain: VolumeGainTracker,
    pub direction_history: DirectionChangeHistory,
    pub direction_source: String,
    pub direction_reason: String,
    pub tp_reprice_history: TpRepriceHistory,
    pub mrc_risk: MrcRiskManager,
    pub movement_entry_pending: Option<PendingEntryState>,
    pub cascade_tf: CascadeState,
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
            allowed_direction: Direction::Both,
            last_active_tp_recalc_bar: None,
            history: IndicatorHistory::default(),
            position: PositionRuntime::default(),
            trade_budget: 0.0,
            vlines: [VLineState::default(); 9],
            mrc_crossing: MrcCrossingState::default(),
            mrc_reclaim: MrcReclaimState::default(),
            structural_stop_loss: None,
            processed_structural_stop_losses: Vec::new(),
            volume_gain: VolumeGainTracker::default(),
            direction_history: DirectionChangeHistory::default(),
            direction_source: "regime".to_string(),
            direction_reason: "na".to_string(),
            tp_reprice_history: TpRepriceHistory::default(),
            mrc_risk: MrcRiskManager::default(),
            movement_entry_pending: None,
            cascade_tf: CascadeState::default(),
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
    if long.is_none() && short.is_none() {
        state.tp_reprice_history.reset();
    }

    match (long, short) {
        (Some((entry, _)), None) => {
            if state.position.side != PositionState::Long
                || (state.position.entry_price - entry).abs() > f64::EPSILON
            {
                state.structural_stop_loss = None;
                state.processed_structural_stop_losses.clear();
                state.position = PositionRuntime {
                    side: PositionState::Long,
                    entry_price: entry,
                    peak_price: price.max(entry),
                    trough_price: price.min(entry),
                    take_profit: 0.0,
                    stop_loss: 0.0,
                    stop_loss_reason: String::new(),
                    exit_tp_reliable: false,
                    exit_mode: ExitMode::Structural,
                    exit_type: exit_type_for_strategy(state.cfg.strategy_mode),
                    entry_reason: state.position.entry_reason.clone(),
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
                state.structural_stop_loss = None;
                state.processed_structural_stop_losses.clear();
                state.position = PositionRuntime {
                    side: PositionState::Short,
                    entry_price: entry,
                    peak_price: price.max(entry),
                    trough_price: price.min(entry),
                    take_profit: 0.0,
                    stop_loss: 0.0,
                    stop_loss_reason: String::new(),
                    exit_tp_reliable: false,
                    exit_mode: ExitMode::Structural,
                    exit_type: exit_type_for_strategy(state.cfg.strategy_mode),
                    entry_reason: state.position.entry_reason.clone(),
                };
            } else {
                state.position.peak_price = state.position.peak_price.max(price);
                state.position.trough_price = state.position.trough_price.min(price);
            }
        }
        _ => {
            state.structural_stop_loss = None;
            state.processed_structural_stop_losses.clear();
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
