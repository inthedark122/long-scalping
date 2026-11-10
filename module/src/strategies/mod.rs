mod execution;
pub mod sideways;
pub mod trending;

use crate::detection::{movement_window, mrc_cross_analysis::MrcCrossAnalysis};
use crate::model::{Candle, EntryType, MrcBands};
use crate::pattern_matching::{Forecast, PatternKind};
use lte_strategy_bridge::abi::Direction;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    #[default]
    Trending,
    Sideways,
}

impl StrategyKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trending => "trending",
            Self::Sideways => "sideways",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntryPlan {
    pub strategy: StrategyKind,
    pub direction: Direction,
    pub entry_price: f64,
    pub take_profit: Option<f64>,
    pub stop_loss: f64,
    pub reason: &'static str,
    pub note: String,
    pub entry_type: Option<EntryType>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EntryDecision {
    Wait {
        strategy: StrategyKind,
        reason: &'static str,
    },
    Enter(EntryPlan),
}

#[derive(Debug, Clone, Default)]
pub struct StrategyRuntime {
    last_sideways_movement_start_ms: Option<u64>,
    stable_sideways_range: crate::detection::stable_range::StableRangeState,
}

impl StrategyRuntime {
    pub fn record_sideways_movement(&mut self, movement_start_ms: u64) {
        if movement_start_ms > 0 {
            self.last_sideways_movement_start_ms = Some(movement_start_ms);
        }
    }

    pub fn last_sideways_movement_start_ms(&self) -> Option<u64> {
        self.last_sideways_movement_start_ms
    }

    pub fn restore_last_sideways_movement_start_ms(&mut self, movement_start_ms: Option<u64>) {
        self.last_sideways_movement_start_ms = movement_start_ms.filter(|value| *value > 0);
    }

    pub fn update_stable_sideways_range(
        &mut self,
        cascade: &crate::detection::cascade::CascadeState,
        candidate: Option<movement_window::MovementWindow>,
        price: f64,
    ) -> Option<movement_window::MovementWindow> {
        self.stable_sideways_range.update(cascade, candidate, price)
    }

    pub fn stable_sideways_range_bounds(&self) -> Option<(f64, f64)> {
        self.stable_sideways_range.bounds()
    }

    pub fn stable_sideways_range_state(&self) -> crate::detection::stable_range::StableRangeState {
        self.stable_sideways_range
    }

    pub fn restore_stable_sideways_range(
        &mut self,
        stable_range: crate::detection::stable_range::StableRangeState,
    ) {
        self.stable_sideways_range = stable_range;
    }

    pub fn reset_stable_sideways_range(&mut self) {
        self.stable_sideways_range.reset();
    }

    pub fn reset(&mut self) {
        self.last_sideways_movement_start_ms = None;
        self.stable_sideways_range.reset();
    }
}

impl EntryDecision {
    pub fn strategy(&self) -> StrategyKind {
        match self {
            Self::Wait { strategy, .. } => *strategy,
            Self::Enter(plan) => plan.strategy,
        }
    }

    pub fn direction(&self) -> Option<Direction> {
        match self {
            Self::Wait { .. } => None,
            Self::Enter(plan) => Some(plan.direction),
        }
    }
}

pub fn evaluate_entry(
    analysis: Option<&MrcCrossAnalysis>,
    detected_pattern: Option<PatternKind>,
    pattern_forecast: Option<Forecast>,
    detected_trend_direction: Option<i8>,
    mrc: MrcBands,
    candle: Candle,
    market_price: f64,
    tick_size: f64,
    movement_window: Option<movement_window::MovementWindow>,
    stable_sideways_range: Option<movement_window::MovementWindow>,
    last_sideways_movement_start_ms: Option<u64>,
    cascade_movement_start_ms: u64,
    cascade_direction: i8,
    sideways_order_pending: bool,
    trend_only: bool,
    sideways_only: bool,
    sideways_nonstop: bool,
    allowed_direction: Direction,
) -> EntryDecision {
    if trend_only && detected_pattern == Some(PatternKind::MeanRange) {
        return EntryDecision::Wait {
            strategy: StrategyKind::Trending,
            reason: "trend-only-rejects-mean-range",
        };
    }

    if !trend_only && sideways_nonstop {
        let Some(window) = stable_sideways_range else {
            return EntryDecision::Wait {
                strategy: StrategyKind::Sideways,
                reason: "stable-range-not-confirmed",
            };
        };
        return sideways::evaluate(
            window,
            market_price,
            tick_size,
            cascade_direction,
            cascade_movement_start_ms,
            last_sideways_movement_start_ms,
            sideways_order_pending,
            pattern_forecast,
            true,
        );
    }

    if !trend_only && (sideways_only || detected_pattern == Some(PatternKind::MeanRange)) {
        let Some(window) = movement_window else {
            return EntryDecision::Wait {
                strategy: StrategyKind::Sideways,
                reason: "movement-window-not-ready",
            };
        };
        if !sideways::is_range(&window) {
            return EntryDecision::Wait {
                strategy: StrategyKind::Sideways,
                reason: "movement-window-not-confirmed",
            };
        }
        return sideways::evaluate(
            window,
            market_price,
            tick_size,
            cascade_direction,
            cascade_movement_start_ms,
            last_sideways_movement_start_ms,
            sideways_order_pending,
            pattern_forecast,
            sideways_nonstop,
        );
    }

    trending::entry::as_entry_decision(trending::entry::evaluate(
        analysis,
        detected_trend_direction,
        mrc,
        candle,
        market_price,
        trend_only,
        allowed_direction,
    ))
}

pub fn execute_entry(
    input: &lte_strategy_bridge::abi::ModuleInput,
    state: &mut crate::state::EngineState,
    decision: EntryDecision,
) -> execution::Execution {
    execution::execute(input, state, decision)
}

pub use execution::Execution;
