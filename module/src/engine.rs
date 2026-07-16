use crate::builder;
use crate::debug;
use crate::decision;
use crate::detection::{cascade, direction_lock, regime, sideways, vlines};
use crate::execution;
use crate::exit;
use crate::market_data;
use crate::martingale;
use crate::model::{ExitKind, ExitMemory, ExitMode, TradeMode};
use crate::orders::{tp_limit_order, tp_mark};
use crate::risk;
use crate::state::{dir_to_position, sync_position_state, EngineState, PositionRuntime};
use lte_strategy_bridge::abi::{Direction, ModuleEvent, ModuleInput, ModuleOutput, OrderRole};
use lte_strategy_bridge::indicator_access::TimeframeSec;
use lte_strategy_bridge::sug_info::SuggestionInfo;

const TF_1M: i64 = TimeframeSec::Tf1m.as_i64();
const TF_5M: i64 = TimeframeSec::Tf5m.as_i64();
const TF_15M: i64 = TimeframeSec::Tf15m.as_i64();
const TF_1H: i64 = TimeframeSec::Tf1h.as_i64();

pub fn run(input: &ModuleInput, state: &mut EngineState) -> ModuleOutput {
    match &input.event {
        ModuleEvent::Init => {
            state.history.init(&input.indicators);
            refresh_indicators(input, state, &[TF_1M, TF_5M, TF_15M, TF_1H], true)
        }
        ModuleEvent::Indicators { timeframes } => {
            state.history.update(&input.indicators, timeframes);
            refresh_indicators(input, state, timeframes, false)
        }
        ModuleEvent::SugInfo => match input.sug_info.as_ref() {
            Some(sug_info) => process_realtime(input, state, sug_info),
            None => ModuleOutput {
                debug: "sug-info missing".to_string(),
                ..Default::default()
            },
        },
        ModuleEvent::Signal { code, values, .. } => handle_signal(code, values, state),
        ModuleEvent::Action { name, values } => handle_action(input, name, values),
        ModuleEvent::NewPosition {
            direction,
            entry_price,
            ..
        } => handle_new_position(input, state, *direction, *entry_price),
        ModuleEvent::FinishPosition { direction, pnl } => {
            handle_finish_position(input, state, *direction, *pnl)
        }
        ModuleEvent::OrderUpdate {
            direction,
            status,
            role,
            fill_price,
            mark,
            ..
        } => handle_order_update(
            input,
            state,
            *direction,
            status,
            role.clone(),
            *fill_price,
            mark,
        ),
    }
}

fn handle_signal(
    code: &str,
    values: &std::collections::HashMap<String, serde_json::Value>,
    state: &mut EngineState,
) -> ModuleOutput {
    if code == "start" {
        state.trade_budget = 0.0;
        state.direction_lock = Default::default();
        state.movement_entry_pending = None;
        if let Some(cap_init) = values.get("cap_init").and_then(|v| v.as_f64()) {
            state.cfg.cap_init = cap_init.clamp(0.0, 100.0);
        }
        if let Some(trading_amount) = values.get("trading_amount").and_then(|v| v.as_f64()) {
            state.cfg.trading_amount = trading_amount.clamp(10.0, 100.0);
        }
        if let Some(pullback_amount) = values.get("pullback_amount").and_then(|v| v.as_f64()) {
            state.cfg.pullback_amount = pullback_amount.clamp(0.0, 100.0);
        }
        if let Some(trade_mode) = values.get("trade_mode").and_then(|v| v.as_f64()) {
            state.cfg.trade_mode = if trade_mode >= 1.0 {
                TradeMode::Martingale
            } else {
                TradeMode::Single
            };
        }
        state.strategy_builder = Some(builder::InitialBuilder::new(state.cfg.trade_mode).build());
        state.cfg_loaded = true;
        ModuleOutput {
            debug: format!(
                "start cap_init={:.2}% trading_amount={:.2}% pullback_amount={:.2}% trade_mode={}",
                state.cfg.cap_init,
                state.cfg.trading_amount,
                state.cfg.pullback_amount,
                state.cfg.trade_mode.as_str()
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

fn handle_action(
    _input: &ModuleInput,
    name: &str,
    _values: &std::collections::HashMap<String, serde_json::Value>,
) -> ModuleOutput {
    ModuleOutput {
        debug: format!("action {} ignored", name),
        ..Default::default()
    }
}

fn handle_new_position(
    input: &ModuleInput,
    state: &mut EngineState,
    direction: Direction,
    entry_price: f64,
) -> ModuleOutput {
    state.position = PositionRuntime {
        side: dir_to_position(direction),
        entry_price,
        peak_price: input.price.max(entry_price),
        trough_price: input.price.min(entry_price),
        take_profit: 0.0,
        stop_loss: 0.0,
        exit_tp_reliable: false,
        exit_mode: ExitMode::Structural,
        tp_mark: tp_mark(direction),
        entry_reason: state.position.entry_reason.clone(),
        martingale_profile: state.position.martingale_profile,
    };

    let side = dir_to_position(direction);
    state.strategy_builder = Some(builder::InitialBuilder::new(state.cfg.trade_mode).build());
    let strategy = builder::ensure_runtime_builder(state);
    let snapshot =
        market_data::build_indicator_snapshot(&state.history, &input.indicators, input.price);
    direction_lock::refresh(
        &mut state.direction_lock,
        &snapshot.tf1m,
        input.price,
        state.ticks,
    );
    let plan = if strategy.is_martingale() {
        martingale::exit_plan(
            &snapshot,
            side,
            entry_price,
            state.position.entry_reason.as_ref(),
            state.position.martingale_profile,
        )
    } else {
        input.sug_info.as_ref().and_then(|sug_info| {
            let sug_snapshot = market_data::build_sug_info_snapshot(sug_info);
            exit::plan(
                &snapshot,
                &sug_snapshot,
                &state.cascade_sug,
                entry_price,
                side,
                state.position.entry_reason.as_ref(),
            )
        })
    };
    let mut tp_orders = Vec::new();
    if let Some(plan) = plan {
        state.position.take_profit = plan.take_profit;
        state.position.stop_loss = plan.stop_loss;
        let qty = input
            .positions
            .long
            .as_ref()
            .map(|p| p.qty)
            .or_else(|| input.positions.short.as_ref().map(|p| p.qty))
            .unwrap_or(0.0);
        if qty > 0.0 {
            let note = Some(format!("tp:{}", plan.source.as_str()));
            tp_orders.push(tp_limit_order(
                direction,
                qty,
                plan.take_profit,
                &state.position.tp_mark,
                note.as_deref(),
            ));
        }
    }

    ModuleOutput {
        place_orders: tp_orders,
        debug: format!(
            "new-position {:?} {}",
            direction,
            plan.map(|plan| exit::describe(Some(plan)))
                .unwrap_or_else(|| "tp=n/a sl=n/a".to_string())
        ),
        ..Default::default()
    }
}

fn handle_finish_position(
    input: &ModuleInput,
    state: &mut EngineState,
    direction: Direction,
    pnl: f64,
) -> ModuleOutput {
    risk::apply_close_pnl(input, state, pnl);
    if state.position.side == dir_to_position(direction) {
        state.position = PositionRuntime::default();
    }
    ModuleOutput {
        debug: format!("finish {:?} pnl={:.4}", direction, pnl),
        ..Default::default()
    }
}

fn handle_order_update(
    input: &ModuleInput,
    state: &mut EngineState,
    direction: Direction,
    status: &str,
    role: OrderRole,
    fill_price: f64,
    mark: &Option<String>,
) -> ModuleOutput {
    if matches!(role, OrderRole::TakeProfit | OrderRole::StopLoss) {
        state.exit_memory = ExitMemory {
            side: dir_to_position(direction),
            kind: Some(match role {
                OrderRole::TakeProfit => ExitKind::TakeProfit,
                OrderRole::StopLoss => ExitKind::StopLoss,
                OrderRole::Entry | OrderRole::Other => ExitKind::TakeProfit,
            }),
            price: if fill_price > 0.0 {
                fill_price
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

fn refresh_indicators(
    input: &ModuleInput,
    state: &mut EngineState,
    timeframes: &[i64],
    is_init: bool,
) -> ModuleOutput {
    state.ticks = state.ticks.saturating_add(1);
    if timeframes.contains(&TF_1M) {
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

    let snapshot =
        market_data::build_indicator_snapshot(&state.history, &input.indicators, input.price);
    direction_lock::refresh(
        &mut state.direction_lock,
        &snapshot.tf1m,
        input.price,
        state.ticks,
    );
    if is_init || timeframes.contains(&TF_1M) {
        cascade::update(&mut state.cascade_tf, &snapshot.tf1m);
    }
    vlines::update(
        &mut state.vlines,
        snapshot.tf1m.mrc.lines(),
        input.price,
        state.ticks,
    );

    ModuleOutput {
        debug: String::new(),
        ..Default::default()
    }
}

fn process_realtime(
    input: &ModuleInput,
    state: &mut EngineState,
    sug_info: &SuggestionInfo,
) -> ModuleOutput {
    state.ticks = state.ticks.saturating_add(1);
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

    let snapshot =
        market_data::build_indicator_snapshot(&state.history, &input.indicators, input.price);
    let strategy = builder::ensure_runtime_builder(state);
    direction_lock::refresh(
        &mut state.direction_lock,
        &snapshot.tf1m,
        input.price,
        state.ticks,
    );
    let sug_snapshot = market_data::build_sug_info_snapshot(sug_info);
    let candle = crate::model::Candle {
        open: sug_info.candle.enter,
        high: sug_info.candle.max,
        low: sug_info.candle.min,
        close: sug_info.candle.exit,
    };
    let cas_tf = cascade::preview_from_candle(&state.cascade_tf, &candle);
    let cas_sug = cascade::update_from_candle(&mut state.cascade_sug, &candle);
    let sideway = sideways::analyze(state, &snapshot.tf1m);

    let regime = regime::classify_regime(&snapshot.tf5m, &snapshot.tf15m, &snapshot.tf1h);
    let decision = strategy.refine_realtime_decision(
        state,
        &snapshot,
        &sug_snapshot,
        &sideway,
        regime,
        &cas_tf,
        &cas_sug,
        sug_info,
        decision::decide(regime),
    );

    let execution = execution::evaluate_with_cascade(
        input,
        state,
        &snapshot,
        &sug_snapshot,
        &decision,
        &cas_sug,
    );

    ModuleOutput {
        open_positions: execution.open_positions,
        close_positions: execution.close_positions,
        update_positions: execution.update_positions,
        place_orders: execution.tp_orders,
        cancel_orders: execution.cancel_orders,
        debug: debug::render(
            state,
            &snapshot,
            &regime,
            &cas_tf,
            &cas_sug,
            &decision,
            &execution.note,
        ),
        ..Default::default()
    }
}
