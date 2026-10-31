mod execution;
pub mod sideways;
pub mod trending;

use crate::detection::{movement_window, mrc_cross_analysis::MrcCrossAnalysis};
use crate::model::{Candle, EntryType, MrcBands};
use crate::pattern_matching::PatternKind;
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

    pub fn reset(&mut self) {
        self.last_sideways_movement_start_ms = None;
    }
}

impl EntryDecision {
    pub fn strategy(&self) -> StrategyKind {
        match self {
            Self::Wait { strategy, .. } => *strategy,
            Self::Enter(plan) => plan.strategy,
        }
    }
}

pub fn evaluate_entry(
    analysis: Option<&MrcCrossAnalysis>,
    detected_pattern: Option<PatternKind>,
    detected_trend_direction: Option<i8>,
    mrc: MrcBands,
    candle: Candle,
    market_price: f64,
    tick_size: f64,
    movement_window: Option<movement_window::MovementWindow>,
    last_sideways_movement_start_ms: Option<u64>,
    cascade_movement_start_ms: u64,
    cascade_direction: i8,
    sideways_order_pending: bool,
) -> EntryDecision {
    if detected_pattern == Some(PatternKind::MeanRange) {
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
        );
    }

    trending::entry::as_entry_decision(trending::entry::evaluate(
        analysis,
        detected_trend_direction,
        mrc,
        candle,
        market_price,
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
