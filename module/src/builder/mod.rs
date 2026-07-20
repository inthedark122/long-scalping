mod martingale;
mod policy;
mod shared;
mod single;

pub use policy::StrategyPolicy;

use crate::detection::cascade::CascadeSignal;
use crate::detection::sideways::SidewaySignal;
use crate::model::{IndicatorSnapshot, RegimeSnapshot, SugInfoSnapshot, TradeDecision, TradeMode};
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
pub struct InitialBuilder {
    trade_mode: TradeMode,
}

impl InitialBuilder {
    pub fn new(trade_mode: TradeMode) -> Self {
        Self { trade_mode }
    }

    pub fn build(self) -> BuiltStrategy {
        BuiltStrategy::new(
            StrategyPhase::Initial,
            StrategyPolicy::for_mode(self.trade_mode),
        )
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

    pub fn trade_mode(&self) -> TradeMode {
        self.policy.trade_mode
    }

    pub fn is_martingale(&self) -> bool {
        matches!(self.policy.trade_mode, TradeMode::Martingale)
    }

    pub fn entry_wait_gate(&self) -> bool {
        self.dynamic.entry_wait_gate()
    }

    pub fn breakeven_reclaim(&self) -> bool {
        self.dynamic.breakeven_reclaim()
    }

    pub fn virtual_sl_tp_advance(&self) -> bool {
        self.dynamic.virtual_sl_tp_advance()
    }

    pub fn should_close_on_direction_change(
        &self,
        input: &lte_strategy_bridge::abi::ModuleInput,
        state: &EngineState,
    ) -> bool {
        self.dynamic.should_close_on_direction_change(input, state)
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
            "phase={} mode={} entry_wait={} dir_close={} breakeven={} virtual_sl={}",
            self.phase.as_str(),
            self.trade_mode().as_str(),
            self.entry_wait_gate(),
            self.dynamic.direction_change_close(),
            self.breakeven_reclaim(),
            self.virtual_sl_tp_advance(),
        )
    }
}

#[derive(Debug, Clone)]
pub enum DynamicBuilder {
    Single(single::SingleDynamicBuilder),
    Martingale(martingale::MartingaleDynamicBuilder),
}

impl DynamicBuilder {
    fn from_policy(policy: StrategyPolicy) -> Self {
        match policy.trade_mode {
            TradeMode::Single => Self::Single(single::SingleDynamicBuilder::new(policy)),
            TradeMode::Martingale => {
                Self::Martingale(martingale::MartingaleDynamicBuilder::new(policy))
            }
        }
    }

    pub fn entry_wait_gate(&self) -> bool {
        match self {
            Self::Single(builder) => builder.policy.entry_wait_gate,
            Self::Martingale(builder) => builder.policy.entry_wait_gate,
        }
    }

    pub fn breakeven_reclaim(&self) -> bool {
        match self {
            Self::Single(builder) => builder.policy.breakeven_reclaim,
            Self::Martingale(builder) => builder.policy.breakeven_reclaim,
        }
    }

    pub fn virtual_sl_tp_advance(&self) -> bool {
        match self {
            Self::Single(builder) => builder.policy.virtual_sl_tp_advance,
            Self::Martingale(builder) => builder.policy.virtual_sl_tp_advance,
        }
    }

    pub fn should_close_on_direction_change(
        &self,
        input: &lte_strategy_bridge::abi::ModuleInput,
        state: &EngineState,
    ) -> bool {
        match self {
            Self::Single(builder) => builder.should_close_on_direction_change(input, state),
            Self::Martingale(builder) => builder.should_close_on_direction_change(input, state),
        }
    }

    pub fn direction_change_close(&self) -> bool {
        match self {
            Self::Single(builder) => builder.policy.direction_change_close,
            Self::Martingale(builder) => builder.policy.direction_change_close,
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
            Self::Martingale(builder) => builder.refine_realtime_decision(
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
        state.strategy_builder = Some(InitialBuilder::new(state.cfg.trade_mode).build());
    }

    let strategy = state
        .strategy_builder
        .as_mut()
        .expect("strategy builder is always initialized");
    strategy.promote_runtime();
    strategy.clone()
}
