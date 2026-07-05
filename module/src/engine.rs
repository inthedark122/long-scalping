use crate::debug;
use crate::decision;
use crate::detection::{cascade, mrc, regime, sharp_return, vlines};
use crate::exit;
use crate::execution;
use crate::market_data;
use crate::model::{MarketState, RiskSnapshot, TradeDecision};
use crate::orders::limit_open;
use crate::risk;
use crate::state::{dir_to_position, sync_position_state, EngineState, PositionRuntime};
use lte_strategy_bridge::abi::{Direction, ModuleEvent, ModuleInput, ModuleOutput};

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
            };
            let side = dir_to_position(*direction);
            let plan = exit::plan(
                &market_data::build_snapshot(
                    &state.history,
                    &input.indicators,
                    input.sug_info.as_ref(),
                    input.price,
                ),
                input.price,
                side,
            );
            if let Some(plan) = plan {
                state.position.take_profit = plan.take_profit;
                state.position.stop_loss = plan.stop_loss;
            }
            ModuleOutput {
                update_positions: plan
                    .map(|plan| vec![exit::update_position(side, plan)])
                    .unwrap_or_default(),
                debug: format!("new-position {:?} {}", direction, exit::describe(plan)),
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
            mark,
            ..
        } => ModuleOutput {
            debug: format!("order {:?} {} {:?}", direction, status, mark),
            ..Default::default()
        },
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
            strength: 0.0,
            stretched: false,
        }
    };
    let cas5 = if timeframes.contains(&300) || matches!(input.event, ModuleEvent::Init) {
        cascade::update(&mut state.cascade_5m, &snapshot.tf5m)
    } else {
        cascade::CascadeSignal {
            dir: state.cascade_5m.dir,
            strength: 0.0,
            stretched: false,
        }
    };
    let cas15 = if timeframes.contains(&900) || matches!(input.event, ModuleEvent::Init) {
        cascade::update(&mut state.cascade_15m, &snapshot.tf15m)
    } else {
        cascade::CascadeSignal {
            dir: state.cascade_15m.dir,
            strength: 0.0,
            stretched: false,
        }
    };
    let cas1h = if timeframes.contains(&3600) || matches!(input.event, ModuleEvent::Init) {
        cascade::update(&mut state.cascade_1h, &snapshot.tf1h)
    } else {
        cascade::CascadeSignal {
            dir: state.cascade_1h.dir,
            strength: 0.0,
            stretched: false,
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
            risk_high: state.sharp_return.fail_streak >= 2,
            fail_streak: state.sharp_return.fail_streak,
            active: state.sharp_return.active,
        }
    };

    let regime = regime::classify_regime(
        &snapshot.tf5m,
        &snapshot.tf15m,
        &snapshot.tf1h,
        cas5,
        cas15,
        cas1h,
    );
    let mut decision = decision::decide(regime, risk);
    let mrc1 = mrc::analyze(&snapshot.tf1m.mrc, input.price);
    if decision.market_state == MarketState::Sideways && mrc1.bullish_structure && cas1.dir > 0 {
        decision = TradeDecision {
            market_state: MarketState::Long,
            allow_entry: decision.allow_entry,
            reason: format!("{}|1m-recovery", decision.reason),
        };
    } else if decision.market_state == MarketState::Sideways
        && mrc1.bearish_structure
        && cas1.dir < 0
    {
        decision = TradeDecision {
            market_state: MarketState::Short,
            allow_entry: decision.allow_entry,
            reason: format!("{}|1m-breakdown", decision.reason),
        };
    }

    let execution = execution::evaluate(input, state, &snapshot, &decision);

    ModuleOutput {
        open_positions: execution.open_positions,
        close_positions: execution.close_positions,
        update_positions: execution.update_positions,
        debug: debug::render(state, &snapshot, &regime, &risk, &decision, &execution.note),
        ..Default::default()
    }
}
