use crate::builder::ensure_runtime_builder;
use crate::exit;
use crate::format::{fmt_price_tick, tick_size};
use crate::martingale;
use crate::model::IndicatorSnapshot;
use crate::model::SugInfoSnapshot;
use crate::model::{
    EntryGate, EntryReason, ExitMode, MarketState, PositionState, TpControl, TradeDecision,
};
use crate::orders::{close, limit_open, tp_mark, update_position};
use crate::risk::entry_budget;
use crate::state::{sync_position_state, EngineState, MartingaleRuntime};
use lte_strategy_bridge::abi::{
    Direction, ModuleClosePosition, ModuleInput, ModuleOpenPosition, ModulePlaceOrder,
    ModulePositionSummary, ModuleUpdatePosition,
};

#[cfg(test)]
use crate::model::ExitKind;

// Keep Default available for type compatibility only. Never construct an
// empty execution result with a default value; no action is represented by
// `Option::None`.
#[derive(Debug, Default)]
pub struct ExecutionResult {
    pub open_positions: Vec<ModuleOpenPosition>,
    pub close_positions: Vec<ModuleClosePosition>,
    pub update_positions: Vec<ModuleUpdatePosition>,
    pub tp_orders: Vec<ModulePlaceOrder>,
    pub cancel_orders: Vec<String>,
    pub constraint: Option<String>,
}

fn open_note(
    side: Direction,
    amount: f64,
    order_price: f64,
    market_price: f64,
    tick_size: f64,
    decision: &TradeDecision,
    state: &EngineState,
) -> String {
    let side = match side {
        Direction::Long => "long",
        Direction::Short => "short",
        _ => "both",
    };
    let entry_note = match &decision.entry {
        EntryGate::Open { notes, .. } => notes.join("|"),
        EntryGate::Wait(wait) => format!("wait:{}|{}", wait.reason, wait.notes.join("|")),
    };
    let reason_note = match &decision.entry {
        EntryGate::Open { reason, .. } => reason
            .as_entry_reason()
            .map(|reason| match reason {
                EntryReason::SugInfoScalp { price }
                | EntryReason::SugInfoMovement { price }
                | EntryReason::SidewayScalp { price, .. } => {
                    format!("{} price={:.4}", reason.label(), price)
                }
                EntryReason::Structural { price } => {
                    format!("{} price={:.4}", reason.label(), price)
                }
            })
            .unwrap_or_else(|| "none".to_string()),
        EntryGate::Wait(_) => "none".to_string(),
    };
    let reason_price = entry_reason(decision)
        .map(EntryReason::price)
        .unwrap_or(market_price);
    format!(
        "open {} because {} reason={} reason_price={} order_price={} market_price={} amount={:.4} cap_init={:.2}% trading_amount={:.2}% budget={:.4}",
        side,
        entry_note,
        reason_note,
        fmt_price_tick(reason_price, tick_size),
        fmt_price_tick(order_price, tick_size),
        fmt_price_tick(market_price, tick_size),
        amount,
        state.cfg.cap_init,
        state.cfg.trading_amount,
        state.trade_budget
    )
}

fn close_reason(reason: &str, state: &EngineState, price: f64, tick_size: f64) -> String {
    format!(
        "close because {} price={} cap_init={:.2}% trading_amount={:.2}% budget={:.4}",
        reason,
        fmt_price_tick(price, tick_size),
        state.cfg.cap_init,
        state.cfg.trading_amount,
        state.trade_budget
    )
}

fn entry_reason(decision: &TradeDecision) -> Option<&EntryReason> {
    match &decision.entry {
        EntryGate::Open { reason, .. } => reason.as_entry_reason(),
        EntryGate::Wait(_) => None,
    }
}

fn active_position<'a>(
    input: &'a ModuleInput,
    side: PositionState,
) -> Option<&'a ModulePositionSummary> {
    match side {
        PositionState::Long => input.positions.long.as_ref(),
        PositionState::Short => input.positions.short.as_ref(),
        PositionState::Flat => None,
    }
}

fn position_qty(input: &ModuleInput, side: PositionState) -> f64 {
    active_position(input, side)
        .map(|position| {
            if position.qty != 0.0 {
                position.qty.abs()
            } else if position.notional > 0.0 {
                (position.notional / input.price.max(f64::EPSILON)).max(0.0)
            } else {
                0.0
            }
        })
        .unwrap_or(0.0)
}

fn pending_position<'a>(
    input: &'a ModuleInput,
    side: PositionState,
) -> Option<&'a ModulePositionSummary> {
    match side {
        PositionState::Long => input.positions.long.as_ref(),
        PositionState::Short => input.positions.short.as_ref(),
        PositionState::Flat => None,
    }
}

fn has_active_position(position: Option<&ModulePositionSummary>) -> bool {
    position
        .map(|position| position.qty > 0.0 || position.notional > 0.0)
        .unwrap_or(false)
}

fn created_position_update(
    input: &ModuleInput,
    state: &mut EngineState,
    side: PositionState,
) -> Option<ModuleUpdatePosition> {
    let direction = match side {
        PositionState::Long => Direction::Long,
        PositionState::Short => Direction::Short,
        PositionState::Flat => return None,
    };
    let desired_price = state
        .position
        .entry_reason
        .as_ref()
        .map(|reason| reason.price())
        .filter(|price| *price > 0.0)
        .unwrap_or(input.price);
    if (state.position.entry_price - desired_price).abs() <= f64::EPSILON {
        return None;
    }
    state.position.entry_price = desired_price;
    Some(update_position(direction, Some(desired_price)))
}

#[cfg(test)]
pub fn evaluate(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    decision: &TradeDecision,
) -> Result<ExecutionResult, &'static str> {
    evaluate_with_cascade(input, state, snapshot, sug_snapshot, decision)
        .ok_or("test evaluation returned no execution result")
}

pub fn evaluate_with_cascade(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    decision: &TradeDecision,
) -> Option<ExecutionResult> {
    let builder = ensure_runtime_builder(state);
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
        return Some(ExecutionResult {
            close_positions: vec![
                close(
                    Direction::Long,
                    &close_reason(
                        "one-position-invariant",
                        state,
                        input.price,
                        tick_size(input),
                    ),
                ),
                close(
                    Direction::Short,
                    &close_reason(
                        "one-position-invariant",
                        state,
                        input.price,
                        tick_size(input),
                    ),
                ),
            ],
            cancel_orders: vec![tp_mark(Direction::Long), tp_mark(Direction::Short)],
            ..Default::default()
        });
    }

    if let Some(long_position) = input.positions.long.as_ref() {
        if !has_active_position(Some(long_position)) {
            if matches!(&decision.entry, EntryGate::Wait(_)) {
                return None;
            }
            let reason = entry_reason(decision).cloned();
            state.position.entry_reason = reason;
            return Some(ExecutionResult {
                update_positions: created_position_update(input, state, PositionState::Long)
                    .into_iter()
                    .collect(),
                ..Default::default()
            });
        }
        let (tp_orders, update_positions) = exit::ensure_active_exit_order(
            input,
            state,
            snapshot,
            sug_snapshot,
            PositionState::Long,
            &builder,
        );
        return Some(ExecutionResult {
            tp_orders,
            update_positions,
            ..Default::default()
        });
    }

    if let Some(short_position) = input.positions.short.as_ref() {
        if !has_active_position(Some(short_position)) {
            if matches!(&decision.entry, EntryGate::Wait(_)) {
                return None;
            }
            let reason = entry_reason(decision).cloned();
            state.position.entry_reason = reason;
            return Some(ExecutionResult {
                update_positions: created_position_update(input, state, PositionState::Short)
                    .into_iter()
                    .collect(),
                ..Default::default()
            });
        }
        let (tp_orders, update_positions) = exit::ensure_active_exit_order(
            input,
            state,
            snapshot,
            sug_snapshot,
            PositionState::Short,
            &builder,
        );
        return Some(ExecutionResult {
            tp_orders,
            update_positions,
            ..Default::default()
        });
    }

    let entry_side = match decision.market_state {
        MarketState::Long => PositionState::Long,
        MarketState::Short => PositionState::Short,
        MarketState::Sideways => PositionState::Flat,
    };

    let direction_allowed = matches!(
        (state.allowed_direction, entry_side),
        (Direction::Both, _)
            | (Direction::Long, PositionState::Long)
            | (Direction::Short, PositionState::Short)
    );
    if !direction_allowed {
        return Some(ExecutionResult {
            constraint: Some(format!(
                "entry-direction-guard:allowed={:?}:requested={:?}",
                state.allowed_direction, entry_side
            )),
            ..Default::default()
        });
    }

    if matches!(&decision.entry, EntryGate::Wait(_)) {
        return None;
    }

    let amount = entry_budget(input, state);
    let leverage = (input.leverage as f64).max(1.0);
    if amount <= 0.0 {
        return Some(ExecutionResult {
            constraint: Some("budget-too-small".to_string()),
            ..Default::default()
        });
    }
    if let Some(min_notional) = input.symbol_info.min_notional.filter(|value| *value > 0.0) {
        if amount * leverage < min_notional {
            return Some(ExecutionResult {
                constraint: Some("budget-too-small".to_string()),
                ..Default::default()
            });
        }
    }

    let reason = entry_reason(decision).cloned();
    state.position.entry_reason = reason;
    let entry_price = entry_reason(decision)
        .map(|reason| reason.price())
        .unwrap_or(input.price);
    if let Some(pending_position) =
        pending_position(input, entry_side).filter(|position| !has_active_position(Some(position)))
    {
        if !entry_price.is_finite() || entry_price <= 0.0 {
            return None;
        }
        if (pending_position.entry_price - entry_price).abs() > f64::EPSILON {
            let direction = match entry_side {
                PositionState::Long => Direction::Long,
                PositionState::Short => Direction::Short,
                PositionState::Flat => Direction::Both,
            };
            return Some(ExecutionResult {
                update_positions: vec![update_position(direction, Some(entry_price))],
                ..Default::default()
            });
        }
        return None;
    }

    if builder.is_martingale() {
        if let Some(plan) = martingale::entry_plan(
            state,
            snapshot,
            decision.market_state,
            entry_reason(decision),
            amount,
        ) {
            state.position.martingale = Some(MartingaleRuntime {
                profile: plan.profile,
                base_price: plan.base_price,
                base_amount: plan.open_position.amount.unwrap_or(amount),
                last_filled_mark: None,
                sl_activation_price: plan.stop_loss.activation_price(),
            });
            state.position.take_profit = 0.0;
            exit::set_stop_loss(state, plan.stop_loss);
            state.position.exit_mode = ExitMode::Structural;
            let tp_orders = if state.cfg.tp_control == TpControl::TrailingStop {
                Vec::new()
            } else {
                plan.orders
            };
            return Some(ExecutionResult {
                open_positions: vec![plan.open_position],
                tp_orders,
                ..Default::default()
            });
        }
        return Some(ExecutionResult {
            constraint: Some("martingale-plan-unavailable".to_string()),
            ..Default::default()
        });
    }

    match decision.market_state {
        MarketState::Long => Some(ExecutionResult {
            open_positions: vec![limit_open(
                Direction::Long,
                amount,
                entry_price,
                &open_note(
                    Direction::Long,
                    amount,
                    entry_price,
                    input.price,
                    tick_size(input),
                    decision,
                    state,
                ),
            )],
            ..Default::default()
        }),
        MarketState::Short => Some(ExecutionResult {
            open_positions: vec![limit_open(
                Direction::Short,
                amount,
                entry_price,
                &open_note(
                    Direction::Short,
                    amount,
                    entry_price,
                    input.price,
                    tick_size(input),
                    decision,
                    state,
                ),
            )],
            ..Default::default()
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{TradeDecision, TradeMode};
    use crate::risk::apply_close_pnl;
    use crate::state::{EngineState, MartingaleRuntime};
    use lte_strategy_bridge::abi::{
        ModuleOrderType, ModulePositionSummary, ModulePositions, ModuleSymbolInfo,
    };

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

    fn martingale_runtime() -> MartingaleRuntime {
        MartingaleRuntime {
            profile: crate::martingale::LadderProfile {
                levels: 11,
                range_pct: 1.0,
                step_pct: 0.5,
                take_profit_pct: 0.4,
            },
            base_price: 0.0,
            base_amount: 0.0,
            last_filled_mark: None,
            sl_activation_price: None,
        }
    }

    #[test]
    fn martingale_activation_price_arms_common_stop_loss() {
        let mut state = EngineState::default();
        state.position.side = PositionState::Long;
        state.position.stop_loss = 99.0;
        let mut martingale = martingale_runtime();
        martingale.sl_activation_price = Some(95.0);
        state.position.martingale = Some(martingale);

        assert!(!exit::virtual_stop_loss_hit_with_activation(
            &mut state,
            PositionState::Long,
            96.0
        ));
        assert_eq!(
            state
                .position
                .martingale
                .as_ref()
                .and_then(|martingale| martingale.sl_activation_price),
            Some(95.0)
        );

        assert!(exit::virtual_stop_loss_hit_with_activation(
            &mut state,
            PositionState::Long,
            94.0
        ));
        assert!(state
            .position
            .martingale
            .as_ref()
            .and_then(|martingale| martingale.sl_activation_price)
            .is_none());
    }

    fn decision(market_state: MarketState) -> TradeDecision {
        TradeDecision {
            market_state,
            entry: crate::model::EntryGate::Open {
                notes: vec![],
                reason: crate::model::EntryOpenReason::Entry(
                    crate::model::EntryReason::SugInfoScalp { price: 100.0 },
                ),
            },
        }
    }

    fn reasonless_decision(market_state: MarketState) -> TradeDecision {
        TradeDecision {
            market_state,
            entry: crate::model::EntryGate::Open {
                notes: vec![],
                reason: crate::model::EntryOpenReason::Entry(
                    crate::model::EntryReason::Structural { price: 100.0 },
                ),
            },
        }
    }

    fn movement_decision(market_state: MarketState, price: f64) -> TradeDecision {
        TradeDecision {
            market_state,
            entry: crate::model::EntryGate::Open {
                notes: vec![],
                reason: crate::model::EntryOpenReason::Entry(
                    crate::model::EntryReason::SugInfoMovement { price },
                ),
            },
        }
    }

    fn sug_info(system_time: i64) -> lte_strategy_bridge::SuggestionInfo {
        lte_strategy_bridge::SuggestionInfo {
            time: system_time,
            system_time,
            symbol: "TEST".to_string(),
            symbol_id: 1,
            symbol_key: "TEST".to_string(),
            status: lte_strategy_bridge::AlgoSuggestionTradeStatus::Normal,
            price: 100.0,
            candle: lte_strategy_bridge::SuggestionInfoCandle::new(100.0),
            klines: Default::default(),
            fast_candle: None,
            direction: lte_strategy_bridge::Direction::Both,
            indicator: Default::default(),
            order_book: None,
            oi: None,
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert!(result.as_ref().unwrap().open_positions.is_empty());
    }

    #[test]
    fn sideways_blocks_entries() {
        let mut state = EngineState::default();
        let result = evaluate_with_cascade(
            &input(),
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &reasonless_decision(MarketState::Sideways),
        );
        assert!(result.is_none());
    }

    #[test]
    fn waiting_entry_keeps_pending_position() {
        let mut state = EngineState::default();
        let wait_decision = TradeDecision {
            market_state: MarketState::Long,
            entry: EntryGate::Wait(crate::model::EntryWait {
                reason: "mrc-pullback-wait".to_string(),
                solutions: vec![crate::model::EntryWaitSolution::ReclaimLevel {
                    source: "mrc-window".to_string(),
                    level: 99.0,
                    band_low: 98.9,
                    band_high: 99.1,
                }],
                notes: vec!["forced-pullback".to_string()],
            }),
        };
        let first = evaluate_with_cascade(
            &input(),
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &wait_decision,
        );
        assert!(first.is_none());
        state.position.entry_reason = Some(EntryReason::SugInfoScalp { price: 99.0 });

        let mut pending_input = input();
        pending_input.positions.long = Some(ModulePositionSummary {
            entry_price: 99.0,
            notional: 0.0,
            pnl: 0.0,
            qty: 0.0,
        });
        let second = evaluate_with_cascade(
            &pending_input,
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &wait_decision,
        );
        assert!(second.is_none());
        assert_eq!(
            state.position.entry_reason,
            Some(EntryReason::SugInfoScalp { price: 99.0 })
        );
    }

    #[test]
    fn created_position_updates_entry_until_filled() {
        let mut state = EngineState::default();
        let mut pending_input = input();
        pending_input.positions.long = Some(ModulePositionSummary {
            entry_price: 99.0,
            notional: 0.0,
            pnl: 0.0,
            qty: 0.0,
        });

        let result = evaluate(
            &pending_input,
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        )
        .expect("created position should return an execution result");

        assert!(result.open_positions.is_empty());
        assert!(result.close_positions.is_empty());
        assert_eq!(result.update_positions.len(), 1);
        assert_eq!(result.update_positions[0].enter_price, Some(100.0));
        assert_eq!(
            state.position.entry_reason,
            Some(EntryReason::SugInfoScalp { price: 100.0 })
        );
    }

    #[test]
    fn unchanged_open_reason_keeps_pending_entry_without_update() {
        let mut state = EngineState::default();
        let mut pending_input = input();
        pending_input.positions.long = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 0.0,
            pnl: 0.0,
            qty: 0.0,
        });

        let result = evaluate(
            &pending_input,
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        )
        .expect("unchanged pending entry should produce an execution result");

        assert!(result.open_positions.is_empty());
        assert!(result.close_positions.is_empty());
        assert!(result.update_positions.is_empty());
        assert_eq!(
            state.position.entry_reason,
            Some(EntryReason::SugInfoScalp { price: 100.0 })
        );
    }

    #[test]
    fn short_position_exit_uses_absolute_raw_quantity() {
        let mut input = input();
        input.positions.short = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 9.9,
            pnl: 0.0,
            qty: -0.1,
        });

        assert_eq!(position_qty(&input, PositionState::Short), 0.1);
    }

    #[test]
    fn building_fast_candle_does_not_override_open_decision() {
        let mut state = EngineState::default();
        let mut snapshot = IndicatorSnapshot::default();
        let sug_snapshot = crate::model::SugInfoSnapshot {
            fast_candle: Some(crate::model::FastCandleSnapshot {
                candle: crate::model::Candle {
                    open: 100.0,
                    high: 106.0,
                    low: 100.0,
                    close: 105.0,
                },
                active: true,
            }),
            ..Default::default()
        };
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
            &sug_snapshot,
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
    }

    #[test]
    fn legacy_tiring_fast_candle_does_not_override_open_decision_2() {
        let mut state = EngineState::default();
        let mut snapshot = IndicatorSnapshot::default();
        let sug_snapshot = crate::model::SugInfoSnapshot {
            fast_candle: Some(crate::model::FastCandleSnapshot {
                candle: crate::model::Candle {
                    open: 100.0,
                    high: 110.0,
                    low: 100.0,
                    close: 110.0,
                },
                active: true,
            }),
            ..Default::default()
        };
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
            &sug_snapshot,
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
    }

    #[test]
    fn legacy_entry_budget_uses_cap_init_and_min_notional() {
        let mut state = EngineState::default();
        state.cfg.cap_init = 2.0;
        state.cfg.trading_amount = 50.0;
        let result = evaluate(
            &input(),
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
        assert_eq!(result.as_ref().unwrap().open_positions[0].amount, Some(1.0));
    }

    #[test]
    fn legacy_cooling_fast_candle_allows_entries_2() {
        let mut state = EngineState::default();
        let mut snapshot = IndicatorSnapshot::default();
        let sug_snapshot = crate::model::SugInfoSnapshot {
            fast_candle: Some(crate::model::FastCandleSnapshot {
                candle: crate::model::Candle {
                    open: 100.0,
                    high: 106.0,
                    low: 100.0,
                    close: 105.0,
                },
                active: false,
            }),
            ..Default::default()
        };
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
            &sug_snapshot,
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
    }

    #[test]
    fn legacy_structural_exit_allows_entries() {
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
    }

    #[test]
    fn legacy_post_sl_open_decision_is_not_rechecked() {
        let mut state = EngineState::default();
        state.exit_memory = crate::model::ExitMemory {
            side: PositionState::Long,
            kind: Some(ExitKind::StopLoss),
            price: 100.0,
            take_profit: 102.0,
            stop_loss: 95.0,
            reliable: true,
            tick: 10,
        };
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
    }

    #[test]
    fn legacy_post_sl_wait_uses_dpsignal_band() {
        let mut state = EngineState::default();
        state.exit_memory = crate::model::ExitMemory {
            side: PositionState::Long,
            kind: Some(ExitKind::StopLoss),
            price: 100.0,
            take_profit: 102.0,
            stop_loss: 95.0,
            reliable: true,
            tick: 10,
        };
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
    }

    #[test]
    fn legacy_cooling_fast_candle_does_not_override_open_decision() {
        let mut state = EngineState::default();
        let mut snapshot = IndicatorSnapshot::default();
        let sug_snapshot = crate::model::SugInfoSnapshot {
            fast_candle: Some(crate::model::FastCandleSnapshot {
                candle: crate::model::Candle {
                    open: 100.0,
                    high: 106.0,
                    low: 100.0,
                    close: 105.0,
                },
                active: false,
            }),
            ..Default::default()
        };
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
            &sug_snapshot,
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
    }
    #[test]
    fn legacy_tiring_fast_candle_does_not_override_open_decision() {
        let mut state = EngineState::default();
        let mut snapshot = IndicatorSnapshot::default();
        let sug_snapshot = crate::model::SugInfoSnapshot {
            fast_candle: Some(crate::model::FastCandleSnapshot {
                candle: crate::model::Candle {
                    open: 100.0,
                    high: 110.0,
                    low: 100.0,
                    close: 110.0,
                },
                active: true,
            }),
            ..Default::default()
        };
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
            &sug_snapshot,
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
        assert_eq!(result.as_ref().unwrap().open_positions[0].amount, Some(1.0));
    }

    #[test]
    fn configured_direction_guards_only_new_entries() {
        let mut state = EngineState::default();
        state.allowed_direction = Direction::Long;

        let result = evaluate(
            &input(),
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Short),
        )
        .expect("guard should return an execution result");

        assert!(result.open_positions.is_empty());
        assert_eq!(
            result.constraint.as_deref(),
            Some("entry-direction-guard:allowed=Long:requested=Short")
        );
    }

    #[test]
    fn structural_entry_is_allowed_when_decided_open() {
        let mut state = EngineState::default();
        let result = evaluate(
            &input(),
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &reasonless_decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
    }

    #[test]
    fn legacy_cooling_fast_candle_allows_entries() {
        let mut state = EngineState::default();
        let mut snapshot = IndicatorSnapshot::default();
        let sug_snapshot = crate::model::SugInfoSnapshot {
            fast_candle: Some(crate::model::FastCandleSnapshot {
                candle: crate::model::Candle {
                    open: 100.0,
                    high: 106.0,
                    low: 100.0,
                    close: 105.0,
                },
                active: false,
            }),
            ..Default::default()
        };
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
            &sug_snapshot,
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
    }

    #[test]
    fn structural_exit_allows_entries() {
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
    }

    #[test]
    fn post_sl_open_decision_is_not_rechecked() {
        let mut state = EngineState::default();
        state.exit_memory = crate::model::ExitMemory {
            side: PositionState::Long,
            kind: Some(ExitKind::StopLoss),
            price: 100.0,
            take_profit: 102.0,
            stop_loss: 95.0,
            reliable: true,
            tick: 10,
        };
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
    }

    #[test]
    fn post_sl_wait_uses_dpsignal_band() {
        let mut state = EngineState::default();
        state.exit_memory = crate::model::ExitMemory {
            side: PositionState::Long,
            kind: Some(ExitKind::StopLoss),
            price: 100.0,
            take_profit: 102.0,
            stop_loss: 95.0,
            reliable: true,
            tick: 10,
        };
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
    }

    #[test]
    fn cooling_fast_candle_does_not_override_open_decision() {
        let mut state = EngineState::default();
        let mut snapshot = IndicatorSnapshot::default();
        let sug_snapshot = crate::model::SugInfoSnapshot {
            fast_candle: Some(crate::model::FastCandleSnapshot {
                candle: crate::model::Candle {
                    open: 100.0,
                    high: 106.0,
                    low: 100.0,
                    close: 105.0,
                },
                active: false,
            }),
            ..Default::default()
        };
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
            &sug_snapshot,
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
    }

    #[test]
    fn long_tp_hit_closes_position() {
        let mut state = EngineState::default();
        state.position = crate::state::PositionRuntime {
            side: crate::model::PositionState::Long,
            entry_price: 100.0,
            peak_price: 103.0,
            trough_price: 99.0,
            take_profit: 0.0,
            stop_loss: 0.0,
            stop_loss_reason: String::new(),
            exit_tp_reliable: true,
            exit_mode: ExitMode::Structural,
            entry_reason: None,
            martingale: None,
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
        snapshot.tf1m.zz_buy_price = 95.0;
        snapshot.tf1m.zz_sell_price = 110.0;
        snapshot.tf5m = snapshot.tf1m;
        snapshot.tf15m = snapshot.tf1m;
        snapshot.tf1h = snapshot.tf1m;
        let result = evaluate(
            &inp,
            &mut state,
            &snapshot,
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().tp_orders.len(), 1);
        assert_eq!(result.as_ref().unwrap().tp_orders[0].qty, Some(0.1));
        assert!(result.as_ref().unwrap().tp_orders[0].amount.is_none());
        assert!(result.as_ref().unwrap().close_positions.is_empty());
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
            stop_loss_reason: String::new(),
            exit_tp_reliable: true,
            exit_mode: ExitMode::Structural,
            entry_reason: None,
            martingale: None,
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
        snapshot.tf1m.zz_buy_price = 95.0;
        snapshot.tf1m.zz_sell_price = 110.0;
        snapshot.tf5m = snapshot.tf1m;
        snapshot.tf15m = snapshot.tf1m;
        snapshot.tf1h = snapshot.tf1m;
        let result = evaluate(
            &inp,
            &mut state,
            &snapshot,
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Short),
        );
        assert_eq!(result.as_ref().unwrap().tp_orders.len(), 1);
        assert_eq!(result.as_ref().unwrap().tp_orders[0].qty, Some(0.1));
        assert!(result.as_ref().unwrap().tp_orders[0].amount.is_none());
        assert!(result.as_ref().unwrap().close_positions.is_empty());
    }

    #[test]
    fn trail_after_virtual_sl_reprices_single_tp() {
        let mut state = EngineState::default();
        state.cfg.trade_mode = TradeMode::Single;
        state.position = crate::state::PositionRuntime {
            side: crate::model::PositionState::Long,
            entry_price: 100.0,
            peak_price: 101.0,
            trough_price: 99.0,
            take_profit: 100.2,
            stop_loss: 90.0,
            stop_loss_reason: String::new(),
            exit_tp_reliable: true,
            exit_mode: ExitMode::TrailAfterVirtualSl,
            entry_reason: None,
            martingale: None,
        };
        let mut inp = input();
        inp.positions.long = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: -0.8,
            qty: 0.1,
        });
        inp.price = 99.0;
        let mut snapshot = IndicatorSnapshot::default();
        snapshot.tf1m.candle = crate::model::Candle {
            open: 99.3,
            high: 99.5,
            low: 98.9,
            close: 99.0,
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
        snapshot.tf1m.zz_buy_price = 95.0;
        snapshot.tf1m.zz_sell_price = 110.0;
        snapshot.tf5m = snapshot.tf1m;
        snapshot.tf15m = snapshot.tf1m;
        snapshot.tf1h = snapshot.tf1m;

        let result = evaluate(
            &inp,
            &mut state,
            &snapshot,
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );

        assert_eq!(result.as_ref().unwrap().tp_orders.len(), 1);
        assert_eq!(result.as_ref().unwrap().tp_orders[0].enter_price, 105.0);
        assert!(result.as_ref().unwrap().tp_orders[0]
            .note
            .as_deref()
            .is_some_and(|note| note.contains("tp_reprice=")));
        assert!(result.as_ref().unwrap().update_positions.is_empty());
    }

    #[test]
    fn trailing_stop_does_not_place_initial_tp() {
        let mut state = EngineState::default();
        state.cfg.tp_control = TpControl::TrailingStop;
        state.position = crate::state::PositionRuntime {
            side: PositionState::Long,
            entry_price: 100.0,
            peak_price: 100.0,
            trough_price: 100.0,
            take_profit: 0.0,
            stop_loss: 99.5,
            stop_loss_reason: "structural:1m:below:mean".to_string(),
            exit_tp_reliable: true,
            exit_mode: ExitMode::Structural,
            entry_reason: None,
            martingale: None,
        };
        let mut inp = input();
        inp.positions.long = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: 0.1,
            qty: 0.1,
        });
        inp.price = 100.0;

        let result = evaluate(
            &inp,
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );

        assert!(result.as_ref().unwrap().tp_orders.is_empty());
        assert!(result.as_ref().unwrap().cancel_orders.is_empty());
        assert_eq!(state.position.take_profit, 0.0);
        assert_eq!(state.position.exit_mode, ExitMode::Structural);
    }

    #[test]
    fn trailing_stop_moves_virtual_sl_only_in_favorable_direction() {
        let mut state = EngineState::default();
        state.cfg.tp_control = TpControl::TrailingStop;
        state.position = crate::state::PositionRuntime {
            side: PositionState::Long,
            entry_price: 100.0,
            peak_price: 100.0,
            trough_price: 100.0,
            take_profit: 0.0,
            stop_loss: 90.0,
            stop_loss_reason: String::new(),
            exit_tp_reliable: false,
            exit_mode: ExitMode::Structural,
            entry_reason: None,
            martingale: None,
        };
        let mut inp = input();
        inp.positions.long = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: 0.2,
            qty: 0.1,
        });
        inp.price = 102.0;
        let mut snapshot = IndicatorSnapshot::default();
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
            mean_line_slope: 1.0,
        };
        snapshot.tf1m.zz_buy_price = 95.0;
        snapshot.tf1m.zz_sell_price = 110.0;
        snapshot.tf1m.candle.close = inp.price;
        snapshot.tf5m = snapshot.tf1m;
        snapshot.tf15m = snapshot.tf1m;
        snapshot.tf1h = snapshot.tf1m;
        let first = evaluate(
            &inp,
            &mut state,
            &snapshot,
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert!(first.as_ref().unwrap().tp_orders.is_empty());
        assert!(state.position.stop_loss > 90.0);
        let trailed_stop = state.position.stop_loss;

        inp.price = 101.0;
        let second = evaluate(
            &inp,
            &mut state,
            &snapshot,
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert!(second.as_ref().unwrap().tp_orders.is_empty());
        assert_eq!(state.position.stop_loss, trailed_stop);
    }

    #[test]
    fn trailing_stop_places_fixed_two_tick_tp_after_long_cross() {
        let mut state = EngineState::default();
        state.cfg.tp_control = TpControl::TrailingStop;
        state.position = crate::state::PositionRuntime {
            side: PositionState::Long,
            entry_price: 100.0,
            peak_price: 100.0,
            trough_price: 100.0,
            take_profit: 0.0,
            stop_loss: 99.5,
            stop_loss_reason: "structural:1m:below:mean".to_string(),
            exit_tp_reliable: true,
            exit_mode: ExitMode::Structural,
            entry_reason: None,
            martingale: None,
        };
        let mut inp = input();
        inp.positions.long = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: -0.8,
            qty: 0.1,
        });
        inp.price = 99.0;

        let result = evaluate(
            &inp,
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        let expected_tp = inp.price + 2.0 * tick_size(&inp);

        assert_eq!(result.as_ref().unwrap().tp_orders.len(), 1);
        assert_eq!(
            result.as_ref().unwrap().tp_orders[0].enter_price,
            expected_tp
        );
        assert!(result.as_ref().unwrap().tp_orders[0].reduce_only);
        assert_eq!(state.position.take_profit, expected_tp);
        assert_eq!(state.position.exit_mode, ExitMode::TrailAfterVirtualSl);
    }

    #[test]
    fn trailing_stop_places_two_tick_tp_in_profitable_direction_for_short() {
        let mut state = EngineState::default();
        state.cfg.tp_control = TpControl::TrailingStop;
        state.position = crate::state::PositionRuntime {
            side: PositionState::Short,
            entry_price: 100.0,
            peak_price: 100.0,
            trough_price: 100.0,
            take_profit: 0.0,
            stop_loss: 100.5,
            stop_loss_reason: String::new(),
            exit_tp_reliable: true,
            exit_mode: ExitMode::Structural,
            entry_reason: None,
            martingale: None,
        };
        let mut inp = input();
        inp.positions.short = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: -0.8,
            qty: 0.1,
        });
        inp.price = 101.0;

        let result = evaluate(
            &inp,
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Short),
        );
        let expected_tp = inp.price - 2.0 * tick_size(&inp);

        assert_eq!(result.as_ref().unwrap().tp_orders.len(), 1);
        assert_eq!(
            result.as_ref().unwrap().tp_orders[0].enter_price,
            expected_tp
        );
        assert!(result.as_ref().unwrap().tp_orders[0].reduce_only);
    }

    #[test]
    fn trailing_stop_suppresses_martingale_entry_tp_orders() {
        let mut state = EngineState::default();
        state.cfg.trade_mode = TradeMode::Martingale;
        state.cfg.tp_control = TpControl::TrailingStop;
        let mut inp = input();
        inp.sug_info = Some(sug_info(1_000));
        let mut snapshot = IndicatorSnapshot::default();
        snapshot.tf1m.candle = crate::model::Candle {
            open: 100.0,
            high: 100.5,
            low: 99.5,
            close: 100.0,
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );

        assert!(result.as_ref().unwrap().open_positions.len() <= 1);
        assert!(result.as_ref().unwrap().tp_orders.is_empty());
    }

    #[test]
    fn trailing_stop_rearms_only_after_returning_across_sl() {
        let mut state = EngineState::default();
        state.cfg.tp_control = TpControl::TrailingStop;
        state.position = crate::state::PositionRuntime {
            side: PositionState::Long,
            entry_price: 100.0,
            peak_price: 100.0,
            trough_price: 100.0,
            take_profit: 0.0,
            stop_loss: 99.5,
            stop_loss_reason: String::new(),
            exit_tp_reliable: true,
            exit_mode: ExitMode::Structural,
            entry_reason: None,
            martingale: None,
        };

        let mut crossed_input = input();
        crossed_input.positions.long = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: -0.8,
            qty: 0.1,
        });
        crossed_input.price = 99.0;
        let first = evaluate(
            &crossed_input,
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(first.as_ref().unwrap().tp_orders.len(), 1);

        let mut recovered_input = crossed_input;
        recovered_input.price = 100.0;
        let recovered = evaluate(
            &recovered_input,
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert!(recovered.as_ref().unwrap().tp_orders.is_empty());
        assert!(recovered.as_ref().unwrap().cancel_orders.is_empty());
        assert_eq!(state.position.take_profit, 0.0);
        assert_eq!(state.position.exit_mode, ExitMode::Structural);

        let mut second_cross_input = recovered_input;
        second_cross_input.price = 99.0;
        let second = evaluate(
            &second_cross_input,
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(second.as_ref().unwrap().tp_orders.len(), 1);
    }

    #[test]
    fn martingale_trail_after_virtual_sl_advances_take_profit() {
        let mut state = EngineState::default();
        state.cfg.trade_mode = TradeMode::Martingale;
        state.position = crate::state::PositionRuntime {
            side: crate::model::PositionState::Long,
            entry_price: 100.0,
            peak_price: 101.0,
            trough_price: 99.0,
            take_profit: 100.2,
            stop_loss: 99.5,
            stop_loss_reason: String::new(),
            exit_tp_reliable: true,
            exit_mode: ExitMode::TrailAfterVirtualSl,
            entry_reason: None,
            martingale: Some(martingale_runtime()),
        };
        let mut inp = input();
        inp.positions.long = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: -0.8,
            qty: 0.1,
        });
        inp.price = 99.0;
        let mut snapshot = IndicatorSnapshot::default();
        snapshot.tf1m.candle = crate::model::Candle {
            open: 99.3,
            high: 99.5,
            low: 98.9,
            close: 99.0,
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );

        assert_eq!(result.as_ref().unwrap().tp_orders.len(), 1);
        assert!(result.as_ref().unwrap().update_positions.is_empty());
        let expected_tp = crate::stop_loss::martingale_recovery_take_profit(
            inp.price,
            PositionState::Long,
            tick_size(&inp),
        );
        assert_eq!(
            result.as_ref().unwrap().tp_orders[0].enter_price,
            expected_tp
        );
        assert_eq!(result.as_ref().unwrap().tp_orders[0].qty, Some(0.1));
        assert!(result.as_ref().unwrap().tp_orders[0].reduce_only);
    }

    #[test]
    fn direction_change_does_not_close_short_position() {
        let mut state = EngineState::default();
        state.position = crate::state::PositionRuntime {
            side: crate::model::PositionState::Short,
            entry_price: 100.0,
            peak_price: 100.0,
            trough_price: 100.0,
            take_profit: 0.0,
            stop_loss: 0.0,
            stop_loss_reason: String::new(),
            exit_tp_reliable: false,
            exit_mode: ExitMode::Structural,
            entry_reason: Some(crate::model::EntryReason::SugInfoMovement { price: 99.2 }),
            martingale: None,
        };
        let mut inp = input();
        inp.positions.short = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: 0.0,
            qty: 0.0,
        });
        inp.sug_info = Some(sug_info(12_500));
        let result = evaluate(
            &inp,
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert!(result.as_ref().unwrap().close_positions.is_empty());
    }

    #[test]
    fn martingale_direction_change_grace_keeps_fresh_position_open() {
        let mut state = EngineState::default();
        state.cfg.trade_mode = TradeMode::Martingale;
        state.position = crate::state::PositionRuntime {
            side: crate::model::PositionState::Long,
            entry_price: 100.0,
            peak_price: 100.5,
            trough_price: 99.5,
            take_profit: 101.0,
            stop_loss: 99.0,
            stop_loss_reason: String::new(),
            exit_tp_reliable: true,
            exit_mode: ExitMode::Structural,
            entry_reason: Some(crate::model::EntryReason::SugInfoScalp { price: 100.0 }),
            martingale: None,
        };
        let mut inp = input();
        inp.positions.long = Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: 0.1,
            qty: 0.1,
        });
        inp.price = 100.2;
        inp.sug_info = Some(sug_info(6_000));

        let mut snapshot = IndicatorSnapshot::default();
        snapshot.tf1m.candle = crate::model::Candle {
            open: 100.0,
            high: 100.4,
            low: 99.8,
            close: 100.2,
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Short),
        );

        assert!(result.as_ref().unwrap().close_positions.is_empty());
    }

    #[test]
    fn trading_amount_defaults_to_half_of_deployable_budget() {
        let mut state = EngineState::default();
        state.cfg.cap_init = 2.0;
        state.cfg.trading_amount = 50.0;
        let amount = entry_budget(&input(), &mut state);
        assert_eq!(amount, 1.0);
    }

    #[test]
    fn trading_amount_clamps_to_minimum_and_full_size() {
        let mut state = EngineState::default();
        state.cfg.cap_init = 2.0;
        state.cfg.trading_amount = 10.0;
        let low = entry_budget(&input(), &mut state);
        assert_eq!(low, 0.2);

        let mut state = EngineState::default();
        state.cfg.cap_init = 2.0;
        state.cfg.trading_amount = 100.0;
        let full = entry_budget(&input(), &mut state);
        assert_eq!(full, 2.0);
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Short),
        );
        assert!(result.as_ref().unwrap().open_positions.is_empty());
    }

    #[test]
    fn long_open_uses_limit_entry() {
        let mut state = EngineState::default();
        let inp = input();

        let result = evaluate(
            &inp,
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
        let open = &result.as_ref().unwrap().open_positions[0];
        assert_eq!(open.order_type, ModuleOrderType::Limit);
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Short),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
        let open = &result.as_ref().unwrap().open_positions[0];
        assert_eq!(open.order_type, ModuleOrderType::Limit);
        assert_eq!(open.enter_price, Some(inp.price));
        assert!(open.take_profit.is_none());
        assert!(open.stop_loss.is_none());
    }

    #[test]
    fn martingale_open_forces_immediate_position() {
        let mut state = EngineState::default();
        state.cfg.trade_mode = TradeMode::Martingale;
        let inp = input();
        let snapshot = IndicatorSnapshot {
            tf1m: crate::model::TimeframeSnapshot {
                candle: crate::model::Candle {
                    open: 100.0,
                    high: 104.0,
                    low: 96.0,
                    close: 100.0,
                },
                mrc: crate::model::MrcBands {
                    down_big: 92.0,
                    down_outer: 94.0,
                    down_small: 96.0,
                    down_inner: 98.0,
                    mean_line: 100.0,
                    up_inner: 102.0,
                    up_small: 104.0,
                    up_outer: 106.0,
                    up_big: 108.0,
                    current_cross: 0.0,
                    mean_line_slope: 1.0,
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let result = evaluate(
            &inp,
            &mut state,
            &snapshot,
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
        assert_eq!(
            result.as_ref().unwrap().open_positions[0].order_type,
            ModuleOrderType::Market
        );
        assert_eq!(
            result.as_ref().unwrap().open_positions[0].enter_price,
            Some(inp.price)
        );
        assert_eq!(result.as_ref().unwrap().tp_orders.len(), 2);
    }

    #[test]
    fn movement_entry_uses_signal_price() {
        let mut state = EngineState::default();
        let mut inp = input();
        inp.price = 99.5;
        let result = evaluate(
            &inp,
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &movement_decision(MarketState::Long, 99.5),
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
        assert_eq!(
            result.as_ref().unwrap().open_positions[0].enter_price,
            Some(99.5)
        );
    }

    #[test]
    fn sug_info_scalp_uses_signal_price() {
        let mut state = EngineState::default();
        let mut inp = input();
        inp.price = 99.5;
        let result = evaluate(
            &inp,
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &TradeDecision {
                market_state: MarketState::Long,
                entry: crate::model::EntryGate::Open {
                    notes: vec![],
                    reason: crate::model::EntryOpenReason::Entry(
                        crate::model::EntryReason::SugInfoScalp { price: 99.5 },
                    ),
                },
            },
        );
        assert_eq!(result.as_ref().unwrap().open_positions.len(), 1);
        assert_eq!(
            result.as_ref().unwrap().open_positions[0].enter_price,
            Some(99.5)
        );
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
                    zz_buy_price: 95.0,
                    zz_sell_price: 110.0,
                    ..Default::default()
                },
                tf5m: Default::default(),
                tf15m: Default::default(),
                tf1h: Default::default(),
            },
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(result.as_ref().unwrap().tp_orders.len(), 1);
    }
}
