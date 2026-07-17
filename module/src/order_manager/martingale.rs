use super::shared;
use crate::martingale;
use crate::state::EngineState;
use lte_strategy_bridge::abi::{
    Direction, ModuleInput, ModuleOrderStatus, ModuleOutput, OrderRole,
};

#[derive(Debug, Clone, Default)]
pub struct MartingaleOrderManager;

impl MartingaleOrderManager {
    pub fn new() -> Self {
        Self
    }

    pub fn handle_order_update(
        &self,
        input: &ModuleInput,
        state: &mut EngineState,
        direction: Direction,
        status: ModuleOrderStatus,
        role: OrderRole,
        fill_price: f64,
        filled_qty: f64,
        mark: &Option<String>,
    ) -> ModuleOutput {
        shared::record_exit_memory(input, state, direction, &role, fill_price);

        let is_filled = filled_qty > 0.0 && matches!(status, ModuleOrderStatus::Filled);
        if !matches!(role, OrderRole::Entry) || !is_filled {
            return ModuleOutput {
                debug: shared::debug_line(direction, status, role, mark),
                ..Default::default()
            };
        }

        let Some(filled_level) = martingale::parse_ladder_mark(mark.as_deref()) else {
            return ModuleOutput {
                debug: format!(
                    "order {:?} {:?} {:?} {:?} martingale-skip reason=missing-mark",
                    direction, status, role, mark
                ),
                ..Default::default()
            };
        };

        if filled_level == 0 {
            return ModuleOutput {
                debug: format!(
                    "order {:?} {:?} {:?} {:?} martingale-skip reason=seed-mark",
                    direction, status, role, mark
                ),
                ..Default::default()
            };
        }

        if state
            .position
            .martingale
            .as_ref()
            .and_then(|martingale| martingale.last_filled_mark.as_deref())
            == mark.as_deref()
        {
            return ModuleOutput {
                debug: format!(
                    "order {:?} {:?} {:?} {:?} martingale-skip reason=duplicate-mark filled={}",
                    direction, status, role, mark, filled_level
                ),
                ..Default::default()
            };
        }

        let Some(martingale) = state.position.martingale.as_ref() else {
            return ModuleOutput {
                debug: shared::debug_line(direction, status, role, mark),
                ..Default::default()
            };
        };

        if state.position.side == crate::model::PositionState::Flat
            || martingale.base_price <= 0.0
            || martingale.base_amount <= 0.0
        {
            return ModuleOutput {
                debug: shared::debug_line(direction, status, role, mark),
                ..Default::default()
            };
        }

        let side = match state.position.side {
            crate::model::PositionState::Long => crate::model::MarketState::Long,
            crate::model::PositionState::Short => crate::model::MarketState::Short,
            crate::model::PositionState::Flat => crate::model::MarketState::Sideways,
        };
        let start_level = filled_level.saturating_add(1);
        let target_level = filled_level
            .saturating_add(2)
            .min(martingale.profile.levels);
        if start_level > target_level {
            return ModuleOutput {
                debug: format!(
                    "order {:?} {:?} {:?} {:?} martingale-skip reason=window-exhausted filled={}",
                    direction, status, role, mark, filled_level
                ),
                ..Default::default()
            };
        }

        let mut orders = Vec::new();
        for level in start_level..=target_level {
            if let Some(order) = martingale::ladder_order(
                side,
                martingale.profile,
                martingale.base_price,
                martingale.base_amount,
                level,
            ) {
                orders.push(order);
            }
        }

        if orders.is_empty() {
            return ModuleOutput {
                debug: shared::debug_line(direction, status, role, mark),
                ..Default::default()
            };
        }

        if let Some(martingale) = state.position.martingale.as_mut() {
            martingale.last_filled_mark = mark.clone();
        }
        ModuleOutput {
            place_orders: orders,
            debug: format!(
                "order {:?} {:?} {:?} {:?} martingale-replenish filled={} window={}..{}",
                direction, status, role, mark, filled_level, start_level, target_level
            ),
            ..Default::default()
        }
    }
}
