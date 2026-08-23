use crate::detection::cascade::{CascadeSignal, CascadeState, CascadeTradeState};
use crate::detection::decision::{DecisionComponents, DecisionState};
use crate::detection::mrc_cross_analysis::MrcCrossAnalysis;
use crate::detection::mrc_direction_prediction::MrcDirectionPrediction;
use crate::detection::mrc_pattern_classification::{
    MrcPatternKind, StaircaseLifecycle, StaircaseTransition, MRC_PATTERN_COUNT,
};
use crate::detection::mrc_prediction::MrcPredictionContext;
use crate::detection::mrc_touch::MrcTouchState;
use crate::detection::mrc_touch_learning::MrcTouchLearningState;
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionLifetime {
    #[default]
    OneShot,
    RecyclableStairs,
}

impl PositionLifetime {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OneShot => "one-shot",
            Self::RecyclableStairs => "recyclable-stairs",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionExitPlanSource {
    #[default]
    None,
    DynamicSameSide,
    LastValidPositionPlan,
    EntryAnchor,
    EmergencyClose,
}

impl PositionExitPlanSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::DynamicSameSide => "dynamic",
            Self::LastValidPositionPlan => "cached",
            Self::EntryAnchor => "entry-anchor",
            Self::EmergencyClose => "emergency-close",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionDecisionAction {
    #[default]
    Hold,
    UpdateExit,
    Close,
}

impl PositionDecisionAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::UpdateExit => "update-exit",
            Self::Close => "close",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PositionPlanSnapshot {
    pub take_profit: f64,
    pub stop_loss: f64,
    pub tp_reliable: bool,
}

impl PositionPlanSnapshot {
    pub fn valid_for(self, side: PositionState, reference_price: f64) -> bool {
        if !self.take_profit.is_finite()
            || !self.stop_loss.is_finite()
            || self.take_profit <= 0.0
            || self.stop_loss <= 0.0
            || reference_price <= 0.0
        {
            return false;
        }
        match side {
            PositionState::Long => {
                self.take_profit > reference_price && self.stop_loss < reference_price
            }
            PositionState::Short => {
                self.take_profit < reference_price && self.stop_loss > reference_price
            }
            PositionState::Flat => false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PositionContext {
    pub side: PositionState,
    pub entry_price: f64,
    pub entry_time_ms: u64,
    pub entry_reason: Option<EntryReason>,
    pub entry_anchor: Option<PositionPlanSnapshot>,
    pub last_valid_plan: Option<PositionPlanSnapshot>,
    pub last_plan_source: PositionExitPlanSource,
    pub last_action: PositionDecisionAction,
    pub decision_reason: String,
    pub lifetime: PositionLifetime,
    pub cycle_started_ms: u64,
    pub cycle_direction: i8,
    pub cycle_pattern: Vec<u8>,
    pub consecutive_entries: u32,
    pub reentry_allowed: bool,
    pub reentry_reason: String,
}

impl PositionContext {
    pub const MAX_RECYCLABLE_ENTRIES: u32 = 8;
    pub const MAX_CYCLE_AGE_MS: u64 = 6 * 60 * 60 * 1_000;

    pub fn begin(
        &mut self,
        side: PositionState,
        entry_price: f64,
        entry_time_ms: u64,
        entry_reason: Option<EntryReason>,
        lifetime: PositionLifetime,
        cycle_pattern: Vec<u8>,
    ) {
        let same_cycle = self.cycle_pattern.is_empty()
            || cycle_pattern.is_empty()
            || self.cycle_pattern == cycle_pattern;
        let previous_entries = if self.lifetime == lifetime
            && self.side == PositionState::Flat
            && lifetime == PositionLifetime::RecyclableStairs
            && self.cycle_is_valid(entry_time_ms)
            && same_cycle
        {
            self.consecutive_entries
        } else {
            0
        };
        let cycle_started_ms = if previous_entries > 0 && self.cycle_started_ms > 0 {
            self.cycle_started_ms
        } else {
            entry_time_ms
        };
        let cycle_direction = match side {
            PositionState::Long => 1,
            PositionState::Short => -1,
            PositionState::Flat => 0,
        };
        *self = Self {
            side,
            entry_price,
            entry_time_ms,
            entry_reason,
            entry_anchor: None,
            last_valid_plan: None,
            last_plan_source: PositionExitPlanSource::None,
            last_action: PositionDecisionAction::Hold,
            decision_reason: "position-started".to_string(),
            lifetime,
            cycle_started_ms,
            cycle_direction,
            cycle_pattern,
            consecutive_entries: previous_entries.saturating_add(1),
            reentry_allowed: lifetime == PositionLifetime::RecyclableStairs,
            reentry_reason: if lifetime == PositionLifetime::RecyclableStairs {
                "active-stairs-cycle".to_string()
            } else {
                "one-shot-position".to_string()
            },
        };
    }

    pub fn remember_plan(&mut self, plan: PositionPlanSnapshot, source: PositionExitPlanSource) {
        if !plan.take_profit.is_finite() || !plan.stop_loss.is_finite() {
            return;
        }
        self.last_valid_plan = Some(plan);
        self.last_plan_source = source;
        if self.entry_anchor.is_none() {
            self.entry_anchor = Some(plan);
        }
    }

    pub fn seed_entry_anchor_from_reason(&mut self, reason: Option<&EntryReason>) {
        if self.entry_anchor.is_some() {
            return;
        }
        let Some(EntryReason::MrcCrossing {
            target_price,
            stop_price,
            ..
        }) = reason
        else {
            return;
        };
        let plan = PositionPlanSnapshot {
            take_profit: *target_price,
            stop_loss: *stop_price,
            tp_reliable: true,
        };
        if plan.take_profit.is_finite() && plan.stop_loss.is_finite() {
            self.entry_anchor = Some(plan);
        }
    }

    pub fn on_flat(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        if self.lifetime == PositionLifetime::RecyclableStairs
            && self.reentry_allowed
            && self.consecutive_entries < Self::MAX_RECYCLABLE_ENTRIES
        {
            self.side = PositionState::Flat;
            self.entry_price = 0.0;
            self.entry_time_ms = 0;
            self.entry_reason = None;
            self.entry_anchor = None;
            self.last_valid_plan = None;
            self.last_plan_source = PositionExitPlanSource::None;
            self.last_action = PositionDecisionAction::Hold;
            self.decision_reason = "position-finished-reentry-context-retained".to_string();
            self.reentry_reason = reason;
        } else {
            *self = Self {
                side: PositionState::Flat,
                reentry_allowed: false,
                reentry_reason: reason,
                ..Self::default()
            };
        }
    }

    pub fn disable_reentry(&mut self, reason: impl Into<String>) {
        self.reentry_allowed = false;
        self.reentry_reason = reason.into();
    }

    pub fn set_decision(&mut self, action: PositionDecisionAction, reason: impl Into<String>) {
        self.last_action = action;
        self.decision_reason = reason.into();
    }

    pub fn cycle_is_valid(&self, now_ms: u64) -> bool {
        self.lifetime == PositionLifetime::RecyclableStairs
            && self.reentry_allowed
            && self.consecutive_entries < Self::MAX_RECYCLABLE_ENTRIES
            && (self.cycle_started_ms == 0
                || now_ms.saturating_sub(self.cycle_started_ms) <= Self::MAX_CYCLE_AGE_MS)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExitProgressMode {
    #[default]
    Primary,
    Continuation,
    Stalled,
}

impl ExitProgressMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Continuation => "continuation",
            Self::Stalled => "stalled",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StaircaseProgress {
    pub direction: i8,
    pub lifecycle: Option<StaircaseLifecycle>,
    pub transition: Option<StaircaseTransition>,
    pub protected_line: Option<u8>,
    pub extreme_line: Option<u8>,
    pub favorable_extreme_price: f64,
    pub favorable_extreme_time_ms: u64,
    pub adverse_extreme_price: f64,
    pub adverse_extreme_time_ms: u64,
    pub pattern_elapsed_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExitProgress {
    pub side: PositionState,
    pub mode: ExitProgressMode,
    pub entry_time_ms: u64,
    pub last_extreme_price: f64,
    pub last_extreme_update_ms: u64,
    pub far_target_price: f64,
    pub orders_initialized: bool,
    pub last_ntps: i64,
    pub ntps_rising_count: u32,
    pub last_volume_gain: f64,
    pub volume_rising_count: u32,
}

impl ExitProgress {
    pub fn begin(&mut self, side: PositionState, entry_time_ms: u64, entry_price: f64) {
        *self = Self {
            side,
            mode: ExitProgressMode::Primary,
            entry_time_ms,
            last_extreme_price: entry_price,
            last_extreme_update_ms: entry_time_ms,
            ..Self::default()
        };
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn observe_activity(&mut self, ntps: i64, volume_gain: f64) {
        if self.side == PositionState::Flat {
            return;
        }
        if ntps > self.last_ntps && self.last_ntps > 0 {
            self.ntps_rising_count = self.ntps_rising_count.saturating_add(1);
        } else if ntps < self.last_ntps {
            self.ntps_rising_count = 0;
        }
        if volume_gain.is_finite() {
            if self.last_volume_gain.is_finite() && volume_gain > self.last_volume_gain {
                self.volume_rising_count = self.volume_rising_count.saturating_add(1);
            } else if volume_gain < self.last_volume_gain {
                self.volume_rising_count = 0;
            }
            self.last_volume_gain = volume_gain;
        }
        self.last_ntps = ntps;
    }

    pub fn observe_extreme(&mut self, direction: i8, price: f64, time_ms: u64) {
        if self.side == PositionState::Flat
            || direction != position_direction(self.side)
            || !price.is_finite()
            || price <= 0.0
            || time_ms < self.entry_time_ms
        {
            return;
        }
        let improved = match self.side {
            PositionState::Long => price > self.last_extreme_price,
            PositionState::Short => price < self.last_extreme_price,
            PositionState::Flat => false,
        };
        if improved {
            self.last_extreme_price = price;
            self.last_extreme_update_ms = time_ms;
        }
    }

    pub fn activity_supports_continuation(self, no_pullback_count: u32) -> bool {
        no_pullback_count > 0 && self.ntps_rising_count > 0 && self.volume_rising_count > 0
    }
}

fn position_direction(side: PositionState) -> i8 {
    match side {
        PositionState::Long => 1,
        PositionState::Short => -1,
        PositionState::Flat => 0,
    }
}

/// Fixed quantity allocation for the partial take-profit legs of one
/// position. The allocation is created from the quantity produced by the
/// calculated entry amount and is never re-based on the shrinking live
/// position after a TP fill.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TakeProfitAllocation {
    #[serde(default)]
    pub side: PositionState,
    #[serde(default)]
    pub total_qty: f64,
    #[serde(default)]
    pub planned_qty: [f64; 3],
    #[serde(default)]
    pub filled_qty: [f64; 3],
}

impl Default for TakeProfitAllocation {
    fn default() -> Self {
        Self {
            side: PositionState::Flat,
            total_qty: 0.0,
            planned_qty: [0.0; 3],
            filled_qty: [0.0; 3],
        }
    }
}

impl TakeProfitAllocation {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn begin(&mut self, side: PositionState, qty: f64) {
        let qty = qty.abs();
        if !matches!(side, PositionState::Long | PositionState::Short) || qty <= 0.0 {
            return;
        }
        if self.side != side || self.total_qty <= 0.0 {
            *self = Self {
                side,
                total_qty: qty,
                planned_qty: [0.0; 3],
                filled_qty: [0.0; 3],
            };
            return;
        }

        // A later NewPosition notification can reflect an additional same-
        // side fill. Keep the existing TP history and extend the allocation
        // only when the host reports a larger aggregate position.
        self.total_qty = self.total_qty.max(qty);
        let planned_total: f64 = self.planned_qty.iter().sum();
        if planned_total > 0.0 && self.total_qty > planned_total {
            self.planned_qty[2] += self.total_qty - planned_total;
        }
    }

    pub fn record_fill(&mut self, side: PositionState, leg: usize, qty: f64) {
        if self.side != side || leg >= self.filled_qty.len() || !qty.is_finite() || qty <= 0.0 {
            return;
        }
        self.filled_qty[leg] = self.filled_qty[leg].max(qty.abs());
    }

    pub fn has_plan(&self, side: PositionState) -> bool {
        self.side == side && self.planned_qty.iter().any(|qty| *qty > 0.0)
    }
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
    pub pattern_performance: [PatternPerformance; MRC_PATTERN_COUNT],
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
            pattern_performance: [PatternPerformance::default(); MRC_PATTERN_COUNT],
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
    /// Latest same-side staircase analysis retained for the open position's
    /// exit controller. It is intentionally not replaced by an opposite-side
    /// market analysis.
    pub active_mrc_analysis: Option<MrcCrossAnalysis>,
    /// Latest same-side non-indicator/cascade context paired with the active
    /// position analysis.
    pub active_mrc_context: MrcPredictionContext,
    /// Latest selected market analysis, regardless of the held position side.
    /// This context is used for future entries and transitions.
    pub market_mrc_analysis: Option<MrcCrossAnalysis>,
    /// Latest market context paired with `market_mrc_analysis`.
    pub market_mrc_context: MrcPredictionContext,
    pub history: IndicatorHistory,
    pub position: PositionRuntime,
    /// Immutable and cached decision context owned by the current position.
    pub position_context: PositionContext,
    pub take_profit_allocation: TakeProfitAllocation,
    pub trade_budget: f64,
    pub vlines: [VLineState; 9],
    pub mrc_crossing: MrcCrossingState,
    /// Time-decayed per-line touch pressure used as context for stairs.
    pub mrc_touch: MrcTouchState,
    /// Continuous direction prediction derived from the current MRC touch
    /// state and the selected learned route.
    pub mrc_direction_prediction: MrcDirectionPrediction,
    /// Persistent empirical outcomes of exact MRC-line touch episodes.
    pub mrc_touch_learning: MrcTouchLearningState,
    pub mrc_crossing_model: MrcCrossingModel,
    pub cascade: CascadeState,
    pub cascade_signal: CascadeSignal,
    pub selected_cascade_trade_state: CascadeTradeState,
    /// Latest staircase extreme/protected-swing information used by exits.
    pub staircase_progress: StaircaseProgress,
    /// One-way exit state. Cascade contributes timing/activity context only.
    pub exit_progress: ExitProgress,
    pub pending_entry_bar: Option<u64>,
    pub volume_gain: VolumeGainTracker,
    pub direction_source: String,
    pub direction_reason: String,
    pub tp_reprice_history: TpRepriceHistory,
    pub mrc_risk: MrcRiskManager,
    pub exit_memory: ExitMemory,
    /// Last meaningful exit lifecycle event, kept in the runtime debug line.
    pub last_exit_event: String,
    /// One-shot context explaining why the in-memory position runtime was
    /// rebuilt before the next exit evaluation.
    pub pending_exit_context: Option<String>,
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
            active_mrc_analysis: None,
            active_mrc_context: MrcPredictionContext::default(),
            market_mrc_analysis: None,
            market_mrc_context: MrcPredictionContext::default(),
            history: IndicatorHistory::default(),
            position: PositionRuntime::default(),
            position_context: PositionContext::default(),
            take_profit_allocation: TakeProfitAllocation::default(),
            trade_budget: 0.0,
            vlines: [VLineState::default(); 9],
            mrc_crossing: MrcCrossingState::default(),
            mrc_touch: MrcTouchState::default(),
            mrc_direction_prediction: MrcDirectionPrediction::default(),
            mrc_touch_learning: MrcTouchLearningState::default(),
            mrc_crossing_model: MrcCrossingModel::default(),
            cascade: CascadeState::default(),
            cascade_signal: CascadeSignal::default(),
            selected_cascade_trade_state: CascadeTradeState::NotConfigured,
            staircase_progress: StaircaseProgress::default(),
            exit_progress: ExitProgress::default(),
            pending_entry_bar: None,
            volume_gain: VolumeGainTracker::default(),
            direction_source: "pattern-selector".to_string(),
            direction_reason: "na".to_string(),
            tp_reprice_history: TpRepriceHistory::default(),
            mrc_risk: MrcRiskManager::default(),
            exit_memory: ExitMemory::default(),
            last_exit_event: String::new(),
            pending_exit_context: None,
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
    position_context: PositionContext,
    #[serde(default)]
    take_profit_allocation: TakeProfitAllocation,
    #[serde(default)]
    exit_memory: ExitMemory,
    #[serde(default)]
    pattern_performance: Vec<PatternPerformance>,
    #[serde(default)]
    active_pattern: Option<u8>,
    #[serde(default)]
    active_pattern_score: f64,
    #[serde(default)]
    decision_state: DecisionState,
    #[serde(default)]
    trade_direction_lock: Option<i8>,
    #[serde(default)]
    mrc_touch_learning: MrcTouchLearningState,
    #[serde(default)]
    exit_progress: ExitProgress,
}

pub fn save(state: &EngineState) -> Option<serde_json::Value> {
    serde_json::to_value(PersistedState {
        ticks: state.ticks,
        bar_1m: state.bar_1m,
        cfg: state.cfg.clone(),
        cfg_loaded: state.cfg_loaded,
        trade_budget: state.trade_budget,
        position: state.position.side,
        position_context: state.position_context.clone(),
        take_profit_allocation: state.take_profit_allocation,
        exit_memory: state.exit_memory,
        pattern_performance: state.mrc_crossing_model.pattern_performance.to_vec(),
        active_pattern: state.mrc_crossing_model.active_pattern,
        active_pattern_score: state.mrc_crossing_model.active_pattern_score,
        decision_state: state.mrc_crossing_model.decision_state.clone(),
        trade_direction_lock: state.mrc_crossing_model.trade_direction_lock,
        mrc_touch_learning: state.mrc_touch_learning.clone(),
        exit_progress: state.exit_progress,
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
    state.cfg.decision_components = DecisionComponents::from_names(&state.cfg.decision_components)
        .0
        .names();
    state.cfg_loaded = persisted.cfg_loaded;
    state.trade_budget = persisted.trade_budget;
    state.position.side = persisted.position;
    state.position_context = persisted.position_context;
    state.take_profit_allocation = persisted.take_profit_allocation;
    state.exit_memory = persisted.exit_memory;
    state.mrc_crossing_model.pattern_performance =
        [PatternPerformance::default(); MRC_PATTERN_COUNT];
    for (slot, performance) in persisted
        .pattern_performance
        .into_iter()
        .enumerate()
        .take(MRC_PATTERN_COUNT)
    {
        state.mrc_crossing_model.pattern_performance[slot] = performance;
    }
    state.mrc_crossing_model.active_pattern = persisted
        .active_pattern
        .filter(|index| MrcPatternKind::ALL.get(*index as usize).is_some());
    state.mrc_crossing_model.active_pattern_score = persisted.active_pattern_score;
    state.mrc_crossing_model.decision_components =
        DecisionComponents::from_names(&state.cfg.decision_components).0;
    state.mrc_crossing_model.decision_state = persisted.decision_state;
    state.mrc_crossing_model.trade_direction_lock = persisted.trade_direction_lock;
    state.mrc_touch_learning = persisted.mrc_touch_learning;
    state.exit_progress = persisted.exit_progress;
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
            state.position_context.on_flat("server-confirmed-flat");
        }
        state.take_profit_allocation.reset();
        state.exit_progress.reset();
        state.tp_reprice_history.reset();
        state.last_exit_event.clear();
        state.pending_exit_context = None;
        state.active_mrc_analysis = None;
        state.active_mrc_context = MrcPredictionContext::default();
        state.pending_entry_bar = None;
    }

    match (long, short) {
        (Some((entry, _)), None) => {
            let entry_reason = state
                .position
                .entry_reason
                .clone()
                .or_else(|| state.position_context.entry_reason.clone());
            if state.position.side != PositionState::Long
                || (state.position.entry_price - entry).abs() > f64::EPSILON
            {
                state.pending_exit_context = Some(format!(
                    "position-sync=runtime-rebuilt side=Long server_entry={:.8} previous_entry={:.8} previous_reason={}",
                    entry,
                    state.position.entry_price,
                    state.position.entry_reason.is_some()
                ));
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
                    entry_reason: entry_reason.clone(),
                };
                if state.position_context.side != PositionState::Long
                    || (state.position_context.entry_price - entry).abs() > f64::EPSILON
                {
                    state.position_context.begin(
                        PositionState::Long,
                        entry,
                        state.position_context.entry_time_ms,
                        entry_reason.clone(),
                        PositionLifetime::OneShot,
                        Vec::new(),
                    );
                }
            } else {
                state.position.peak_price = state.position.peak_price.max(price);
                state.position.trough_price = state.position.trough_price.min(price);
            }
            state.position_context.entry_reason = entry_reason;
            state
                .position_context
                .seed_entry_anchor_from_reason(state.position.entry_reason.as_ref());
        }
        (None, Some((entry, _))) => {
            let entry_reason = state
                .position
                .entry_reason
                .clone()
                .or_else(|| state.position_context.entry_reason.clone());
            if state.position.side != PositionState::Short
                || (state.position.entry_price - entry).abs() > f64::EPSILON
            {
                state.pending_exit_context = Some(format!(
                    "position-sync=runtime-rebuilt side=Short server_entry={:.8} previous_entry={:.8} previous_reason={}",
                    entry,
                    state.position.entry_price,
                    state.position.entry_reason.is_some()
                ));
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
                    entry_reason: entry_reason.clone(),
                };
                if state.position_context.side != PositionState::Short
                    || (state.position_context.entry_price - entry).abs() > f64::EPSILON
                {
                    state.position_context.begin(
                        PositionState::Short,
                        entry,
                        state.position_context.entry_time_ms,
                        entry_reason.clone(),
                        PositionLifetime::OneShot,
                        Vec::new(),
                    );
                }
            } else {
                state.position.peak_price = state.position.peak_price.max(price);
                state.position.trough_price = state.position.trough_price.min(price);
            }
            state.position_context.entry_reason = entry_reason;
            state
                .position_context
                .seed_entry_anchor_from_reason(state.position.entry_reason.as_ref());
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
