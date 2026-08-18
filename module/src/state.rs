use crate::detection::decision::{DecisionComponents, DecisionState};
use crate::model::{
    Config, EntryReason, ExitMemory, ExitMode, ExitType, IndicatorSnapshot, PositionState,
    MRC_CROSSING_EXIT_TYPE,
};
use crate::observer::{TpRepriceHistory, VolumeGainTracker};
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
    pub movement_market_start: f64,
    pub movement_target: f64,
    pub movement_target_line: Option<u8>,
    pub movement_target_low: f64,
    pub movement_target_high: f64,
    pub movement_mrc_low: f64,
    pub movement_mrc_high: f64,
    pub movement_high: f64,
    pub movement_low: f64,
    pub movement_extreme: f64,
    pub movement_direction: i8,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MrcMovementSnapshot {
    pub start_line: u8,
    pub start_price: f64,
    pub market_start_price: f64,
    pub target_price: f64,
    pub target_low: f64,
    pub target_high: f64,
    pub mrc_low: f64,
    pub mrc_high: f64,
    pub high: f64,
    pub low: f64,
    pub extreme: f64,
    pub direction: i8,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PatternPerformance {
    pub observations: u64,
    pub trades: u64,
    pub wins: u64,
    pub pnl_sum_pct: f64,
    pub ema_pnl_pct: f64,
    pub last_pnl_pct: f64,
}

impl PatternPerformance {
    pub fn observe(&mut self) {
        self.observations = self.observations.saturating_add(1);
    }

    pub fn record_trade(&mut self, pnl_pct: f64) {
        if !pnl_pct.is_finite() {
            return;
        }
        self.trades = self.trades.saturating_add(1);
        self.wins = self.wins.saturating_add(u64::from(pnl_pct > 0.0));
        self.pnl_sum_pct += pnl_pct;
        self.last_pnl_pct = pnl_pct;
        self.ema_pnl_pct = if self.trades == 1 {
            pnl_pct
        } else {
            self.ema_pnl_pct * 0.80 + pnl_pct * 0.20
        };
    }

    pub fn win_rate(self) -> f64 {
        if self.trades == 0 {
            0.0
        } else {
            self.wins as f64 / self.trades as f64
        }
    }

    pub fn average_pnl_pct(self) -> f64 {
        if self.trades == 0 {
            0.0
        } else {
            self.pnl_sum_pct / self.trades as f64
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MrcForecastStats {
    pub count: u32,
    pub movement_samples: u32,
    pub next_line_cross_count: u32,
    pub support_resistance_count: u32,
    pub price_only_count: u32,
    pub success_count: u32,
    pub failure_count: u32,
    pub duration_sum_ms: u64,
    pub favorable_pct_sum: f64,
    pub adverse_pct_sum: f64,
    pub outcome_pct_sum: f64,
}

impl MrcForecastStats {
    pub fn average_duration_ms(self) -> Option<f64> {
        (self.count > 0).then_some(self.duration_sum_ms as f64 / self.count as f64)
    }

    pub fn average_favorable_pct(self) -> Option<f64> {
        (self.movement_samples > 0).then_some(self.favorable_pct_sum / self.movement_samples as f64)
    }

    pub fn average_adverse_pct(self) -> Option<f64> {
        (self.movement_samples > 0).then_some(self.adverse_pct_sum / self.movement_samples as f64)
    }

    pub fn movement_success_rate(self) -> Option<f64> {
        (self.movement_samples > 0)
            .then_some(self.success_count as f64 / self.movement_samples as f64)
    }

    pub fn expected_value_pct(self) -> Option<f64> {
        (self.movement_samples > 0).then_some(self.outcome_pct_sum / self.movement_samples as f64)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MrcHistoryEvent {
    pub line: u8,
    pub direction: i8,
    pub now_ms: u64,
    pub market_price: f64,
    pub mrc_low: f64,
    pub mrc_high: f64,
    pub no_pullback_count: u32,
    pub reverted_line: Option<u8>,
    pub completed_movement: Option<MrcMovementSnapshot>,
}

#[derive(Debug, Clone)]
pub struct MrcCrossingModel {
    /// Recent crossing cycles used for local transition learning. The detector
    /// keeps this bounded instead of matching against the complete history.
    pub history: Vec<MrcHistoryEvent>,
    /// Longer-lived crossing history used to estimate exact transition and
    /// fast-break outcomes. Pattern classification still uses `history` so a
    /// stale market regime cannot change the local pattern shape.
    pub prediction_history: Vec<MrcHistoryEvent>,
    pub seeded_from_history: bool,
    pub last_line: Option<u8>,
    pub last_direction: i8,
    pub last_event_ms: u64,
    pub current_entry: Option<EntryReason>,
    pub pattern_performance: [PatternPerformance; 7],
    pub active_pattern: Option<u8>,
    pub active_pattern_score: f64,
    pub active_pattern_since_ms: u64,
    pub last_plan_key: Option<(u8, u8, u8)>,
    pub decision_components: DecisionComponents,
    pub decision_state: DecisionState,
    /// Explicit direction reservation from a directional start signal.
    /// Direction::Both leaves this unset so the strategy may switch sides.
    pub trade_direction_lock: Option<i8>,
}

impl Default for MrcCrossingModel {
    fn default() -> Self {
        Self {
            history: Vec::new(),
            prediction_history: Vec::new(),
            seeded_from_history: false,
            last_line: None,
            last_direction: 0,
            last_event_ms: 0,
            current_entry: None,
            pattern_performance: [PatternPerformance::default(); 7],
            active_pattern: None,
            active_pattern_score: 0.0,
            active_pattern_since_ms: 0,
            last_plan_key: None,
            decision_components: DecisionComponents::default(),
            decision_state: DecisionState::default(),
            trade_direction_lock: None,
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
            movement_market_start: 0.0,
            movement_target: 0.0,
            movement_target_line: None,
            movement_target_low: 0.0,
            movement_target_high: 0.0,
            movement_mrc_low: 0.0,
            movement_mrc_high: 0.0,
            movement_high: 0.0,
            movement_low: 0.0,
            movement_extreme: 0.0,
            movement_direction: 0,
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
    pub mrc_crossing_model: MrcCrossingModel,
    pub pending_entry_bar: Option<u64>,
    pub volume_gain: VolumeGainTracker,
    pub direction_source: String,
    pub direction_reason: String,
    pub tp_reprice_history: TpRepriceHistory,
    pub mrc_risk: MrcRiskManager,
    pub exit_memory: ExitMemory,
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
            mrc_crossing_model: MrcCrossingModel::default(),
            pending_entry_bar: None,
            volume_gain: VolumeGainTracker::default(),
            direction_source: "pattern-selector".to_string(),
            direction_reason: "na".to_string(),
            tp_reprice_history: TpRepriceHistory::default(),
            mrc_risk: MrcRiskManager::default(),
            exit_memory: ExitMemory::default(),
            last_error: None,
            last_snapshot: None,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedState {
    #[serde(default)]
    ticks: u64,
    #[serde(default)]
    bar_1m: u64,
    #[serde(default)]
    cfg: Config,
    #[serde(default)]
    cfg_loaded: bool,
    #[serde(default)]
    trade_budget: f64,
    #[serde(default)]
    position: PositionState,
    #[serde(default)]
    exit_memory: ExitMemory,
    #[serde(default)]
    pattern_performance: [PatternPerformance; 7],
    #[serde(default)]
    active_pattern: Option<u8>,
    #[serde(default)]
    active_pattern_score: f64,
    #[serde(default)]
    decision_state: DecisionState,
    #[serde(default)]
    trade_direction_lock: Option<i8>,
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
        pattern_performance: state.mrc_crossing_model.pattern_performance,
        active_pattern: state.mrc_crossing_model.active_pattern,
        active_pattern_score: state.mrc_crossing_model.active_pattern_score,
        decision_state: state.mrc_crossing_model.decision_state.clone(),
        trade_direction_lock: state.mrc_crossing_model.trade_direction_lock,
    })
    .ok()
}

pub fn restore(state: &mut EngineState, value: Option<&serde_json::Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let Ok(persisted) = serde_json::from_value::<PersistedState>(value.clone()) else {
        return false;
    };

    state.ticks = persisted.ticks;
    state.bar_1m = persisted.bar_1m;
    state.cfg = persisted.cfg;
    state.cfg_loaded = persisted.cfg_loaded;
    state.trade_budget = persisted.trade_budget;
    state.position.side = persisted.position;
    state.exit_memory = persisted.exit_memory;
    state.mrc_crossing_model.pattern_performance = persisted.pattern_performance;
    state.mrc_crossing_model.active_pattern = persisted.active_pattern.filter(|index| *index < 7);
    state.mrc_crossing_model.active_pattern_score = persisted.active_pattern_score;
    state.mrc_crossing_model.decision_components =
        DecisionComponents::from_names(&state.cfg.decision_components).0;
    state.mrc_crossing_model.decision_state = persisted.decision_state;
    state.mrc_crossing_model.trade_direction_lock = persisted.trade_direction_lock;
    state
        .mrc_crossing_model
        .decision_state
        .reconfigure(state.mrc_crossing_model.decision_components);
    true
}

pub fn sync_position_state(
    state: &mut EngineState,
    long: Option<(f64, f64)>,
    short: Option<(f64, f64)>,
    price: f64,
) {
    if long.is_none() && short.is_none() {
        if state.position.side != PositionState::Flat {
            // The plan key only suppresses duplicate offers for the current
            // position. Once the host reports flat, the primary selector may
            // offer a fresh same-direction route again.
            state.mrc_crossing_model.last_plan_key = None;
        }
        state.tp_reprice_history.reset();
        state.pending_entry_bar = None;
    }

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
                    stop_loss_reason: String::new(),
                    exit_tp_reliable: false,
                    exit_mode: ExitMode::Fixed,
                    exit_type: MRC_CROSSING_EXIT_TYPE,
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
                state.position = PositionRuntime {
                    side: PositionState::Short,
                    entry_price: entry,
                    peak_price: price.max(entry),
                    trough_price: price.min(entry),
                    take_profit: 0.0,
                    stop_loss: 0.0,
                    stop_loss_reason: String::new(),
                    exit_tp_reliable: false,
                    exit_mode: ExitMode::Fixed,
                    exit_type: MRC_CROSSING_EXIT_TYPE,
                    entry_reason: state.position.entry_reason.clone(),
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

pub fn direction_sign(direction: Direction) -> Option<i8> {
    match direction {
        Direction::Long => Some(1),
        Direction::Short => Some(-1),
        Direction::Both => None,
    }
}
