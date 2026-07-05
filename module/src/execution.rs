use crate::model::IndicatorSnapshot;
use crate::model::{MarketState, TradeDecision};
use crate::orders::{close, limit_open};
use crate::risk::entry_budget;
use crate::state::{sync_position_state, EngineState};
use lte_strategy_bridge::abi::{Direction, ModuleClosePosition, ModuleInput, ModuleOpenPosition};

#[derive(Debug, Default)]
pub struct ExecutionResult {
    pub open_positions: Vec<ModuleOpenPosition>,
    pub close_positions: Vec<ModuleClosePosition>,
    pub note: String,
}

fn open_note(
    side: Direction,
    amount: f64,
    price: f64,
    decision: &TradeDecision,
    state: &EngineState,
) -> String {
    let side = match side {
        Direction::Long => "long",
        Direction::Short => "short",
        _ => "both",
    };
    format!(
        "open {} because {} price={:.4} amount={:.4} cap_init={:.2}% budget={:.4}",
        side,
        decision.reason,
        price,
        amount,
        state.cfg.cap_init,
        state.trade_budget
    )
}

fn close_reason(reason: &str, state: &EngineState, price: f64) -> String {
    format!(
        "close because {} price={:.4} cap_init={:.2}% budget={:.4}",
        reason,
        price,
        state.cfg.cap_init,
        state.trade_budget
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

    if !decision.allow_entry {
        return ExecutionResult {
            note: "entry-blocked".to_string(),
            ..Default::default()
        };
    }

    let amount = entry_budget(input, state);
    let min_notional = input.symbol_info.min_notional.unwrap_or(1.0).max(1.0);
    if amount < min_notional {
        return ExecutionResult {
            note: "budget-too-small".to_string(),
            ..Default::default()
        };
    }

    match decision.market_state {
        MarketState::Long => {
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
        MarketState::Short => {
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
    use crate::risk::apply_close_pnl;
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
    fn cap_init_zero_blocks_entries() {
        let mut state = EngineState::default();
        state.cfg.cap_init = 0.0;
        let result = evaluate(
            &input(),
            &mut state,
            &IndicatorSnapshot::default(),
            &TradeDecision {
                market_state: MarketState::Long,
                allow_entry: true,
                reason: String::new(),
            },
        );
        assert!(result.open_positions.is_empty());
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
    fn entry_budget_uses_cap_init_and_min_notional() {
        let mut state = EngineState::default();
        state.cfg.cap_init = 2.0;
        let result = evaluate(
            &input(),
            &mut state,
            &IndicatorSnapshot::default(),
            &TradeDecision {
                market_state: MarketState::Long,
                allow_entry: true,
                reason: String::new(),
            },
        );
        assert_eq!(result.open_positions.len(), 1);
        assert_eq!(result.open_positions[0].amount, Some(1.0));
    }

    #[test]
    fn profit_doubles_before_leverage_growth() {
        let mut state = EngineState::default();
        let inp = input();
        state.trade_budget = 1.0;

        apply_close_pnl(&inp, &mut state, 0.25);
        assert_eq!(state.trade_budget, 2.0);

        apply_close_pnl(&inp, &mut state, 0.25);
        assert_eq!(state.trade_budget, 4.5);
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
