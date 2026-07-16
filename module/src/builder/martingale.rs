use super::policy::StrategyPolicy;
use super::shared::{refine_common, should_close_on_direction_change_impl};
use crate::detection::{cascade::CascadeSignal, sideways::SidewaySignal};
use crate::model::{IndicatorSnapshot, RegimeSnapshot, SugInfoSnapshot, TradeDecision};
use crate::state::EngineState;
use lte_strategy_bridge::sug_info::SuggestionInfo;

#[derive(Debug, Clone)]
pub struct MartingaleDynamicBuilder {
    pub(super) policy: StrategyPolicy,
}

impl MartingaleDynamicBuilder {
    pub(super) fn new(policy: StrategyPolicy) -> Self {
        Self { policy }
    }

    pub(super) fn should_close_on_direction_change(
        &self,
        input: &lte_strategy_bridge::abi::ModuleInput,
        state: &EngineState,
    ) -> bool {
        should_close_on_direction_change_impl(self.policy, input, state)
    }

    pub(super) fn refine_realtime_decision(
        &self,
        state: &mut EngineState,
        snapshot: &IndicatorSnapshot,
        sug_snapshot: &SugInfoSnapshot,
        sideway: &SidewaySignal,
        regime: RegimeSnapshot,
        cascade_tf: &CascadeSignal,
        cascade_sug: &CascadeSignal,
        sug_info: &SuggestionInfo,
        decision: TradeDecision,
    ) -> TradeDecision {
        refine_common(
            state,
            snapshot,
            sug_snapshot,
            sideway,
            regime,
            cascade_tf,
            cascade_sug,
            sug_info,
            decision,
            false,
        )
    }
}
