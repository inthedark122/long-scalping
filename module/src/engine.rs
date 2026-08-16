use crate::builder;
use crate::debug;
use crate::detection::{cascade, mrc_crossing, regime, sideways, vlines};
use crate::execution;
use crate::exit;
use crate::guard;
use crate::market_data;
use crate::model::{exit_type_for_strategy, EntryType, ExitMode, StrategyMode};
use crate::order_manager;
use crate::orders::{tp_limit_order, tp_mark};
use crate::prediction;
use crate::risk;
use crate::state::{dir_to_position, sync_position_state, EngineState, PositionRuntime};
use lte_strategy_bridge::abi::{
    Direction, ModuleEvent, ModuleInput, ModuleOrderStatus, ModuleOutput, OrderRole,
};
use lte_strategy_bridge::indicator_access::TimeframeSec;
use lte_strategy_bridge::sug_info::SuggestionInfo;

const TF_1M: i64 = TimeframeSec::Tf1m.as_i64();
const TF_5M: i64 = TimeframeSec::Tf5m.as_i64();
const TF_15M: i64 = TimeframeSec::Tf15m.as_i64();
const TF_1H: i64 = TimeframeSec::Tf1h.as_i64();

fn snapshot_error(
    event: &str,
    context: &str,
    raw: &market_data::RawIndicators,
    error: String,
) -> String {
    if context.is_empty() {
        format!(
            "event={} {} {}",
            event,
            error,
            market_data::render_raw_indicator_inventory(raw)
        )
    } else {
        format!(
            "event={} {} {} {}",
            event,
            context,
            error,
            market_data::render_raw_indicator_inventory(raw)
        )
    }
}

fn timeframes_context(timeframes: &[i64]) -> String {
    if timeframes.is_empty() {
        String::new()
    } else {
        format!(
            "tfs=[{}]",
            timeframes
                .iter()
                .map(|tf| tf.to_string())
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

fn refresh_last_snapshot(state: &mut EngineState, snapshot: &crate::model::IndicatorSnapshot) {
    state.last_snapshot = Some(snapshot.clone());
}

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
        ModuleEvent::Signal {
            direction,
            code,
            values,
        } => handle_signal(direction, code, values, state),
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
            filled_qty,
            mark,
            ..
        } => handle_order_update(
            input,
            state,
            *direction,
            *status,
            role.clone(),
            *fill_price,
            *filled_qty,
            mark,
        ),
    }
}

fn handle_signal(
    direction: &Direction,
    code: &str,
    values: &std::collections::HashMap<String, serde_json::Value>,
    state: &mut EngineState,
) -> ModuleOutput {
    if code == "start" {
        state.allowed_direction = *direction;
        state.trade_budget = 0.0;
        state.movement_entry_pending = None;
        state.mrc_crossing_model.pending_entry = None;
        state.pending_entry_bar = None;
        if let Some(cap_init) = values.get("cap_init").and_then(|v| v.as_f64()) {
            state.cfg.cap_init = cap_init.clamp(0.0, 100.0);
        }
        if let Some(trading_amount) = values.get("trading_amount").and_then(|v| v.as_f64()) {
            state.cfg.trading_amount = trading_amount.clamp(10.0, 100.0);
        }
        if let Some(min_tp_percent) = values.get("min_tp_percent").and_then(|v| v.as_f64()) {
            state.cfg.min_tp_percent = min_tp_percent.max(0.0);
        }
        if let Some(min_sl_percent) = values.get("min_sl_percent").and_then(|v| v.as_f64()) {
            state.cfg.min_sl_percent = min_sl_percent.max(0.0);
        }
        if let Some(risk_target_percent) =
            values.get("risk_target_percent").and_then(|v| v.as_f64())
        {
            state.cfg.risk_target_percent = risk_target_percent.max(0.0);
        }
        if let Some(strategy_mode) = values.get("strategy_mode").and_then(|v| v.as_f64()) {
            state.cfg.strategy_mode = if strategy_mode >= 1.0 {
                StrategyMode::MrcCrossing
            } else {
                StrategyMode::Stairs
            };
        }
        if let Some(entry_type) = values
            .get("entry_type")
            .and_then(|value| value.as_f64())
            .filter(|value| value.is_finite())
        {
            state.cfg.entry_type = if entry_type >= 0.5 {
                EntryType::Limit
            } else {
                EntryType::Market
            };
        }
        if let Some(required_samples) = values
            .get("required_samples")
            .and_then(|value| value.as_f64())
            .filter(|value| value.is_finite())
        {
            state.cfg.required_samples =
                required_samples.round().clamp(0.0, u32::MAX as f64) as u32;
        }
        if state.cfg.strategy_mode == StrategyMode::MrcCrossing {
            state.mrc_crossing_model = crate::state::MrcCrossingModel::default();
            state.vlines = [crate::state::VLineState::default(); 9];
            state.mrc_crossing = crate::state::MrcCrossingState::default();
            mrc_crossing::seed_from_history(
                &mut state.mrc_crossing_model,
                &mut state.vlines,
                &mut state.mrc_crossing,
                &state.history,
                state.ticks.max(1).saturating_mul(60_000),
                state.cfg.required_samples(),
            );
            state.mrc_crossing_model.pending_entry = None;
        }
        state.strategy_builder = Some(builder::InitialBuilder::new().build());
        state.cfg_loaded = true;
        ModuleOutput {
            debug: format!(
                "start direction={:?} strategy={} entry={} cap_init={:.2}% trading_amount={:.2}% min_tp_percent={:.2}% min_sl_percent={:.2}% risk_target_percent={:.2}% required_samples={} exit={:?}",
                state.allowed_direction,
                state.cfg.strategy_mode.as_str(),
                state.cfg.entry_type.as_str(),
                state.cfg.cap_init,
                state.cfg.trading_amount,
                state.cfg.min_tp_percent(),
                state.cfg.min_sl_percent(),
                state.cfg.risk_target_percent(),
                state.cfg.required_samples(),
                exit_type_for_strategy(state.cfg.strategy_mode)
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
    state.structural_stop_loss = None;
    state.processed_structural_stop_losses.clear();
    state.position = PositionRuntime {
        side: dir_to_position(direction),
        entry_price,
        peak_price: input.price.max(entry_price),
        trough_price: input.price.min(entry_price),
        take_profit: 0.0,
        stop_loss: 0.0,
        stop_loss_reason: String::new(),
        exit_tp_reliable: false,
        exit_mode: ExitMode::Structural,
        exit_type: exit_type_for_strategy(state.cfg.strategy_mode),
        entry_reason: state.position.entry_reason.clone(),
    };
    state.mrc_crossing_model.pending_entry = None;
    state.pending_entry_bar = None;

    let side = dir_to_position(direction);
    state.strategy_builder = Some(builder::InitialBuilder::new().build());
    builder::ensure_runtime_builder(state);
    let snapshot = match market_data::build_indicator_snapshot(&state.history, &input.indicators) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let error = snapshot_error("Init", "tfs=[60,300,900,3600]", &input.indicators, error);
            state.last_error = Some(error.clone());
            return ModuleOutput {
                debug: error,
                ..Default::default()
            };
        }
    };
    let plan = if let Some(crate::model::EntryReason::MrcCrossing {
        target_price,
        stop_price,
        ..
    }) = state.position.entry_reason.as_ref()
    {
        exit::mrc_crossing_plan(*target_price, *stop_price).and_then(|plan| {
            guard::minimum_exit_distances::enforce_minimum_exit_distances(
                plan,
                entry_price,
                side,
                state.cfg.min_tp_percent(),
                state.cfg.min_sl_percent(),
            )
        })
    } else {
        input.sug_info.as_ref().and_then(|sug_info| {
            let sug_snapshot = market_data::build_sug_info_snapshot(sug_info);
            exit::build_exit_plan(
                &snapshot,
                &sug_snapshot,
                &state.cascade_tf,
                entry_price,
                side,
                exit::ExitPlanContext::Entry,
                state.cfg.required_samples(),
                &[],
                0.0,
            )
        })
    };
    let mut tp_orders = Vec::new();
    let order_mark = tp_mark(direction);
    if let Some(plan) = plan {
        state.position.take_profit = plan.take_profit.price();
        exit::set_stop_loss(state, plan.stop_loss);
        let qty = input
            .positions
            .long
            .as_ref()
            .map(|p| p.qty)
            .or_else(|| input.positions.short.as_ref().map(|p| p.qty))
            .unwrap_or(0.0);
        if qty > 0.0 {
            let note = Some(plan.take_profit.order_note());
            tp_orders.push(tp_limit_order(
                direction,
                qty,
                plan.take_profit.price(),
                &order_mark,
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
        state.structural_stop_loss = None;
        state.processed_structural_stop_losses.clear();
        state.position = PositionRuntime::default();
        state.pending_entry_bar = None;
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
    status: ModuleOrderStatus,
    role: OrderRole,
    fill_price: f64,
    filled_qty: f64,
    mark: &Option<String>,
) -> ModuleOutput {
    order_manager::handle_order_update(
        input, state, direction, status, role, fill_price, filled_qty, mark,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lte_strategy_bridge::abi::{ModuleEvent, ModuleInput, ModulePositions, ModuleSymbolInfo};
    use std::collections::BTreeMap;

    fn input(event: ModuleEvent) -> ModuleInput {
        ModuleInput {
            event,
            price: 100.0,
            symbol: "TEST".to_string(),
            max_amount: 100.0,
            symbol_info: ModuleSymbolInfo::default(),
            auto_max_amount: 100.0,
            leverage: 10,
            indicators: BTreeMap::new(),
            positions: ModulePositions {
                long: None,
                short: None,
            },
            sug_info: None,
            state: None,
        }
    }

    #[test]
    fn start_selects_mrc_crossing_without_changing_stairs_default() {
        let mut state = EngineState::default();
        let mut values = std::collections::HashMap::new();
        values.insert("strategy_mode".to_string(), serde_json::json!(1.0));
        values.insert("entry_type".to_string(), serde_json::json!(0.0));
        values.insert("required_samples".to_string(), serde_json::json!(5.0));
        values.insert("min_sl_percent".to_string(), serde_json::json!(0.75));
        values.insert("risk_target_percent".to_string(), serde_json::json!(1.25));
        let output = run(
            &input(ModuleEvent::Signal {
                direction: Direction::Both,
                code: "start".to_string(),
                values,
            }),
            &mut state,
        );
        assert!(output.debug.contains("strategy=mrc-crossing"));
        assert_eq!(state.cfg.strategy_mode, StrategyMode::MrcCrossing);
        assert_eq!(state.cfg.entry_type, EntryType::Market);
        assert_eq!(state.cfg.required_samples(), 5);
        assert_eq!(state.cfg.min_sl_percent(), 0.75);
        assert_eq!(state.cfg.risk_target_percent(), 1.25);
        assert!(output.debug.contains("required_samples=5"));
        assert!(output.debug.contains("min_sl_percent=0.75%"));
        assert!(output.debug.contains("risk_target_percent=1.25%"));
        assert!(output.debug.contains("entry=market"));
        assert_eq!(
            EngineState::default().cfg.strategy_mode,
            StrategyMode::Stairs
        );
    }
}

fn refresh_indicators(
    input: &ModuleInput,
    state: &mut EngineState,
    timeframes: &[i64],
    is_init: bool,
) -> ModuleOutput {
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

    let snapshot = match market_data::merge_indicator_snapshot(
        state.last_snapshot.as_ref(),
        &state.history,
        &input.indicators,
        timeframes,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let error = snapshot_error(
                "Indicators",
                &timeframes_context(timeframes),
                &input.indicators,
                error,
            );
            state.last_error = Some(error.clone());
            return ModuleOutput {
                debug: error,
                ..Default::default()
            };
        }
    };
    if let Err(error) = market_data::validate_indicator_snapshot(&snapshot) {
        state.last_error = Some(error.clone());
        return ModuleOutput {
            debug: error,
            ..Default::default()
        };
    }
    refresh_last_snapshot(state, &snapshot);
    state.last_error = None;
    state.ticks = state.ticks.saturating_add(1);
    if is_init || timeframes.contains(&TF_1M) {
        cascade::update(&mut state.cascade_tf, &snapshot.tf1m);
        state.volume_gain.observe(
            state.bar_1m,
            snapshot.tf1m.volume_gain,
            snapshot.tf1m.buy_sell_gain,
            snapshot.tf1m.mrc_signal.zone,
            state.mrc_crossing.candle_cross_mask,
            snapshot.tf1m.candle.high,
            snapshot.tf1m.candle.low,
            snapshot.tf1m.candle.close,
        );
        state.mrc_crossing.candle_cross_mask = 0;
    }
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

    let Some(snapshot) = state.last_snapshot.clone() else {
        let error = format!(
            "event=SugInfo time={} symbol={} missing-cached-snapshot {}",
            sug_info.time,
            sug_info.symbol_key,
            market_data::render_raw_indicator_inventory(&input.indicators)
        );
        state.last_error = Some(error.clone());
        return ModuleOutput {
            debug: error,
            ..Default::default()
        };
    };
    state.last_error = None;
    state.ticks = state.ticks.saturating_add(1);
    let strategy = builder::ensure_runtime_builder(state);
    let mrc_events = vlines::update_with_events(
        &mut state.vlines,
        &mut state.mrc_crossing,
        snapshot.tf1m.mrc.lines(),
        input.price,
        state.ticks,
        sug_info.time as u64,
    );
    let sug_snapshot = market_data::build_sug_info_snapshot(sug_info);
    let is_mrc_crossing = state.cfg.strategy_mode == StrategyMode::MrcCrossing;
    let candle = crate::model::Candle {
        open: sug_info.candle.enter,
        high: sug_info.candle.max,
        low: sug_info.candle.min,
        close: sug_info.candle.exit,
    };
    let cas_tf = cascade::preview_from_candle(&state.cascade_tf, &candle);
    let sideway = sideways::analyze(state, &snapshot.tf1m);
    state
        .mrc_risk
        .observe(&snapshot, &sug_snapshot, input.price, sug_info.time as u64);

    let mrc_observation = if is_mrc_crossing {
        Some(mrc_crossing::observe(
            &mut state.mrc_crossing_model,
            &mrc_events,
            snapshot.tf1m.mrc.lines(),
            state.cfg.required_samples(),
        ))
    } else {
        None
    };

    let regime = regime::classify_regime(&snapshot.tf5m, &snapshot.tf15m, &snapshot.tf1h);
    state.direction_source = "regime".to_string();
    state.direction_reason = regime::direction_reason(&snapshot.tf5m, &snapshot.tf15m);
    let mrc_reason_direction = mrc_observation
        .as_ref()
        .and_then(|observation| observation.entry.as_ref())
        .map(mrc_crossing::reason_direction)
        .or_else(|| {
            state
                .mrc_crossing_model
                .pending_entry
                .as_ref()
                .map(mrc_crossing::reason_direction)
        });
    let entry_direction = if is_mrc_crossing {
        mrc_reason_direction
            .filter(|direction| *direction != 0)
            .map(mrc_crossing::market_state)
            .unwrap_or_else(|| mrc_crossing::market_state(state.mrc_crossing_model.last_direction))
    } else {
        regime.confirmed
    };
    let entry_side = match entry_direction {
        crate::model::MarketState::Long => crate::model::PositionState::Long,
        crate::model::MarketState::Short => crate::model::PositionState::Short,
        crate::model::MarketState::Sideways => crate::model::PositionState::Flat,
    };
    let entry_gate = if is_mrc_crossing {
        let mut gate = prediction::mrc_crossing_entry_gate(state);
        if let Some(observation) = mrc_observation {
            if let Some(reason) = observation.entry {
                let direction = match entry_direction {
                    crate::model::MarketState::Long => 1,
                    crate::model::MarketState::Short => -1,
                    crate::model::MarketState::Sideways => 0,
                };
                let valid = match reason {
                    crate::model::EntryReason::MrcCrossing {
                        price: _,
                        target_price,
                        stop_price,
                        ..
                    } => mrc_crossing::valid_price_levels(
                        direction,
                        input.price,
                        target_price,
                        stop_price,
                        Some(input.symbol_info.price_tick_size),
                    ),
                    _ => false,
                };
                if valid {
                    gate = crate::model::EntryGate::Open {
                        notes: vec![
                            "strategy=mrc-crossing".to_string(),
                            format!("samples={}", observation.samples),
                        ],
                        reason: crate::model::EntryOpenReason::Entry(reason),
                    };
                } else {
                    state.mrc_crossing_model.pending_entry = None;
                    gate = crate::model::EntryGate::Wait(crate::model::EntryWait {
                        reason: "mrc-invalid-exit-levels".to_string(),
                        solutions: Vec::new(),
                        notes: vec!["MRC target/stop collapse to the entry tick".to_string()],
                    });
                }
            } else if let Some(wait_reason) = observation.wait_reason {
                gate = prediction::mrc_wait_entry_gate(state, wait_reason);
            }
        }
        gate
    } else {
        prediction::entry_gate(&snapshot, &sug_snapshot, &cas_tf, state, entry_side)
    };
    let base_decision = crate::model::TradeDecision {
        market_state: entry_direction,
        entry: entry_gate,
    };
    let decision = if is_mrc_crossing {
        // MRC crossing is a self-contained signal path. The generic builder
        // may rewrite structural entries from sideways/cascade inputs, which
        // are unrelated to the MRC crossing model.
        base_decision
    } else {
        strategy.refine_realtime_decision(
            state,
            &snapshot,
            &sug_snapshot,
            &sideway,
            regime,
            &cas_tf,
            sug_info,
            base_decision,
        )
    };
    state
        .direction_history
        .observe(decision.market_state, input.price, sug_info.time as u64);

    let execution =
        execution::evaluate_with_cascade(input, state, &snapshot, &sug_snapshot, &decision);
    let debug = debug::render(
        state,
        &snapshot,
        &regime,
        &cas_tf,
        &decision,
        execution
            .as_ref()
            .and_then(|result| result.constraint.as_deref()),
        input.symbol_info.price_tick_size,
    );
    let execution = execution.unwrap_or_default();

    ModuleOutput {
        open_positions: execution.open_positions,
        close_positions: execution.close_positions,
        update_positions: execution.update_positions,
        place_orders: execution.tp_orders,
        cancel_orders: execution.cancel_orders,
        debug,
        ..Default::default()
    }
}
