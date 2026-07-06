use crate::exit;
use crate::model::IndicatorSnapshot;
use crate::model::{EntryGate, ExitMode, MarketState, PositionState, TradeDecision};
use crate::orders::{close, limit_open, tp_limit_order, tp_mark};
use crate::prediction;
use crate::risk::entry_budget;
use crate::state::{sync_position_state, EngineState};
use lte_strategy_bridge::abi::{
    Direction, ModuleClosePosition, ModuleInput, ModuleOpenPosition, ModulePlaceOrder,
    ModulePositionSummary,
};

#[cfg(test)]
use crate::model::ExitKind;

#[derive(Debug, Default)]
pub struct ExecutionResult {
    pub open_positions: Vec<ModuleOpenPosition>,
    pub close_positions: Vec<ModuleClosePosition>,
    pub place_orders: Vec<ModulePlaceOrder>,
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
        reason, price, state.cfg.cap_init, state.cfg.trading_amount, state.trade_budget
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

fn position_notional(input: &ModuleInput, side: PositionState) -> f64 {
    active_position(input, side)
        .map(|position| {
            if position.notional > 0.0 {
                position.notional
            } else {
                (position.qty.max(0.0)) * input.price.max(0.0)
            }
        })
        .unwrap_or(0.0)
}

fn tick_size(input: &ModuleInput) -> f64 {
    input.symbol_info.price_tick_size.max(f64::EPSILON)
}

fn trailing_tp_price(side: PositionState, anchor: f64, tick_size: f64) -> f64 {
    match side {
        PositionState::Long => (anchor - tick_size * 2.0).max(tick_size),
        PositionState::Short => anchor + tick_size * 2.0,
        PositionState::Flat => anchor,
    }
}

fn initialize_exit_plan(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    side: PositionState,
) -> Option<ModulePlaceOrder> {
    let direction = match side {
        PositionState::Long => Direction::Long,
        PositionState::Short => Direction::Short,
        PositionState::Flat => return None,
    };
    let plan = exit::plan(snapshot, input.price, side)?;
    state.position.take_profit = plan.take_profit;
    state.position.stop_loss = plan.stop_loss;
    state.position.exit_tp_reliable = plan.tp_reliable;
    state.position.exit_mode = ExitMode::Structural;
    if state.position.tp_mark.is_empty() {
        state.position.tp_mark = tp_mark(direction);
    }
    let amount = position_notional(input, side);
    if amount <= 0.0 {
        return None;
    }
    Some(tp_limit_order(
        direction,
        amount,
        plan.take_profit,
        &state.position.tp_mark,
    ))
}

fn ensure_active_exit_order(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
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
    let amount = position_notional(input, side);
    if amount <= 0.0 {
        return (
            Vec::new(),
            format!("exit={} amount=0", state.position.exit_mode.as_str()),
        );
    }

    if state.position.take_profit <= 0.0 || state.position.stop_loss <= 0.0 {
        if let Some(order) = initialize_exit_plan(input, state, snapshot, side) {
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
    let virtual_sl_hit = match side {
        PositionState::Long => input.price <= state.position.stop_loss,
        PositionState::Short => input.price >= state.position.stop_loss,
        PositionState::Flat => false,
    };

    if virtual_sl_hit && state.position.exit_mode != ExitMode::TrailAfterVirtualSl {
        state.position.exit_mode = ExitMode::TrailAfterVirtualSl;
        state.position.trail_anchor = input.price;
        notes.push(format!("virtual-sl-hit={:.4}", state.position.stop_loss));
    }

    if matches!(state.position.exit_mode, ExitMode::TrailAfterVirtualSl) {
        if state.position.trail_anchor <= 0.0 {
            state.position.trail_anchor = input.price;
        } else {
            match side {
                PositionState::Long => {
                    state.position.trail_anchor = state.position.trail_anchor.max(input.price);
                }
                PositionState::Short => {
                    state.position.trail_anchor = state.position.trail_anchor.min(input.price);
                }
                PositionState::Flat => {}
            }
        }
        let desired_tp = trailing_tp_price(side, state.position.trail_anchor, tick);
        if (desired_tp - state.position.take_profit).abs() > f64::EPSILON {
            state.position.take_profit = desired_tp;
            notes.push(format!("tp-trail={:.4}", desired_tp));
            let order = tp_limit_order(direction, amount, desired_tp, &state.position.tp_mark);
            return (
                vec![order],
                format!(
                    "exit={} anchor={:.4} {}",
                    state.position.exit_mode.as_str(),
                    state.position.trail_anchor,
                    notes.join("|")
                ),
            );
        }
        return (
            Vec::new(),
            format!(
                "exit={} anchor={:.4} tp={:.4} sl={:.4}",
                state.position.exit_mode.as_str(),
                state.position.trail_anchor,
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
            cancel_orders: vec![tp_mark(Direction::Long), tp_mark(Direction::Short)],
            note: "flatten-both".to_string(),
            ..Default::default()
        };
    }

    if input.positions.long.is_some() {
        let (place_orders, note) =
            ensure_active_exit_order(input, state, snapshot, PositionState::Long);
        return ExecutionResult {
            place_orders,
            note: format!("hold-long {}", note),
            ..Default::default()
        };
    }

    if input.positions.short.is_some() {
        let (place_orders, note) =
            ensure_active_exit_order(input, state, snapshot, PositionState::Short);
        return ExecutionResult {
            place_orders,
            note: format!("hold-short {}", note),
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

    match decision.market_state {
        MarketState::Long => ExecutionResult {
            open_positions: vec![limit_open(
                Direction::Long,
                amount,
                input.price,
                &open_note(Direction::Long, amount, input.price, decision, state),
            )],
            note: "open-long".to_string(),
            ..Default::default()
        },
        MarketState::Short => ExecutionResult {
            open_positions: vec![limit_open(
                Direction::Short,
                amount,
                input.price,
                &open_note(Direction::Short, amount, input.price, decision, state),
            )],
            note: "open-short".to_string(),
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
        assert_eq!(result.open_positions[0].amount, Some(1.0));
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
            take_profit: 0.0,
            stop_loss: 0.0,
            exit_tp_reliable: true,
            exit_mode: ExitMode::Structural,
            tp_mark: "tp-long".to_string(),
            trail_anchor: 0.0,
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
        let result = evaluate(&inp, &mut state, &snapshot, &decision(MarketState::Long));
        assert_eq!(result.place_orders.len(), 1);
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
            trail_anchor: 0.0,
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
        let result = evaluate(&inp, &mut state, &snapshot, &decision(MarketState::Short));
        assert_eq!(result.place_orders.len(), 1);
        assert!(result.close_positions.is_empty());
        assert!(result.note.contains("trail-after-virtual-sl"));
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
                sug_candle: None,
                sug_direction: 0,
                signal: Default::default(),
            },
            &decision(MarketState::Long),
        );
        assert_eq!(result.place_orders.len(), 1);
        assert!(result.note.contains("tp-init"));
    }
}
