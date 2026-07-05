use crate::model::{MarketState, PositionState, TradeDecision};
use crate::orders::{close, market_open};
use crate::state::{sync_position_state, EngineState};
use lte_strategy_bridge::abi::{Direction, ModuleClosePosition, ModuleInput, ModuleOpenPosition};

#[derive(Debug, Default)]
pub struct ExecutionResult {
    pub open_positions: Vec<ModuleOpenPosition>,
    pub close_positions: Vec<ModuleClosePosition>,
    pub note: String,
}

fn size_for_entry(input: &ModuleInput, state: &EngineState) -> f64 {
    let cap_notional = input.auto_max_amount.max(input.max_amount) * state.cfg.cap_pct;
    let leverage = (input.leverage as f64).max(1.0);
    let risk_notional =
        ((cap_notional / leverage) * state.cfg.risk_pct / state.cfg.stop_pct).min(cap_notional);
    let min_notional = input.symbol_info.min_notional.unwrap_or(1.0).max(1.0);
    (risk_notional * state.cfg.seed_pct * state.risk_drawdown).max(min_notional)
}

fn arm_stop_penalty(state: &mut EngineState, side: PositionState, price: f64) {
    state.cooldown_until = state.ticks.saturating_add(state.cfg.cooldown_ticks);
    state.risk_drawdown = (state.risk_drawdown * 0.5).max(0.25);
    state.last_stop_price = price;
    state.last_stop_side = side;
}

fn clear_stop_penalty(state: &mut EngineState) {
    state.risk_drawdown = 1.0;
}

fn same_price_reentry_block(state: &EngineState, side: PositionState, price: f64) -> bool {
    if side != state.last_stop_side || state.last_stop_price <= 0.0 {
        return false;
    }
    ((price - state.last_stop_price).abs() / state.last_stop_price)
        < state.cfg.tp_pullback_pct.max(0.001)
}

pub fn evaluate(
    input: &ModuleInput,
    state: &mut EngineState,
    decision: &TradeDecision,
) -> ExecutionResult {
    let long = input
        .positions
        .long
        .as_ref()
        .map(|p| (p.entry_price, p.pnl));
    let short = input
        .positions
        .short
        .as_ref()
        .map(|p| (p.entry_price, p.pnl));
    sync_position_state(state, long, short, input.price);

    if input.positions.long.is_some() && input.positions.short.is_some() {
        return ExecutionResult {
            close_positions: vec![
                close(Direction::Long, "one-position-invariant"),
                close(Direction::Short, "one-position-invariant"),
            ],
            note: "flatten-both".to_string(),
            ..Default::default()
        };
    }

    if let Some(pos) = &input.positions.long {
        let stop_price = pos.entry_price * (1.0 - state.cfg.stop_pct);
        if input.price <= stop_price {
            arm_stop_penalty(state, PositionState::Long, input.price);
            return ExecutionResult {
                close_positions: vec![close(Direction::Long, "sl-long")],
                note: "sl-long".to_string(),
                ..Default::default()
            };
        }
        let profitable = pos.pnl > 0.0;
        let pullback_hit =
            input.price <= state.position.peak_price * (1.0 - state.cfg.tp_pullback_pct);
        if profitable && pullback_hit {
            clear_stop_penalty(state);
            return ExecutionResult {
                close_positions: vec![close(Direction::Long, "tp-pullback-long")],
                note: "tp-pullback-long".to_string(),
                ..Default::default()
            };
        }
        if decision.market_state != MarketState::Long && pos.pnl <= 0.0 {
            return ExecutionResult {
                close_positions: vec![close(Direction::Long, "regime-invalid-long")],
                note: "regime-invalid-long".to_string(),
                ..Default::default()
            };
        }
        return ExecutionResult {
            note: "hold-long".to_string(),
            ..Default::default()
        };
    }

    if let Some(pos) = &input.positions.short {
        let stop_price = pos.entry_price * (1.0 + state.cfg.stop_pct);
        if input.price >= stop_price {
            arm_stop_penalty(state, PositionState::Short, input.price);
            return ExecutionResult {
                close_positions: vec![close(Direction::Short, "sl-short")],
                note: "sl-short".to_string(),
                ..Default::default()
            };
        }
        let profitable = pos.pnl > 0.0;
        let pullback_hit =
            input.price >= state.position.trough_price * (1.0 + state.cfg.tp_pullback_pct);
        if profitable && pullback_hit {
            clear_stop_penalty(state);
            return ExecutionResult {
                close_positions: vec![close(Direction::Short, "tp-pullback-short")],
                note: "tp-pullback-short".to_string(),
                ..Default::default()
            };
        }
        if decision.market_state != MarketState::Short && pos.pnl <= 0.0 {
            return ExecutionResult {
                close_positions: vec![close(Direction::Short, "regime-invalid-short")],
                note: "regime-invalid-short".to_string(),
                ..Default::default()
            };
        }
        return ExecutionResult {
            note: "hold-short".to_string(),
            ..Default::default()
        };
    }

    if state.ticks < state.cooldown_until || !decision.allow_entry {
        return ExecutionResult {
            note: if state.ticks < state.cooldown_until {
                "cooldown".to_string()
            } else {
                "entry-blocked".to_string()
            },
            ..Default::default()
        };
    }

    let amount = size_for_entry(input, state);
    match decision.market_state {
        MarketState::Long if !same_price_reentry_block(state, PositionState::Long, input.price) => {
            ExecutionResult {
                open_positions: vec![market_open(Direction::Long, amount, "regime-long")],
                note: "open-long".to_string(),
                ..Default::default()
            }
        }
        MarketState::Short
            if !same_price_reentry_block(state, PositionState::Short, input.price) =>
        {
            ExecutionResult {
                open_positions: vec![market_open(Direction::Short, amount, "regime-short")],
                note: "open-short".to_string(),
                ..Default::default()
            }
        }
        _ => ExecutionResult {
            note: "flat".to_string(),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Config, TradeDecision};
    use crate::state::EngineState;
    use lte_strategy_bridge::abi::{ModulePositionSummary, ModulePositions, ModuleSymbolInfo};

    fn input() -> ModuleInput {
        ModuleInput {
            event: lte_strategy_bridge::abi::ModuleEvent::SugInfo,
            price: 100.0,
            symbol: "TEST".to_string(),
            max_amount: 100.0,
            symbol_info: ModuleSymbolInfo::default(),
            auto_max_amount: 100.0,
            leverage: 10,
            indicators: Default::default(),
            positions: ModulePositions {
                long: None,
                short: None,
            },
            sug_info: None,
            state: None,
        }
    }

    #[test]
    fn sideways_blocks_entries() {
        let mut state = EngineState::default();
        let result = evaluate(
            &input(),
            &mut state,
            &TradeDecision {
                market_state: MarketState::Sideways,
                allow_entry: true,
                reason: String::new(),
            },
        );
        assert!(result.open_positions.is_empty());
    }

    #[test]
    fn no_opposite_open_while_position_is_active() {
        let mut state = EngineState::default();
        let mut inp = input();
        inp.positions.long = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: 1.0,
            qty: 0.1,
        });
        let result = evaluate(
            &inp,
            &mut state,
            &TradeDecision {
                market_state: MarketState::Short,
                allow_entry: true,
                reason: String::new(),
            },
        );
        assert!(result.open_positions.is_empty());
    }

    #[test]
    fn tp_pullback_closes_without_reversal() {
        let mut state = EngineState::default();
        state.position.side = PositionState::Long;
        state.position.entry_price = 100.0;
        state.position.peak_price = 105.0;

        let mut inp = input();
        inp.price = 104.0;
        inp.positions.long = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: 1.0,
            qty: 0.1,
        });

        let result = evaluate(
            &inp,
            &mut state,
            &TradeDecision {
                market_state: MarketState::Short,
                allow_entry: true,
                reason: String::new(),
            },
        );
        assert_eq!(result.close_positions.len(), 1);
        assert!(result.open_positions.is_empty());
    }

    #[test]
    fn stop_loss_applies_cooldown_and_drawdown() {
        let mut state = EngineState::default();
        state.cfg = Config::default();
        state.ticks = 10;

        let mut inp = input();
        inp.price = 99.0;
        inp.positions.long = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: -1.0,
            qty: 0.1,
        });

        let result = evaluate(
            &inp,
            &mut state,
            &TradeDecision {
                market_state: MarketState::Long,
                allow_entry: true,
                reason: String::new(),
            },
        );
        assert_eq!(result.close_positions.len(), 1);
        assert!(state.cooldown_until > state.ticks);
        assert!(state.risk_drawdown < 1.0);
    }
}
