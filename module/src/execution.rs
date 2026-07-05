use crate::model::{MarketState, PositionState, TradeDecision};
use crate::orders::{close, limit_open};
use crate::state::{sync_position_state, EngineState};
use crate::model::IndicatorSnapshot;
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

fn same_price_reentry_block(state: &EngineState, side: PositionState, price: f64) -> bool {
    if side != state.last_stop_side || state.last_stop_price <= 0.0 {
        return false;
    }
    ((price - state.last_stop_price).abs() / state.last_stop_price)
        < state.cfg.tp_pullback_pct.max(0.001)
}

fn open_note(side: Direction, amount: f64, price: f64, decision: &TradeDecision, state: &EngineState) -> String {
    let side = match side {
        Direction::Long => "long",
        Direction::Short => "short",
        _ => "both",
    };
    format!(
        "open {} because {} price={:.4} amount={:.4} risk_drawdown={:.2}",
        side,
        decision.reason,
        price,
        amount,
        state.risk_drawdown
    )
}

fn close_reason(reason: &str, state: &EngineState, price: f64) -> String {
    format!(
        "close because {} price={:.4} cooldown_until={} risk_drawdown={:.2}",
        reason,
        price,
        state.cooldown_until,
        state.risk_drawdown
    )
}

pub fn evaluate(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
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
                close(
                    Direction::Long,
                    &close_reason("one-position-invariant", state, input.price),
                ),
                close(
                    Direction::Short,
                    &close_reason("one-position-invariant", state, input.price),
                ),
            ],
            note: "flatten-both".to_string(),
            ..Default::default()
        };
    }

    if input.positions.long.is_some() {
        let _ = snapshot;
        return ExecutionResult {
            note: "hold-long".to_string(),
            ..Default::default()
        };
    }

    if input.positions.short.is_some() {
        let _ = snapshot;
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
                open_positions: vec![limit_open(
                    Direction::Long,
                    amount,
                    input.price,
                    &open_note(
                        Direction::Long,
                        amount,
                        input.price,
                        decision,
                        state,
                    ),
                )],
                note: "open-long".to_string(),
                ..Default::default()
            }
        }
        MarketState::Short
            if !same_price_reentry_block(state, PositionState::Short, input.price) =>
        {
            ExecutionResult {
                open_positions: vec![limit_open(
                    Direction::Short,
                    amount,
                    input.price,
                    &open_note(
                        Direction::Short,
                        amount,
                        input.price,
                        decision,
                        state,
                    ),
                )],
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
    use crate::model::TradeDecision;
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
            &IndicatorSnapshot::default(),
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
            &IndicatorSnapshot::default(),
            &TradeDecision {
                market_state: MarketState::Short,
                allow_entry: true,
                reason: String::new(),
            },
        );
        assert!(result.open_positions.is_empty());
    }

    #[test]
    fn long_open_uses_limit_entry() {
        let mut state = EngineState::default();
        let inp = input();

        let result = evaluate(
            &inp,
            &mut state,
            &IndicatorSnapshot::default(),
            &TradeDecision {
                market_state: MarketState::Long,
                allow_entry: true,
                reason: String::new(),
            },
        );
        assert_eq!(result.open_positions.len(), 1);
        let open = &result.open_positions[0];
        assert_eq!(open.order_type, "Limit");
        assert_eq!(open.enter_price, Some(inp.price));
        assert!(open.take_profit.is_none());
        assert!(open.stop_loss.is_none());
    }

    #[test]
    fn short_open_uses_limit_entry() {
        let mut state = EngineState::default();
        let inp = input();

        let result = evaluate(
            &inp,
            &mut state,
            &IndicatorSnapshot::default(),
            &TradeDecision {
                market_state: MarketState::Short,
                allow_entry: true,
                reason: String::new(),
            },
        );
        assert_eq!(result.open_positions.len(), 1);
        let open = &result.open_positions[0];
        assert_eq!(open.order_type, "Limit");
        assert_eq!(open.enter_price, Some(inp.price));
        assert!(open.take_profit.is_none());
        assert!(open.stop_loss.is_none());
    }
}
