use crate::debug;
use crate::decision;
use crate::detection::{cascade, mrc, regime, sharp_return, vlines};
use crate::execution;
use crate::exit;
use crate::market_data;
use crate::model::{ExitKind, ExitMemory, ExitMode, MarketState, RiskSnapshot, TradeDecision};
use crate::orders::{limit_open, tp_limit_order, tp_mark};
use crate::risk;
use crate::state::{dir_to_position, sync_position_state, EngineState, PositionRuntime};
use lte_strategy_bridge::abi::{Direction, ModuleEvent, ModuleInput, ModuleOutput, OrderRole};

pub fn run(input: &ModuleInput, state: &mut EngineState) -> ModuleOutput {
    match &input.event {
        ModuleEvent::Init => {
            state.history.init(&input.indicators);
            process(input, state, true, &[60, 300, 900, 3600])
        }
        ModuleEvent::Indicators { timeframes } => {
            state.history.update(&input.indicators, timeframes);
            process(input, state, timeframes.contains(&60), timeframes)
        }
        ModuleEvent::SugInfo => process(input, state, false, &[]),
        ModuleEvent::Signal { code, values, .. } => {
            if code == "start" {
                state.trade_budget = 0.0;
                if let Some(cap_init) = values.get("cap_init").and_then(|v| v.as_f64()) {
                    state.cfg.cap_init = cap_init.clamp(0.0, 100.0);
                }
                if let Some(trading_amount) = values.get("trading_amount").and_then(|v| v.as_f64())
                {
                    state.cfg.trading_amount = trading_amount.clamp(10.0, 100.0);
                }
                state.cfg_loaded = true;
                ModuleOutput {
                    debug: format!(
                        "start cap_init={:.2}% trading_amount={:.2}%",
                        state.cfg.cap_init, state.cfg.trading_amount
                    ),
                    ..Default::default()
                }
            } else {
                ModuleOutput {
                    debug: format!("signal {} ignored", code),
                    ..Default::default()
                }
            }
        }
        ModuleEvent::Action { name, values } => {
            let amount = values.get("amount").and_then(|v| v.as_f64()).unwrap_or(5.0);
            let action_note = |side: &str| {
                format!(
                    "manual {} via {} price={:.4} amount={:.4}",
                    side, name, input.price, amount
                )
            };
            match name.as_str() {
                "force_open_long" | "force_open" => ModuleOutput {
                    open_positions: vec![limit_open(
                        Direction::Long,
                        amount,
                        input.price,
                        &action_note("open long"),
                    )],
                    debug: format!("action {} long {}", name, amount),
                    ..Default::default()
                },
                "force_open_short" => ModuleOutput {
                    open_positions: vec![limit_open(
                        Direction::Short,
                        amount,
                        input.price,
                        &action_note("open short"),
                    )],
                    debug: format!("action {} short {}", name, amount),
                    ..Default::default()
                },
                _ => ModuleOutput {
                    debug: format!("action {} ignored", name),
                    ..Default::default()
                },
            }
        }
        ModuleEvent::NewPosition {
            direction,
            entry_price,
            ..
        } => {
            state.position = PositionRuntime {
                side: dir_to_position(*direction),
                entry_price: *entry_price,
                peak_price: input.price.max(*entry_price),
                trough_price: input.price.min(*entry_price),
                take_profit: 0.0,
                stop_loss: 0.0,
                exit_tp_reliable: false,
                exit_mode: ExitMode::Structural,
                tp_mark: tp_mark(*direction),
            };
            let side = dir_to_position(*direction);
            let snapshot = market_data::build_snapshot(
                &state.history,
                &input.indicators,
                input.sug_info.as_ref(),
                input.price,
            );
            let plan = exit::plan(&snapshot, input.price, side);
            let mut place_orders = Vec::new();
            if let Some(plan) = plan {
                state.position.take_profit = plan.take_profit;
                state.position.stop_loss = plan.stop_loss;
                let amount = input
                    .positions
                    .long
                    .as_ref()
                    .map(|p| p.notional)
                    .or_else(|| input.positions.short.as_ref().map(|p| p.notional))
                    .unwrap_or(0.0);
                if amount > 0.0 {
                    place_orders.push(tp_limit_order(
                        *direction,
                        amount,
                        plan.take_profit,
                        &state.position.tp_mark,
                    ));
                }
            }
            ModuleOutput {
                place_orders,
                debug: format!(
                    "new-position {:?} {}",
                    direction,
                    plan.map(|plan| exit::describe(Some(plan)))
                        .unwrap_or_else(|| "tp=n/a sl=n/a".to_string())
                ),
                ..Default::default()
            }
        }
        ModuleEvent::FinishPosition { direction, pnl } => {
            risk::apply_close_pnl(input, state, *pnl);
            if state.position.side == dir_to_position(*direction) {
                state.position = PositionRuntime::default();
            }
            ModuleOutput {
                debug: format!("finish {:?} pnl={:.4}", direction, pnl),
                ..Default::default()
            }
        }
        ModuleEvent::OrderUpdate {
            direction,
            status,
            role,
            fill_price,
            mark,
            ..
        } => {
            if matches!(role, OrderRole::TakeProfit | OrderRole::StopLoss) {
                state.exit_memory = ExitMemory {
                    side: dir_to_position(*direction),
                    kind: Some(match role {
                        OrderRole::TakeProfit => ExitKind::TakeProfit,
                        OrderRole::StopLoss => ExitKind::StopLoss,
                        OrderRole::Entry | OrderRole::Other => ExitKind::TakeProfit,
                    }),
                    price: if *fill_price > 0.0 {
                        *fill_price
                    } else {
                        input.price
                    },
                    take_profit: state.position.take_profit,
                    stop_loss: state.position.stop_loss,
                    reliable: state.position.exit_tp_reliable,
                    tick: state.ticks,
                };
            }
            ModuleOutput {
                debug: format!("order {:?} {} {:?} {:?}", direction, status, role, mark),
                ..Default::default()
            }
        }
    }
}

fn process(
    input: &ModuleInput,
    state: &mut EngineState,
    update_risk: bool,
    timeframes: &[i64],
) -> ModuleOutput {
    state.ticks = state.ticks.saturating_add(1);
    if timeframes.contains(&60) {
        state.bar_1m = state.bar_1m.saturating_add(1);
    }

    sync_position_state(
        state,
        input
            .positions
            .long
            .as_ref()
            .map(|p| (p.entry_price, p.pnl)),
        input
            .positions
            .short
            .as_ref()
            .map(|p| (p.entry_price, p.pnl)),
        input.price,
    );

    let snapshot = market_data::build_snapshot(
        &state.history,
        &input.indicators,
        input.sug_info.as_ref(),
        input.price,
    );
    vlines::update(
        &mut state.vlines,
        snapshot.tf1m.mrc.lines(),
        input.price,
        state.ticks,
    );

    let cas1 = if timeframes.contains(&60) || matches!(input.event, ModuleEvent::Init) {
        cascade::update(&mut state.cascade_1m, &snapshot.tf1m)
    } else {
        cascade::CascadeSignal {
            dir: state.cascade_1m.dir,
            ..Default::default()
        }
    };
    let cas_sug = if let Some(sug) = input.sug_info.as_ref() {
        let candle = crate::model::Candle {
            open: sug.candle.enter,
            high: sug.candle.max,
            low: sug.candle.min,
            close: sug.candle.exit,
        };
        let dir_hint = if sug.direction.is_long() {
            1
        } else if sug.direction.is_short() {
            -1
        } else {
            0
        };
        cascade::update_from_candle(&mut state.cascade_sug, &candle, dir_hint)
    } else {
        cascade::CascadeSignal {
            dir: state.cascade_sug.dir,
            ..Default::default()
        }
    };
    let cas15 = if timeframes.contains(&900) || matches!(input.event, ModuleEvent::Init) {
        cascade::update(&mut state.cascade_15m, &snapshot.tf15m)
    } else {
        cascade::CascadeSignal {
            dir: state.cascade_15m.dir,
            ..Default::default()
        }
    };
    let cas1h = if timeframes.contains(&3600) || matches!(input.event, ModuleEvent::Init) {
        cascade::update(&mut state.cascade_1h, &snapshot.tf1h)
    } else {
        cascade::CascadeSignal {
            dir: state.cascade_1h.dir,
            ..Default::default()
        }
    };

    let risk = if update_risk {
        sharp_return::update(
            &mut state.sharp_return,
            snapshot.tf1m.candle,
            &snapshot.tf1m.mrc,
            state.bar_1m,
        )
    } else {
        RiskSnapshot {
            fail_streak: state.sharp_return.fail_streak,
            active: state.sharp_return.active,
        }
    };

    let regime = regime::classify_regime(
        &snapshot.tf5m,
        &snapshot.tf15m,
        &snapshot.tf1h,
        cas_sug,
        cas15,
        cas1h,
    );
    let mut decision = decision::decide(regime, risk);
    let mrc1 = mrc::analyze(&snapshot.tf1m.mrc, input.price);
    if decision.market_state == MarketState::Sideways && mrc1.bullish_structure && cas1.dir > 0 {
        decision = TradeDecision {
            market_state: MarketState::Long,
            entry: match decision.entry {
                crate::model::EntryGate::Open { mut notes } => {
                    notes.push("1m-recovery".to_string());
                    crate::model::EntryGate::Open { notes }
                }
                crate::model::EntryGate::Wait(mut wait) => {
                    wait.notes.push("1m-recovery".to_string());
                    crate::model::EntryGate::Wait(wait)
                }
            },
        };
    } else if decision.market_state == MarketState::Sideways
        && mrc1.bearish_structure
        && cas1.dir < 0
    {
        decision = TradeDecision {
            market_state: MarketState::Short,
            entry: match decision.entry {
                crate::model::EntryGate::Open { mut notes } => {
                    notes.push("1m-breakdown".to_string());
                    crate::model::EntryGate::Open { notes }
                }
                crate::model::EntryGate::Wait(mut wait) => {
                    wait.notes.push("1m-breakdown".to_string());
                    crate::model::EntryGate::Wait(wait)
                }
            },
        };
    }

    let execution = execution::evaluate(input, state, &snapshot, &decision);

    ModuleOutput {
        open_positions: execution.open_positions,
        close_positions: execution.close_positions,
        place_orders: execution.place_orders,
        cancel_orders: execution.cancel_orders,
        debug: debug::render(
            state,
            &snapshot,
            &regime,
            &cas1,
            &cas_sug,
            &risk,
            &decision,
            &execution.note,
        ),
        ..Default::default()
    }
}
