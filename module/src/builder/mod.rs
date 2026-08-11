mod policy;
mod shared;
mod single;

pub use policy::StrategyPolicy;

use crate::detection::cascade::CascadeSignal;
use crate::detection::sideways::SidewaySignal;
use crate::model::{IndicatorSnapshot, RegimeSnapshot, SugInfoSnapshot, TradeDecision};
use crate::state::EngineState;
use lte_strategy_bridge::sug_info::SuggestionInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyPhase {
    Initial,
    Dynamic,
}

impl StrategyPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Dynamic => "dynamic",
        }
    }
}

#[derive(Debug, Clone)]
pub struct InitialBuilder {}

impl InitialBuilder {
    pub fn new() -> Self {
        Self {}
    }

    pub fn build(self) -> BuiltStrategy {
        BuiltStrategy::new(StrategyPhase::Initial, StrategyPolicy::single())
    }
}

#[derive(Debug, Clone)]
pub struct BuiltStrategy {
    phase: StrategyPhase,
    policy: StrategyPolicy,
    dynamic: DynamicBuilder,
}

impl BuiltStrategy {
    fn new(phase: StrategyPhase, policy: StrategyPolicy) -> Self {
        Self {
            phase,
            dynamic: DynamicBuilder::from_policy(policy),
            policy,
        }
    }

    pub fn promote_runtime(&mut self) {
        self.phase = StrategyPhase::Dynamic;
    }

    pub fn entry_wait_gate(&self) -> bool {
        self.dynamic.entry_wait_gate()
    }

    pub fn breakeven_reclaim(&self) -> bool {
        self.dynamic.breakeven_reclaim()
    }

    pub fn refine_realtime_decision(
        &self,
        state: &mut EngineState,
        snapshot: &IndicatorSnapshot,
        sug_snapshot: &SugInfoSnapshot,
        sideway: &SidewaySignal,
        regime: RegimeSnapshot,
        cascade: &CascadeSignal,
        sug_info: &SuggestionInfo,
        base_decision: TradeDecision,
    ) -> TradeDecision {
        self.dynamic.refine_realtime_decision(
            state,
            snapshot,
            sug_snapshot,
            sideway,
            regime,
            cascade,
            sug_info,
            base_decision,
        )
    }

    pub fn describe(&self) -> String {
        format!(
            "phase={} mode=single entry_wait={} breakeven={}",
            self.phase.as_str(),
            self.entry_wait_gate(),
            self.breakeven_reclaim(),
        )
    }
}

#[derive(Debug, Clone)]
pub enum DynamicBuilder {
    Single(single::SingleDynamicBuilder),
}

impl DynamicBuilder {
    fn from_policy(policy: StrategyPolicy) -> Self {
        Self::Single(single::SingleDynamicBuilder::new(policy))
    }

    pub fn entry_wait_gate(&self) -> bool {
        match self {
            Self::Single(builder) => builder.policy.entry_wait_gate,
        }
    }

    pub fn breakeven_reclaim(&self) -> bool {
        match self {
            Self::Single(builder) => builder.policy.breakeven_reclaim,
        }
    }

    fn refine_realtime_decision(
        &self,
        state: &mut EngineState,
        snapshot: &IndicatorSnapshot,
        sug_snapshot: &SugInfoSnapshot,
        sideway: &SidewaySignal,
        regime: RegimeSnapshot,
        cascade: &CascadeSignal,
        sug_info: &SuggestionInfo,
        base_decision: TradeDecision,
    ) -> TradeDecision {
        match self {
            Self::Single(builder) => builder.refine_realtime_decision(
                state,
                snapshot,
                sug_snapshot,
                sideway,
                regime,
                cascade,
                sug_info,
                base_decision,
            ),
        }
    }
}

pub fn ensure_runtime_builder(state: &mut EngineState) -> BuiltStrategy {
    if state.strategy_builder.is_none() {
        state.strategy_builder = Some(InitialBuilder::new().build());
    }

    let strategy = state
        .strategy_builder
        .as_mut()
        .expect("strategy builder is always initialized");
    strategy.promote_runtime();
    strategy.clone()
}
