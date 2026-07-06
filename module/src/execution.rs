use crate::model::IndicatorSnapshot;
use crate::model::{EntryGate, ExitKind, MarketState, PositionState, TradeDecision};
use crate::orders::{close, limit_open};
use crate::exit;
use crate::prediction;
use crate::risk::entry_budget;
use crate::state::{sync_position_state, EngineState};
use lte_strategy_bridge::abi::{
    Direction, ModuleClosePosition, ModuleInput, ModuleOpenPosition, ModuleUpdatePosition,
};

#[derive(Debug, Default)]
pub struct ExecutionResult {
    pub open_positions: Vec<ModuleOpenPosition>,
    pub close_positions: Vec<ModuleClosePosition>,
    pub update_positions: Vec<ModuleUpdatePosition>,
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
    let entry_note = match &decision.entry {
        EntryGate::Open { notes } => notes.join("|"),
        EntryGate::Wait(wait) => format!("wait:{}|{}", wait.reason, wait.notes.join("|")),
    };
    format!(
        "open {} because {} price={:.4} amount={:.4} cap_init={:.2}% trading_amount={:.2}% budget={:.4}",
        side,
        entry_note,
        price,
        amount,
        state.cfg.cap_init,
        state.cfg.trading_amount,
        state.trade_budget
    )
}

fn close_reason(reason: &str, state: &EngineState, price: f64) -> String {
    format!(
        "close because {} price={:.4} cap_init={:.2}% trading_amount={:.2}% budget={:.4}",
        reason,
        price,
        state.cfg.cap_init,
        state.cfg.trading_amount,
        state.trade_budget
    )
}

fn exit_hit_reason(
    side: PositionState,
    price: f64,
    take_profit: f64,
    stop_loss: f64,
) -> Option<&'static str> {
    match side {
        PositionState::Long => {
            if take_profit > 0.0 && price >= take_profit {
                Some("tp")
            } else if stop_loss > 0.0 && price <= stop_loss {
                Some("sl")
            } else {
                None
            }
        }
        PositionState::Short => {
            if take_profit > 0.0 && price <= take_profit {
                Some("tp")
            } else if stop_loss > 0.0 && price >= stop_loss {
                Some("sl")
            } else {
                None
            }
        }
        PositionState::Flat => None,
    }
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
        if let Some(hit) = exit_hit_reason(
            PositionState::Long,
            input.price,
            state.position.take_profit,
            state.position.stop_loss,
        ) {
            let exit_kind = if hit == "tp" {
                ExitKind::TakeProfit
            } else {
                ExitKind::StopLoss
            };
            state.last_exit_kind = Some(exit_kind);
            state.last_exit_side = PositionState::Long;
            state.last_exit_price = input.price;
            state.last_exit_tick = state.ticks;
            state.last_exit_tp_reliable = state.position.exit_tp_reliable;
            return ExecutionResult {
                close_positions: vec![close(
                    Direction::Long,
                    &format!(
                        "tp/sl hit={} price={:.4} tp={:.4} sl={:.4}",
                        hit, input.price, state.position.take_profit, state.position.stop_loss
                    ),
                )],
                note: format!("close-long {}", hit),
                ..Default::default()
            };
        }

        let plan = exit::plan(snapshot, input.price, crate::model::PositionState::Long);
        if let Some(plan) = plan {
            state.position.take_profit = plan.take_profit;
            state.position.stop_loss = plan.stop_loss;
            state.position.exit_tp_reliable = plan.tp_reliable;
        }
        return ExecutionResult {
            update_positions: plan
                .map(|plan| vec![exit::update_position(crate::model::PositionState::Long, plan)])
                .unwrap_or_default(),
            note: format!("hold-long {}", exit::describe(plan)),
            ..Default::default()
        };
    }

    if input.positions.short.is_some() {
        if let Some(hit) = exit_hit_reason(
            PositionState::Short,
            input.price,
            state.position.take_profit,
            state.position.stop_loss,
        ) {
            let exit_kind = if hit == "tp" {
                ExitKind::TakeProfit
            } else {
                ExitKind::StopLoss
            };
            state.last_exit_kind = Some(exit_kind);
            state.last_exit_side = PositionState::Short;
            state.last_exit_price = input.price;
            state.last_exit_tick = state.ticks;
            state.last_exit_tp_reliable = state.position.exit_tp_reliable;
            return ExecutionResult {
                close_positions: vec![close(
                    Direction::Short,
                    &format!(
                        "tp/sl hit={} price={:.4} tp={:.4} sl={:.4}",
                        hit, input.price, state.position.take_profit, state.position.stop_loss
                    ),
                )],
                note: format!("close-short {}", hit),
                ..Default::default()
            };
        }

        let plan = exit::plan(snapshot, input.price, crate::model::PositionState::Short);
        if let Some(plan) = plan {
            state.position.take_profit = plan.take_profit;
            state.position.stop_loss = plan.stop_loss;
            state.position.exit_tp_reliable = plan.tp_reliable;
        }
        return ExecutionResult {
            update_positions: plan
                .map(|plan| vec![exit::update_position(crate::model::PositionState::Short, plan)])
                .unwrap_or_default(),
            note: format!("hold-short {}", exit::describe(plan)),
            ..Default::default()
        };
    }

    let entry_side = match decision.market_state {
        MarketState::Long => PositionState::Long,
        MarketState::Short => PositionState::Short,
        MarketState::Sideways => PositionState::Flat,
    };

    match &decision.entry {
        EntryGate::Wait(wait) => {
            return ExecutionResult {
                note: format!(
                    "entry-wait reason={} solutions={} notes={}",
                    wait.reason,
                    wait.solutions.len(),
                    wait.notes.join("|")
                ),
                ..Default::default()
            };
        }
        EntryGate::Open { .. } => {}
    }

    if let EntryGate::Wait(wait) = prediction::entry_gate(snapshot, state, entry_side) {
        return ExecutionResult {
            note: format!(
                "entry-wait reason={} solutions={} notes={}",
                wait.reason,
                wait.solutions.len(),
                wait.notes.join("|")
            ),
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

    fn decision(market_state: MarketState) -> TradeDecision {
        TradeDecision {
            market_state,
            entry: crate::model::EntryGate::Open { notes: vec![] },
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
            &decision(MarketState::Long),
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
            &decision(MarketState::Sideways),
        );
        assert!(result.open_positions.is_empty());
    }

    #[test]
    fn building_fast_candle_blocks_entries() {
        let mut state = EngineState::default();
        let mut snapshot = IndicatorSnapshot::default();
        snapshot.fast_candle = Some(crate::model::FastCandleSnapshot {
            candle: crate::model::Candle {
                open: 100.0,
                high: 106.0,
                low: 100.0,
                close: 105.0,
            },
            active: true,
        });
        snapshot.tf1m.candle = crate::model::Candle {
            open: 100.0,
            high: 105.5,
            low: 99.5,
            close: 105.0,
        };
        snapshot.tf1m.volume_gain = 8.0;
        snapshot.tf1m.volume_gain_min = 1.0;
        snapshot.tf1m.volume_gain_max = 20.0;

        let result = evaluate(
            &input(),
            &mut state,
            &snapshot,
            &decision(MarketState::Long),
        );
        assert!(result.open_positions.is_empty());
        assert!(result.note.contains("fc-phase=Building"));
    }

    #[test]
    fn tiring_fast_candle_blocks_entries() {
        let mut state = EngineState::default();
        let mut snapshot = IndicatorSnapshot::default();
        snapshot.fast_candle = Some(crate::model::FastCandleSnapshot {
            candle: crate::model::Candle {
                open: 100.0,
                high: 110.0,
                low: 100.0,
                close: 110.0,
            },
            active: true,
        });
        snapshot.tf1m.candle = crate::model::Candle {
            open: 110.0,
            high: 110.1,
            low: 109.8,
            close: 110.0,
        };
        snapshot.tf1m.volume_gain = 18.0;
        snapshot.tf1m.volume_gain_min = 1.0;
        snapshot.tf1m.volume_gain_max = 20.0;

        let result = evaluate(
            &input(),
            &mut state,
            &snapshot,
            &decision(MarketState::Long),
        );
        assert!(result.open_positions.is_empty());
        assert!(result.note.contains("fc-phase=Tiring"));
    }

    #[test]
    fn entry_budget_uses_cap_init_and_min_notional() {
        let mut state = EngineState::default();
        state.cfg.cap_init = 2.0;
        state.cfg.trading_amount = 50.0;
        let result = evaluate(
            &input(),
            &mut state,
            &IndicatorSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(result.open_positions.len(), 1);
        assert_eq!(result.open_positions[0].amount, Some(10.0));
    }

    #[test]
    fn cooling_fast_candle_allows_entries() {
        let mut state = EngineState::default();
        let mut snapshot = IndicatorSnapshot::default();
        snapshot.fast_candle = Some(crate::model::FastCandleSnapshot {
            candle: crate::model::Candle {
                open: 100.0,
                high: 106.0,
                low: 100.0,
                close: 105.0,
            },
            active: false,
        });
        snapshot.tf1m.candle = crate::model::Candle {
            open: 105.0,
            high: 105.2,
            low: 104.9,
            close: 105.0,
        };
        snapshot.tf1m.volume_gain = 2.0;
        snapshot.tf1m.volume_gain_min = 1.0;
        snapshot.tf1m.volume_gain_max = 20.0;

        let result = evaluate(
            &input(),
            &mut state,
            &snapshot,
            &decision(MarketState::Long),
        );
        assert_eq!(result.open_positions.len(), 1);
    }

    #[test]
    fn unreliable_tp_blocks_entries() {
        let mut state = EngineState::default();
        let mut snapshot = IndicatorSnapshot::default();
        snapshot.tf1m.candle = crate::model::Candle {
            open: 100.0,
            high: 100.8,
            low: 99.9,
            close: 100.0,
        };
        snapshot.tf1m.mrc = crate::model::MrcBands {
            down_big: 80.0,
            down_outer: 85.0,
            down_small: 90.0,
            down_inner: 95.0,
            mean_line: 100.0,
            up_inner: 100.7,
            up_small: 101.0,
            up_outer: 101.5,
            up_big: 102.0,
            current_cross: 0.0,
            mean_line_slope: 0.0,
        };
        snapshot.tf5m = snapshot.tf1m;
        snapshot.tf15m = snapshot.tf1m;
        snapshot.tf1h = snapshot.tf1m;

        let result = evaluate(
            &input(),
            &mut state,
            &snapshot,
            &decision(MarketState::Long),
        );
        assert!(result.open_positions.is_empty());
        assert!(result.note.contains("tp-unreliable"));
    }

    #[test]
    fn post_sl_wait_requires_reclaim() {
        let mut state = EngineState::default();
        state.last_exit_kind = Some(ExitKind::StopLoss);
        state.last_exit_side = PositionState::Long;
        state.last_exit_price = 100.0;
        state.last_exit_tick = 10;
        let mut snapshot = IndicatorSnapshot::default();
        snapshot.tf1m.candle = crate::model::Candle {
            open: 99.0,
            high: 99.5,
            low: 98.8,
            close: 99.2,
        };
        snapshot.tf1m.mrc = crate::model::MrcBands {
            down_big: 80.0,
            down_outer: 85.0,
            down_small: 90.0,
            down_inner: 95.0,
            mean_line: 100.0,
            up_inner: 105.0,
            up_small: 110.0,
            up_outer: 115.0,
            up_big: 120.0,
            current_cross: 0.0,
            mean_line_slope: 0.0,
        };
        snapshot.tf5m = snapshot.tf1m;
        snapshot.tf15m = snapshot.tf1m;
        snapshot.tf1h = snapshot.tf1m;

        let result = evaluate(
            &input(),
            &mut state,
            &snapshot,
            &decision(MarketState::Long),
        );
        assert!(result.open_positions.is_empty());
        assert!(result.note.contains("post-sl-recovery"));
    }

    #[test]
    fn post_sl_wait_uses_dpsignal_band() {
        let mut state = EngineState::default();
        state.last_exit_kind = Some(ExitKind::StopLoss);
        state.last_exit_side = PositionState::Long;
        state.last_exit_price = 100.0;
        let mut snapshot = IndicatorSnapshot::default();
        snapshot.tf1m.candle = crate::model::Candle {
            open: 99.0,
            high: 99.4,
            low: 98.7,
            close: 99.1,
        };
        snapshot.tf1m.dp_zee = 101.0;
        snapshot.tf1m.dp_zee1 = 100.5;
        snapshot.tf1m.mrc = crate::model::MrcBands {
            down_big: 80.0,
            down_outer: 85.0,
            down_small: 90.0,
            down_inner: 95.0,
            mean_line: 100.0,
            up_inner: 105.0,
            up_small: 110.0,
            up_outer: 115.0,
            up_big: 120.0,
            current_cross: 0.0,
            mean_line_slope: 0.0,
        };
        snapshot.tf5m = snapshot.tf1m;
        snapshot.tf15m = snapshot.tf1m;
        snapshot.tf1h = snapshot.tf1m;

        let result = evaluate(
            &input(),
            &mut state,
            &snapshot,
            &decision(MarketState::Long),
        );
        assert!(result.note.contains("post-sl-recovery"));
        assert!(result.note.contains("solutions=3"));
    }

    #[test]
    fn cooling_fast_candle_with_large_candle_waits() {
        let mut state = EngineState::default();
        let mut snapshot = IndicatorSnapshot::default();
        snapshot.fast_candle = Some(crate::model::FastCandleSnapshot {
            candle: crate::model::Candle {
                open: 100.0,
                high: 106.0,
                low: 100.0,
                close: 105.0,
            },
            active: false,
        });
        snapshot.tf1m.candle = crate::model::Candle {
            open: 100.0,
            high: 104.5,
            low: 99.5,
            close: 104.0,
        };
        snapshot.tf1m.volume_gain = 2.0;
        snapshot.tf1m.volume_gain_min = 1.0;
        snapshot.tf1m.volume_gain_max = 20.0;

        let result = evaluate(
            &input(),
            &mut state,
            &snapshot,
            &decision(MarketState::Long),
        );
        assert!(result.open_positions.is_empty());
        assert!(result.note.contains("candle-not-small"));
    }

    #[test]
    fn long_tp_hit_closes_position() {
        let mut state = EngineState::default();
        state.position = crate::state::PositionRuntime {
            side: crate::model::PositionState::Long,
            entry_price: 100.0,
            peak_price: 103.0,
            trough_price: 99.0,
            take_profit: 102.5,
            stop_loss: 95.0,
            exit_tp_reliable: true,
        };
        let mut inp = input();
        inp.positions.long = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: 0.8,
            qty: 0.1,
        });
        inp.price = 103.0;
        let mut snapshot = IndicatorSnapshot::default();
        snapshot.tf1m.candle = crate::model::Candle {
            open: 102.0,
            high: 103.0,
            low: 101.5,
            close: 103.0,
        };
        snapshot.tf1m.mrc = crate::model::MrcBands {
            down_big: 80.0,
            down_outer: 85.0,
            down_small: 90.0,
            down_inner: 95.0,
            mean_line: 100.0,
            up_inner: 105.0,
            up_small: 110.0,
            up_outer: 115.0,
            up_big: 120.0,
            current_cross: 0.0,
            mean_line_slope: 0.0,
        };
        snapshot.tf5m = snapshot.tf1m;
        snapshot.tf15m = snapshot.tf1m;
        snapshot.tf1h = snapshot.tf1m;
        let result = evaluate(
            &inp,
            &mut state,
            &snapshot,
            &decision(MarketState::Long),
        );
        assert_eq!(result.close_positions.len(), 1);
        assert!(result.note.contains("close-long tp"));
    }

    #[test]
    fn short_sl_hit_closes_position() {
        let mut state = EngineState::default();
        state.position = crate::state::PositionRuntime {
            side: crate::model::PositionState::Short,
            entry_price: 100.0,
            peak_price: 101.0,
            trough_price: 99.0,
            take_profit: 97.5,
            stop_loss: 100.5,
            exit_tp_reliable: true,
        };
        let mut inp = input();
        inp.positions.short = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: -0.6,
            qty: 0.1,
        });
        inp.price = 101.0;
        let mut snapshot = IndicatorSnapshot::default();
        snapshot.tf1m.candle = crate::model::Candle {
            open: 100.5,
            high: 101.0,
            low: 100.4,
            close: 101.0,
        };
        snapshot.tf1m.mrc = crate::model::MrcBands {
            down_big: 80.0,
            down_outer: 85.0,
            down_small: 90.0,
            down_inner: 95.0,
            mean_line: 100.0,
            up_inner: 105.0,
            up_small: 110.0,
            up_outer: 115.0,
            up_big: 120.0,
            current_cross: 0.0,
            mean_line_slope: 0.0,
        };
        snapshot.tf5m = snapshot.tf1m;
        snapshot.tf15m = snapshot.tf1m;
        snapshot.tf1h = snapshot.tf1m;
        let result = evaluate(
            &inp,
            &mut state,
            &snapshot,
            &decision(MarketState::Short),
        );
        assert_eq!(result.close_positions.len(), 1);
        assert!(result.note.contains("close-short sl"));
    }

    #[test]
    fn trading_amount_defaults_to_half_of_deployable_budget() {
        let mut state = EngineState::default();
        state.cfg.cap_init = 2.0;
        state.cfg.trading_amount = 50.0;
        let amount = entry_budget(&input(), &mut state);
        assert_eq!(amount, 10.0);
    }

    #[test]
    fn trading_amount_clamps_to_minimum_and_full_size() {
        let mut state = EngineState::default();
        state.cfg.cap_init = 2.0;
        state.cfg.trading_amount = 10.0;
        let low = entry_budget(&input(), &mut state);
        assert_eq!(low, 2.0);

        let mut state = EngineState::default();
        state.cfg.cap_init = 2.0;
        state.cfg.trading_amount = 100.0;
        let full = entry_budget(&input(), &mut state);
        assert_eq!(full, 20.0);
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
            &decision(MarketState::Short),
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
            &decision(MarketState::Long),
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
            &decision(MarketState::Short),
        );
        assert_eq!(result.open_positions.len(), 1);
        let open = &result.open_positions[0];
        assert_eq!(open.order_type, "Limit");
        assert_eq!(open.enter_price, Some(inp.price));
        assert!(open.take_profit.is_none());
        assert!(open.stop_loss.is_none());
    }

    #[test]
    fn active_long_emits_exit_update() {
        let mut state = EngineState::default();
        let mut inp = input();
        inp.positions.long = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: 0.5,
            qty: 0.1,
        });
        let result = evaluate(
            &inp,
            &mut state,
            &IndicatorSnapshot {
                tf1m: crate::model::TimeframeSnapshot {
                    candle: crate::model::Candle {
                        open: 100.0,
                        high: 101.0,
                        low: 99.0,
                        close: 100.0,
                    },
                    mrc: crate::model::MrcBands {
                        down_big: 80.0,
                        down_outer: 85.0,
                        down_small: 90.0,
                        down_inner: 95.0,
                        mean_line: 100.0,
                        up_inner: 105.0,
                        up_small: 110.0,
                        up_outer: 115.0,
                        up_big: 120.0,
                        current_cross: 0.0,
                        mean_line_slope: 0.0,
                    },
                    dp_dir: 0,
                    zz_dir: 0,
                    ..Default::default()
                },
                tf5m: Default::default(),
                tf15m: Default::default(),
                tf1h: Default::default(),
                fast_candle: None,
                signal: Default::default(),
            },
            &decision(MarketState::Long),
        );
        assert_eq!(result.update_positions.len(), 1);
        assert!(result.note.contains("tp="));
        assert!(result.note.contains("sl="));
    }
}
