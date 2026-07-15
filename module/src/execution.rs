use crate::detection::cascade::CascadeSignal;
use crate::exit;
use crate::martingale;
use crate::model::IndicatorSnapshot;
use crate::model::SugInfoSnapshot;
use crate::model::{
    EntryGate, EntryReason, ExitMode, MarketState, PositionState, TradeDecision, TradeMode,
};
use crate::orders::{close, limit_open, tp_limit_order, tp_mark, update_position};
use crate::prediction;
use crate::risk::entry_budget;
use crate::state::{sync_position_state, EngineState};
use crate::stop_loss;
use lte_strategy_bridge::abi::{
    Direction, ModuleClosePosition, ModuleInput, ModuleOpenPosition, ModulePlaceOrder,
    ModulePositionSummary, ModuleUpdatePosition,
};

#[cfg(test)]
use crate::model::ExitKind;

const CREATED_MAX_AGE_MS: u64 = 10_000;
#[derive(Debug, Default)]
pub struct ExecutionResult {
    pub open_positions: Vec<ModuleOpenPosition>,
    pub close_positions: Vec<ModuleClosePosition>,
    pub update_positions: Vec<ModuleUpdatePosition>,
    pub tp_orders: Vec<ModulePlaceOrder>,
    pub cancel_orders: Vec<String>,
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
    let reason_note = decision
        .entry_reason
        .as_ref()
        .map(|reason| match reason {
            EntryReason::SugInfoScalp {
                price,
                started_at_ms,
            }
            | EntryReason::SugInfoMovement {
                price,
                started_at_ms,
            }
            | EntryReason::SidewayScalp {
                price,
                started_at_ms,
                ..
            } => format!(
                "{} price={:.4} started_at_ms={}",
                reason.label(),
                price,
                started_at_ms
            ),
        })
        .unwrap_or_else(|| "none".to_string());
    format!(
        "open {} because {} reason={} price={:.4} amount={:.4} cap_init={:.2}% trading_amount={:.2}% budget={:.4}",
        side,
        entry_note,
        reason_note,
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
        reason, price, state.cfg.cap_init, state.cfg.trading_amount, state.trade_budget
    )
}

fn is_forced_pullback_open(decision: &TradeDecision) -> bool {
    matches!(
        &decision.entry,
        EntryGate::Open { notes }
            if notes.iter().any(|note| note == "forced-pullback")
                && matches!(decision.entry_reason, Some(EntryReason::SugInfoScalp { .. }))
    )
}

fn position_snapshot(input: &ModuleInput, state: &EngineState, side: PositionState) -> String {
    let position = active_position(input, side);
    let raw_qty = position.map(|position| position.qty).unwrap_or(0.0);
    let notional = position.map(|position| position.notional).unwrap_or(0.0);
    let pnl = position.map(|position| position.pnl).unwrap_or(0.0);
    let abs_qty = position_qty(input, side);
    format!(
        "pos={:?} raw_qty={:.8} abs_qty={:.8} notional={:.4} pnl={:.4} entry_price={:.4} tp={:.4} sl={:.4} exit_mode={} started_at_ms={}",
        side,
        raw_qty,
        abs_qty,
        notional,
        pnl,
        state.position.entry_price,
        state.position.take_profit,
        state.position.stop_loss,
        state.position.exit_mode.as_str(),
        match state.position.entry_reason.as_ref() {
            Some(reason) => reason.started_at_ms(),
            None => 0,
        }
    )
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
            if position.qty > 0.0 {
                position.qty
            } else if position.notional > 0.0 {
                (position.notional / input.price.max(f64::EPSILON)).max(0.0)
            } else {
                0.0
            }
        })
        .unwrap_or(0.0)
}

fn tick_size(input: &ModuleInput) -> f64 {
    input.symbol_info.price_tick_size.max(f64::EPSILON)
}

fn created_entry_timeout_reason(
    input: &ModuleInput,
    state: &EngineState,
    side: PositionState,
) -> Option<String> {
    let started_at_ms = state.position.entry_reason.as_ref()?.started_at_ms();
    let current_ms = input.sug_info.as_ref()?.system_time as u64;
    let age_ms = current_ms.saturating_sub(started_at_ms);
    if age_ms < CREATED_MAX_AGE_MS {
        return None;
    }
    let qty = position_qty(input, side);

    Some(format!(
        "created-timeout age_ms={} max_ms={} started_at_ms={} current_ms={} qty={:.8}",
        age_ms, CREATED_MAX_AGE_MS, started_at_ms, current_ms, qty
    ))
}

fn position_qty_is_zero(input: &ModuleInput, side: PositionState) -> bool {
    active_position(input, side)
        .map(|position| position.qty == 0.0)
        .unwrap_or(true)
}

fn direction_changed(position_side: PositionState, decision_state: MarketState) -> bool {
    matches!(
        (position_side, decision_state),
        (PositionState::Long, MarketState::Short) | (PositionState::Short, MarketState::Long)
    )
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
    state.position.entry_price = desired_price;
    Some(update_position(direction, Some(desired_price), None, None))
}

fn initialize_exit_plan(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    side: PositionState,
) -> Option<ModulePlaceOrder> {
    let direction = match side {
        PositionState::Long => Direction::Long,
        PositionState::Short => Direction::Short,
        PositionState::Flat => return None,
    };
    let reference_price = if state.position.entry_price > 0.0 {
        state.position.entry_price
    } else {
        input.price
    };
    let plan = if matches!(state.cfg.trade_mode, TradeMode::Martingale) {
        martingale::exit_plan(
            snapshot,
            side,
            reference_price,
            state.position.entry_reason.as_ref(),
        )?
    } else {
        exit::plan(
            snapshot,
            sug_snapshot,
            &state.cascade_sug,
            reference_price,
            side,
            state.position.entry_reason.as_ref(),
        )?
    };
    state.position.take_profit = plan.take_profit;
    state.position.stop_loss = plan.stop_loss;
    state.position.exit_tp_reliable = plan.tp_reliable;
    state.position.exit_mode = ExitMode::Structural;
    if state.position.tp_mark.is_empty() {
        state.position.tp_mark = tp_mark(direction);
    }
    let qty = position_qty(input, side);
    if qty <= 0.0 {
        return None;
    }
    let order_note = Some(format!("tp:{}", plan.source.as_str()));
    Some(tp_limit_order(
        direction,
        qty,
        plan.take_profit,
        &state.position.tp_mark,
        order_note.as_deref(),
    ))
}

fn ensure_active_exit_order(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    side: PositionState,
) -> (Vec<ModulePlaceOrder>, String) {
    if side == PositionState::Flat {
        return (Vec::new(), "exit=flat".to_string());
    }

    let direction = match side {
        PositionState::Long => Direction::Long,
        PositionState::Short => Direction::Short,
        PositionState::Flat => Direction::Both,
    };
    if state.position.tp_mark.is_empty() {
        state.position.tp_mark = tp_mark(direction);
    }

    let mut notes = Vec::new();
    let qty = position_qty(input, side);
    if qty <= 0.0 {
        return (
            Vec::new(),
            format!("exit={} qty=0", state.position.exit_mode.as_str()),
        );
    }

    if state.position.take_profit <= 0.0 || state.position.stop_loss <= 0.0 {
        if let Some(order) = initialize_exit_plan(input, state, snapshot, sug_snapshot, side) {
            notes.push("tp-init".to_string());
            notes.push(format!("tp={:.4}", state.position.take_profit));
            notes.push(format!("sl={:.4}", state.position.stop_loss));
            return (vec![order], notes.join("|"));
        }
        return (
            Vec::new(),
            format!("exit={} tp=n/a sl=n/a", state.position.exit_mode.as_str()),
        );
    }

    let tick = tick_size(input);
    let virtual_sl_hit =
        stop_loss::virtual_stop_loss_hit(side, input.price, state.position.stop_loss);

    if virtual_sl_hit && state.position.exit_mode != ExitMode::TrailAfterVirtualSl {
        state.position.exit_mode = ExitMode::TrailAfterVirtualSl;
        notes.push(format!("virtual-sl-hit={:.4}", state.position.stop_loss));
    }

    if matches!(state.position.exit_mode, ExitMode::TrailAfterVirtualSl) {
        let desired_tp = stop_loss::breakeven_price(state.position.entry_price, side, tick);
        if (desired_tp - state.position.take_profit).abs() > f64::EPSILON {
            state.position.take_profit = desired_tp;
            notes.push(format!("tp-reclaim={:.4}", desired_tp));
            let note = Some("tp:reclaim".to_string());
            let order = tp_limit_order(
                direction,
                qty,
                desired_tp,
                &state.position.tp_mark,
                note.as_deref(),
            );
            return (
                vec![order],
                format!(
                    "exit={} price={:.4} {}",
                    state.position.exit_mode.as_str(),
                    input.price,
                    notes.join("|")
                ),
            );
        }
        return (
            Vec::new(),
            format!(
                "exit={} price={:.4} tp={:.4} sl={:.4}",
                state.position.exit_mode.as_str(),
                input.price,
                state.position.take_profit,
                state.position.stop_loss
            ),
        );
    }

    if state.position.take_profit > 0.0 {
        return (
            Vec::new(),
            format!(
                "exit={} tp={:.4} sl={:.4}",
                state.position.exit_mode.as_str(),
                state.position.take_profit,
                state.position.stop_loss
            ),
        );
    }

    (
        Vec::new(),
        format!("exit={} tp=n/a sl=n/a", state.position.exit_mode.as_str()),
    )
}

#[cfg(test)]
pub fn evaluate(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    decision: &TradeDecision,
) -> ExecutionResult {
    evaluate_with_cascade(
        input,
        state,
        snapshot,
        sug_snapshot,
        decision,
        &CascadeSignal::default(),
    )
}

pub fn evaluate_with_cascade(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    decision: &TradeDecision,
    cascade: &CascadeSignal,
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
            cancel_orders: vec![tp_mark(Direction::Long), tp_mark(Direction::Short)],
            note: "flatten-both".to_string(),
            ..Default::default()
        };
    }

    if input.positions.long.is_some() {
        if direction_changed(PositionState::Long, decision.market_state) {
            return ExecutionResult {
                close_positions: vec![close(
                    Direction::Long,
                    &format!(
                        "{} {}",
                        close_reason("direction-changed", state, input.price),
                        position_snapshot(input, state, PositionState::Long)
                    ),
                )],
                note: format!(
                    "close-long direction-changed {}",
                    position_snapshot(input, state, PositionState::Long)
                ),
                ..Default::default()
            };
        }
        if position_qty_is_zero(input, PositionState::Long) {
            if let Some(reason) = created_entry_timeout_reason(input, state, PositionState::Long) {
                if let Some(update) = created_position_update(input, state, PositionState::Long) {
                    return ExecutionResult {
                        update_positions: vec![update],
                        note: format!(
                            "update-long {} {}",
                            reason,
                            position_snapshot(input, state, PositionState::Long)
                        ),
                        ..Default::default()
                    };
                }
                return ExecutionResult {
                    note: format!(
                        "hold-long {} {}",
                        reason,
                        position_snapshot(input, state, PositionState::Long)
                    ),
                    ..Default::default()
                };
            }
        }
        let (tp_orders, note) =
            ensure_active_exit_order(input, state, snapshot, sug_snapshot, PositionState::Long);
        return ExecutionResult {
            tp_orders,
            note: format!("hold-long {}", note),
            ..Default::default()
        };
    }

    if input.positions.short.is_some() {
        if direction_changed(PositionState::Short, decision.market_state) {
            return ExecutionResult {
                close_positions: vec![close(
                    Direction::Short,
                    &format!(
                        "{} {}",
                        close_reason("direction-changed", state, input.price),
                        position_snapshot(input, state, PositionState::Short)
                    ),
                )],
                note: format!(
                    "close-short direction-changed {}",
                    position_snapshot(input, state, PositionState::Short)
                ),
                ..Default::default()
            };
        }
        if position_qty_is_zero(input, PositionState::Short) {
            if let Some(reason) = created_entry_timeout_reason(input, state, PositionState::Short) {
                if let Some(update) = created_position_update(input, state, PositionState::Short) {
                    return ExecutionResult {
                        update_positions: vec![update],
                        note: format!(
                            "update-short {} {}",
                            reason,
                            position_snapshot(input, state, PositionState::Short)
                        ),
                        ..Default::default()
                    };
                }
                return ExecutionResult {
                    note: format!(
                        "hold-short {} {}",
                        reason,
                        position_snapshot(input, state, PositionState::Short)
                    ),
                    ..Default::default()
                };
            }
        }
        let (tp_orders, note) =
            ensure_active_exit_order(input, state, snapshot, sug_snapshot, PositionState::Short);
        return ExecutionResult {
            tp_orders,
            note: format!("hold-short {}", note),
            ..Default::default()
        };
    }

    let entry_side = match decision.market_state {
        MarketState::Long => PositionState::Long,
        MarketState::Short => PositionState::Short,
        MarketState::Sideways => PositionState::Flat,
    };

    let martingale_mode = matches!(state.cfg.trade_mode, TradeMode::Martingale);
    if !martingale_mode {
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

        if let EntryGate::Wait(wait) =
            prediction::entry_gate(snapshot, sug_snapshot, cascade, state, entry_side)
        {
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

        if decision.entry_reason.is_none() {
            return ExecutionResult {
                note: format!(
                    "entry-blocked reason=missing-entry-reason market_state={:?}",
                    decision.market_state
                ),
                ..Default::default()
            };
        }
    }

    let mut amount = entry_budget(input, state);
    if is_forced_pullback_open(decision) {
        amount *= state.cfg.pullback_ratio();
    }
    let leverage = (input.leverage as f64).max(1.0);
    if amount <= 0.0 {
        return ExecutionResult {
            note: "budget-too-small".to_string(),
            ..Default::default()
        };
    }
    if let Some(min_notional) = input.symbol_info.min_notional.filter(|value| *value > 0.0) {
        if amount * leverage < min_notional {
            return ExecutionResult {
                note: "budget-too-small".to_string(),
                ..Default::default()
            };
        }
    }

    state.position.entry_reason = decision.entry_reason.clone();
    if martingale_mode {
        if let Some(plan) = martingale::entry_plan(
            state,
            snapshot,
            decision.market_state,
            decision.entry_reason.as_ref(),
            amount,
        ) {
            state.position.take_profit = 0.0;
            state.position.stop_loss = plan.stop_loss;
            state.position.exit_mode = ExitMode::Structural;
            return ExecutionResult {
                open_positions: vec![plan.open_position],
                tp_orders: plan.orders,
                note: format!(
                    "open-{}-martingale immediate levels={} step={:.2}% base={:.4} {}",
                    match plan.market_state {
                        MarketState::Long => "long",
                        MarketState::Short => "short",
                        MarketState::Sideways => "flat",
                    },
                    plan.profile.levels,
                    plan.profile.step_pct,
                    plan.base_price,
                    plan.notes.join("|")
                ),
                ..Default::default()
            };
        }
        return ExecutionResult {
            note: "martingale-plan-unavailable".to_string(),
            ..Default::default()
        };
    }

    let entry_price = decision
        .entry_reason
        .as_ref()
        .map(|reason| reason.price())
        .unwrap_or(input.price);
    let entry_tag = match decision.entry_reason {
        Some(EntryReason::SugInfoScalp { .. }) => "sug-info",
        Some(EntryReason::SugInfoMovement { .. }) => "movement",
        Some(EntryReason::SidewayScalp { .. }) => "sideway",
        None => "default",
    };
    match decision.market_state {
        MarketState::Long => ExecutionResult {
            open_positions: vec![limit_open(
                Direction::Long,
                amount,
                entry_price,
                &open_note(Direction::Long, amount, entry_price, decision, state),
            )],
            note: format!("open-long-{}", entry_tag),
            ..Default::default()
        },
        MarketState::Short => ExecutionResult {
            open_positions: vec![limit_open(
                Direction::Short,
                amount,
                entry_price,
                &open_note(Direction::Short, amount, entry_price, decision, state),
            )],
            note: format!("open-short-{}", entry_tag),
            ..Default::default()
        },
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
            entry_reason: Some(crate::model::EntryReason::SugInfoScalp {
                price: 100.0,
                started_at_ms: 1_000,
            }),
        }
    }

    fn reasonless_decision(market_state: MarketState) -> TradeDecision {
        TradeDecision {
            market_state,
            entry: crate::model::EntryGate::Open { notes: vec![] },
            entry_reason: None,
        }
    }

    fn movement_decision(market_state: MarketState, price: f64) -> TradeDecision {
        TradeDecision {
            market_state,
            entry: crate::model::EntryGate::Open { notes: vec![] },
            entry_reason: Some(crate::model::EntryReason::SugInfoMovement {
                price,
                started_at_ms: 1_000,
            }),
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
        assert!(result.open_positions.is_empty());
    }

    #[test]
    fn sideways_blocks_entries() {
        let mut state = EngineState::default();
        let result = evaluate(
            &input(),
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &reasonless_decision(MarketState::Sideways),
        );
        assert!(result.open_positions.is_empty());
    }

    #[test]
    fn building_fast_candle_blocks_entries() {
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
        assert!(result.open_positions.is_empty());
        assert!(result.note.contains("fc-phase=Building"));
    }

    #[test]
    fn legacy_tiring_fast_candle_blocks_entries_2() {
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
        assert!(result.open_positions.is_empty());
        assert!(result.note.contains("fc-phase=Tiring"));
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
        assert_eq!(result.open_positions.len(), 1);
        assert_eq!(result.open_positions[0].amount, Some(1.0));
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
        assert_eq!(result.open_positions.len(), 1);
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
        assert_eq!(result.open_positions.len(), 1);
        assert!(result.note.contains("open-long"));
    }

    #[test]
    fn legacy_post_sl_wait_requires_reclaim() {
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
        assert!(result.open_positions.is_empty());
        assert!(result.note.contains("post-sl-recovery"));
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
        assert!(result.note.contains("post-sl-recovery"));
        assert!(result.note.contains("solutions=3"));
    }

    #[test]
    fn legacy_cooling_fast_candle_with_large_candle_waits() {
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
        assert!(result.open_positions.is_empty());
        assert!(result.note.contains("candle-not-small"));
    }
    #[test]
    fn legacy_tiring_fast_candle_blocks_entries() {
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(result.open_positions.len(), 1);
        assert_eq!(result.open_positions[0].amount, Some(1.0));
    }

    #[test]
    fn missing_entry_reason_blocks_entries() {
        let mut state = EngineState::default();
        let result = evaluate(
            &input(),
            &mut state,
            &IndicatorSnapshot::default(),
            &crate::model::SugInfoSnapshot::default(),
            &reasonless_decision(MarketState::Long),
        );
        assert!(result.open_positions.is_empty());
        assert!(result.note.contains("missing-entry-reason"));
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
        assert_eq!(result.open_positions.len(), 1);
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
        assert_eq!(result.open_positions.len(), 1);
        assert!(result.note.contains("open-long"));
    }

    #[test]
    fn post_sl_wait_requires_reclaim() {
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
        assert!(result.open_positions.is_empty());
        assert!(result.note.contains("post-sl-recovery"));
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
        assert!(result.note.contains("post-sl-recovery"));
        assert!(result.note.contains("solutions=3"));
    }

    #[test]
    fn cooling_fast_candle_with_large_candle_waits() {
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
            take_profit: 0.0,
            stop_loss: 0.0,
            exit_tp_reliable: true,
            exit_mode: ExitMode::Structural,
            tp_mark: "tp-long".to_string(),
            entry_reason: None,
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(result.tp_orders.len(), 1);
        assert_eq!(result.tp_orders[0].qty, Some(0.1));
        assert!(result.tp_orders[0].amount.is_none());
        assert!(result.close_positions.is_empty());
        assert!(result.note.contains("tp-init"));
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
            exit_mode: ExitMode::Structural,
            tp_mark: "tp-short".to_string(),
            entry_reason: None,
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
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Short),
        );
        assert_eq!(result.tp_orders.len(), 1);
        assert_eq!(result.tp_orders[0].qty, Some(0.1));
        assert!(result.tp_orders[0].amount.is_none());
        assert!(result.close_positions.is_empty());
        assert!(result.note.contains("trail-after-virtual-sl"));
    }

    #[test]
    fn trail_after_virtual_sl_keeps_fee_floor() {
        let mut state = EngineState::default();
        state.position = crate::state::PositionRuntime {
            side: crate::model::PositionState::Long,
            entry_price: 100.0,
            peak_price: 101.0,
            trough_price: 99.0,
            take_profit: 100.2,
            stop_loss: 99.5,
            exit_tp_reliable: true,
            exit_mode: ExitMode::TrailAfterVirtualSl,
            tp_mark: "tp-long".to_string(),
            entry_reason: None,
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

        assert_eq!(result.tp_orders.len(), 1);
        assert!(result.note.contains("tp-reclaim"));
    }

    #[test]
    fn created_long_timeout_updates_entry_price() {
        let mut state = EngineState::default();
        state.position = crate::state::PositionRuntime {
            side: crate::model::PositionState::Long,
            entry_price: 100.0,
            peak_price: 100.0,
            trough_price: 100.0,
            take_profit: 0.0,
            stop_loss: 0.0,
            exit_tp_reliable: false,
            exit_mode: ExitMode::Structural,
            tp_mark: "tp-long".to_string(),
            entry_reason: Some(crate::model::EntryReason::SugInfoMovement {
                price: 101.5,
                started_at_ms: 1_000,
            }),
        };
        let mut inp = input();
        inp.positions.long = Some(ModulePositionSummary {
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
        assert_eq!(result.update_positions.len(), 1);
        assert!(result.close_positions.is_empty());
        assert_eq!(result.update_positions[0].direction, Direction::Long);
        assert_eq!(result.update_positions[0].enter_price, Some(101.5));
        assert!(result.note.contains("created-timeout"));
        assert!(result.note.contains("update-long"));
    }

    #[test]
    fn created_short_timeout_closes_on_direction_change() {
        let mut state = EngineState::default();
        state.position = crate::state::PositionRuntime {
            side: crate::model::PositionState::Short,
            entry_price: 100.0,
            peak_price: 100.0,
            trough_price: 100.0,
            take_profit: 0.0,
            stop_loss: 0.0,
            exit_tp_reliable: false,
            exit_mode: ExitMode::Structural,
            tp_mark: "tp-short".to_string(),
            entry_reason: Some(crate::model::EntryReason::SugInfoMovement {
                price: 99.2,
                started_at_ms: 1_000,
            }),
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
        assert_eq!(result.close_positions.len(), 1);
        assert!(result.update_positions.is_empty());
        assert!(result.note.contains("direction-changed"));
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
            &crate::model::SugInfoSnapshot::default(),
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
            &crate::model::SugInfoSnapshot::default(),
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
        assert_eq!(result.open_positions.len(), 1);
        assert_eq!(result.open_positions[0].order_type, "Market");
        assert_eq!(result.open_positions[0].enter_price, Some(inp.price));
        assert_eq!(result.tp_orders.len(), 1);
        assert!(result.note.contains("immediate"));
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
        assert_eq!(result.open_positions.len(), 1);
        assert_eq!(result.open_positions[0].enter_price, Some(99.5));
        assert_eq!(result.note, "open-long-movement");
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
                entry: crate::model::EntryGate::Open { notes: vec![] },
                entry_reason: Some(crate::model::EntryReason::SugInfoScalp {
                    price: 99.5,
                    started_at_ms: 1_000,
                }),
            },
        );
        assert_eq!(result.open_positions.len(), 1);
        assert_eq!(result.open_positions[0].enter_price, Some(99.5));
        assert_eq!(result.note, "open-long-sug-info");
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
            },
            &crate::model::SugInfoSnapshot::default(),
            &decision(MarketState::Long),
        );
        assert_eq!(result.tp_orders.len(), 1);
        assert!(result.note.contains("tp-init"));
    }
}
