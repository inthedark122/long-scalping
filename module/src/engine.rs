use crate::debug;
use crate::detection::{
    cascade, decision, mrc_crossing, mrc_direction_prediction, mrc_prediction, vlines,
};
use crate::market_data;
use crate::model::EntryType;
use crate::order_manager;
use crate::orders::entry_order_marks;
use crate::risk;
use crate::state::{
    dir_to_position, direction_sign, sync_position_state, EngineState, PositionLifetime,
    PositionRuntime,
};
use crate::strategies::trending::exit;
use crate::strategies::{self, StrategyKind};
use lte_strategy_bridge::abi::{
    Direction, ModuleDataRequirements, ModuleEvent, ModuleInput, ModuleMarketDecision,
    ModuleOrderStatus, ModuleOutput, ModuleUpdatePosition, OrderRole,
};
use lte_strategy_bridge::MODULE_ABI_VERSION;
use std::collections::BTreeMap;

const TF_1M: i32 = market_data::TF_1M;
const TF_5M: i32 = market_data::TF_5M;

fn timeframes_context(timeframes: &[i32]) -> String {
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

fn reset_market_analysis_state(state: &mut EngineState) {
    state.ticks = 0;
    state.ntps_5s_stats.reset();
    state.mrc_crossing_model = Default::default();
    state.mrc_crossing_model.decision_components =
        decision::DecisionComponents::from_names(&state.cfg.decision_components).0;
    state.vlines = [crate::state::VLineState::default(); 9];
    state.mrc_crossing = crate::state::MrcCrossingState::default();
    state.mrc_touch.reset();
    state.mrc_touch_learning.reset();
    state.mrc_direction_prediction = Default::default();
    state.mrc_risk = Default::default();
    state.cascade = crate::detection::cascade::CascadeState::default();
    state.cascade_signal = crate::detection::cascade::CascadeSignal::default();
    state.volume_gain = Default::default();
    state.strategy_runtime.reset_stable_sideways_range();
    state.staircase_progress = crate::state::StaircaseProgress::default();
    state.candle_uptrend_seen = false;
    state.candle_downtrend_seen = false;
    state.candle_trend_direction = 0;
    state.candle_trend_last_closed_ms = 0;
    if state.position.side == crate::model::PositionState::Flat {
        state.active_mrc_analysis = None;
        state.active_mrc_context = Default::default();
    }
    state.market_mrc_analysis = None;
    state.market_mrc_context = Default::default();
    state.selected_cascade_trade_state =
        crate::detection::cascade::CascadeTradeState::NotConfigured;
    state.last_active_tp_recalc_bar = None;
    state.last_error = None;
    state.last_exit_event.clear();
    state.pending_exit_context = None;
    state.tp_reprice_history.reset();
}

fn seed_mrc_from_kline_history(state: &mut EngineState, now_ms: u64) -> usize {
    let Ok(history) = market_data::build_mrc_history(&state.mrc_calculation) else {
        return 0;
    };
    let warmup_bars = market_data::MRC_WARMUP_BARS.min(history.len());
    let replay_start = history
        .len()
        .saturating_sub(market_data::MARKET_REPLAY_BARS)
        .max(warmup_bars);
    let replay_history = &history[replay_start..];
    if replay_history.is_empty() {
        state.mrc_crossing_model.seeded_from_history = true;
        return 0;
    }

    mrc_crossing::seed_from_history_with_touch_learning(
        &mut state.mrc_crossing_model,
        &mut state.vlines,
        &mut state.mrc_crossing,
        replay_history,
        now_ms,
        &mut state.mrc_touch,
        &mut state.mrc_touch_learning,
    );

    if let Ok(snapshots) =
        market_data::build_timeframe_snapshot_history(&state.mrc_calculation, TF_1M)
    {
        let snapshot_start = snapshots.len().saturating_sub(replay_history.len());
        let replay_snapshots = &snapshots[snapshot_start..];
        state.cascade = crate::detection::cascade::CascadeState::default();
        state.cascade_signal = crate::detection::cascade::CascadeSignal::default();
        state.volume_gain = Default::default();
        state.strategy_runtime.reset_stable_sideways_range();
        state.bar_1m = state.bar_1m.max(replay_history.len() as u64);
        let first_bar = state.bar_1m.saturating_sub(replay_history.len() as u64);
        for (index, (sample, snapshot)) in replay_history.iter().zip(replay_snapshots).enumerate() {
            state.cascade_signal = cascade::update_at(&mut state.cascade, snapshot, sample.time_ms);
            state.volume_gain.observe(
                first_bar.saturating_add(index as u64 + 1),
                snapshot.volume_gain,
                snapshot.buy_sell_gain,
                snapshot.mrc_signal.zone,
                0,
                snapshot.candle.high,
                snapshot.candle.low,
                snapshot.candle.close,
            );
            if state.cfg.sideways_nonstop {
                let candidate = crate::detection::movement_window::detect(
                    &state.cascade,
                    snapshot.candle.close,
                )
                .filter(crate::strategies::sideways::is_range);
                state.strategy_runtime.update_stable_sideways_range(
                    &state.cascade,
                    candidate,
                    snapshot.candle.close,
                );
            }
        }
    }
    state.ticks = state.ticks.max(replay_history.len() as u64);
    replay_history.len()
}

pub fn run(input: &ModuleInput, state: &mut EngineState) -> ModuleOutput {
    if let Some(market_decision) = input.market_decision.as_ref() {
        state.last_market_decision_time_ms = market_decision.time.max(0) as u64;
    }

    match &input.event {
        ModuleEvent::Requirements => ModuleOutput {
            data_requirements: ModuleDataRequirements {
                abi_version: MODULE_ABI_VERSION,
                klines: market_data::kline_requirements(),
                internal_indicators: BTreeMap::new(),
            },
            debug: format!(
                "requirements klines=[60:{},300:{}] indicators=unsupported",
                market_data::TF_1M_HISTORY_COUNT,
                market_data::TF_5M_HISTORY_COUNT
            ),
            ..Default::default()
        },
        ModuleEvent::Init => {
            reset_market_analysis_state(state);
            state.active_mrc_analysis = None;
            state.active_mrc_context = Default::default();
            state.last_snapshot = None;
            state.klines.init(&input.klines);
            let mut output = refresh_klines(input, state, &[TF_1M, TF_5M], true, true);
            if state.last_error.is_none() {
                let latest_kline_time_ms = state
                    .klines
                    .get_prev(TF_1M, 0)
                    .map(|kline| kline.time.saturating_add(i64::from(TF_1M) * 1_000).max(0) as u64)
                    .filter(|time| *time > 0)
                    .unwrap_or_default();
                let now_ms = state.last_market_decision_time_ms.max(latest_kline_time_ms);
                state.last_market_decision_time_ms = now_ms;
                let replayed = seed_mrc_from_kline_history(state, now_ms);
                output.debug = format!(
                    "{} market-replay={replayed}/{} warmup={}",
                    output.debug,
                    market_data::MARKET_REPLAY_BARS,
                    market_data::MRC_WARMUP_BARS
                );
            }
            output
        }
        ModuleEvent::KlineUpdate { timeframes } => {
            let previous_1m_time = state.klines.get_prev(TF_1M, 0).map(|kline| kline.time);
            state.klines.update(&input.klines);
            let current_1m_time = state.klines.get_prev(TF_1M, 0).map(|kline| kline.time);
            let new_1m_bar = timeframes.contains(&TF_1M)
                && current_1m_time.is_some_and(|time| previous_1m_time != Some(time));
            refresh_klines(input, state, timeframes, false, new_1m_bar)
        }
        ModuleEvent::Indicators { timeframes } => ModuleOutput {
            debug: format!(
                "unsupported-event=Indicators tfs=[{}]",
                timeframes
                    .iter()
                    .map(i64::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            ..Default::default()
        },
        ModuleEvent::InternalIndicators { timeframes } => ModuleOutput {
            debug: format!("unsupported-event=InternalIndicators tfs={timeframes:?}"),
            ..Default::default()
        },
        ModuleEvent::MarketDecision => match input.market_decision.as_ref() {
            Some(market_decision) => process_realtime(input, state, market_decision),
            None => ModuleOutput {
                debug: "market-decision missing".to_string(),
                ..Default::default()
            },
        },
        ModuleEvent::Signal {
            direction,
            code,
            values,
        } => handle_signal(direction, code, values, state),
        ModuleEvent::Action { name, values } => handle_action(input, state, name, values),
        ModuleEvent::NewPosition {
            direction,
            entry_price,
            qty,
            take_profit,
            stop_loss,
        } => handle_new_position(
            input,
            state,
            *direction,
            *entry_price,
            *qty,
            *take_profit,
            *stop_loss,
        ),
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
        // A directional start is an explicit reservation. Direction::Both
        // intentionally remains free to switch sides.
        state.mrc_crossing_model.trade_direction_lock = direction_sign(*direction);
        state.trade_budget = 0.0;
        state.mrc_crossing_model.current_entry = None;
        state.pending_entry_reason = None;
        state.pending_entry_plan = None;
        state.pending_entry_bar = None;
        state.pending_strategy = None;
        state.strategy_runtime.reset();
        state.cfg.trend_only = false;
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
        if let Some(sideways_only) = values
            .get("sideways_only")
            .and_then(|value| value.as_f64())
            .filter(|value| value.is_finite())
        {
            state.cfg.sideways_only = sideways_only >= 0.5;
        }
        if let Some(sideways_nonstop) = values
            .get("sideways_nonstop")
            .and_then(|value| value.as_f64())
            .filter(|value| value.is_finite())
        {
            state.cfg.sideways_nonstop = sideways_nonstop >= 0.5;
        }
        if let Some(trend_only) = values
            .get("trend_only")
            .and_then(|value| value.as_f64())
            .filter(|value| value.is_finite())
        {
            state.cfg.trend_only = trend_only >= 0.5;
        }
        if let Some(required_samples) = values
            .get("required_samples")
            .and_then(|value| value.as_f64())
            .filter(|value| value.is_finite())
        {
            state.cfg.required_samples =
                required_samples.round().clamp(0.0, u32::MAX as f64) as u32;
        }
        if let Some(names) = value_names(values.get("decision_components")) {
            apply_decision_components(state, names, false);
        } else if let Some(mode) = primary_mode_name(values.get("primary_mode")) {
            if let Some(components) = decision::DecisionComponents::for_primary_mode(&mode) {
                apply_decision_components(state, components.names(), false);
            } else {
                state.mrc_crossing_model.decision_components =
                    decision::DecisionComponents::from_names(&state.cfg.decision_components).0;
            }
        } else {
            state.mrc_crossing_model.decision_components =
                decision::DecisionComponents::from_names(&state.cfg.decision_components).0;
        }
        reset_market_analysis_state(state);
        state.mrc_crossing_model.trade_direction_lock = direction_sign(*direction);
        state.exit_progress.reset();
        let now_ms = state.last_market_decision_time_ms;
        if now_ms > 0 {
            seed_mrc_from_kline_history(state, now_ms);
        }
        state.cfg_loaded = true;
        ModuleOutput {
            debug: format!(
                "start direction={:?} strategy_mode={} strategies=[trending,sideways] cap_init={:.2}% trading_amount={:.2}% min_tp_percent={:.2}% min_sl_percent={:.2}% risk_target_percent={:.2}% required_samples={} decision_components=[{}]",
                state.allowed_direction,
                if state.cfg.trend_only {
                    "trend-only"
                } else if state.cfg.sideways_nonstop {
                    "sideways-stable-range-experiment"
                } else if state.cfg.sideways_only {
                    "sideways-only"
                } else {
                    "mixed"
                },
                state.cfg.cap_init,
                state.cfg.trading_amount,
                state.cfg.min_tp_percent(),
                state.cfg.min_sl_percent(),
                state.cfg.risk_target_percent(),
                state.cfg.required_samples(),
                state.cfg.decision_components.join(",")
            ),
            cancel_orders: entry_order_marks(),
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
    state: &mut EngineState,
    name: &str,
    values: &std::collections::HashMap<String, serde_json::Value>,
) -> ModuleOutput {
    let normalized = name.replace('-', "_").to_ascii_lowercase();
    if !matches!(
        normalized.as_str(),
        "decision_components" | "decision_component"
    ) {
        return ModuleOutput {
            debug: format!("action {} ignored", name),
            ..Default::default()
        };
    }

    let operation = if values.contains_key("components") {
        let Some(names) = value_names(values.get("components")) else {
            return ModuleOutput {
                debug: "action decision_components invalid components".to_string(),
                ..Default::default()
            };
        };
        apply_decision_components(state, names, true)
    } else if values.contains_key("add") {
        let Some(names) = value_names(values.get("add")) else {
            return ModuleOutput {
                debug: "action decision_components invalid add".to_string(),
                ..Default::default()
            };
        };
        let mut components = state.cfg.decision_components.clone();
        components.extend(names);
        apply_decision_components(state, components, true)
    } else if values.contains_key("remove") {
        let Some(names) = value_names(values.get("remove")) else {
            return ModuleOutput {
                debug: "action decision_components invalid remove".to_string(),
                ..Default::default()
            };
        };
        let names = names.into_iter().collect::<std::collections::HashSet<_>>();
        let components = state
            .cfg
            .decision_components
            .iter()
            .filter(|component| !names.contains(*component))
            .cloned()
            .collect();
        apply_decision_components(state, components, true)
    } else {
        return ModuleOutput {
            debug: "action decision_components requires components, add, or remove".to_string(),
            ..Default::default()
        };
    };
    ModuleOutput {
        debug: format!("action decision_components {}", operation),
        ..Default::default()
    }
}

fn value_names(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
    match value? {
        serde_json::Value::String(name) => Some(
            name.split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect(),
        ),
        serde_json::Value::Array(values) => values
            .iter()
            .map(|value| value.as_str().map(str::to_string))
            .collect(),
        _ => None,
    }
}

fn primary_mode_name(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(mode) => Some(mode.clone()),
        serde_json::Value::Number(value) => match value.as_f64()?.round() as i32 {
            0 => Some("all".to_string()),
            1 => Some("stairs".to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn apply_decision_components(
    state: &mut EngineState,
    names: Vec<String>,
    report_unknown: bool,
) -> String {
    let (components, unknown) = decision::DecisionComponents::from_names(&names);
    if report_unknown && !unknown.is_empty() && !names.is_empty() && components.names().is_empty() {
        return format!(
            "rejected unknown=[{}] current=[{}]",
            unknown.join(","),
            state.cfg.decision_components.join(",")
        );
    }
    let previous_primary = state.mrc_crossing_model.decision_state.primary;
    state.cfg.decision_components = components.names();
    state.mrc_crossing_model.decision_components = components;
    state
        .mrc_crossing_model
        .decision_state
        .reconfigure(components);
    if previous_primary.is_some() && state.mrc_crossing_model.decision_state.primary.is_none() {
        state.mrc_crossing_model.current_entry = None;
        state.mrc_crossing_model.last_plan_key = None;
        state.mrc_crossing_model.active_pattern = None;
        state.mrc_crossing_model.active_pattern_score = 0.0;
    }
    let mut result = format!("components=[{}]", state.cfg.decision_components.join(","));
    if report_unknown && !unknown.is_empty() {
        result.push_str(&format!(" unknown=[{}]", unknown.join(",")));
    }
    result
}

fn handle_new_position(
    input: &ModuleInput,
    state: &mut EngineState,
    direction: Direction,
    entry_price: f64,
    qty: f64,
    take_profit: Option<f64>,
    stop_loss: Option<f64>,
) -> ModuleOutput {
    let side = dir_to_position(direction);
    let entry_time_ms = state.last_market_decision_time_ms;
    let strategy = state.pending_strategy.take().unwrap_or_default();
    let pending_plan = state.pending_entry_plan.take();
    let host_take_profit = take_profit.filter(|price| price.is_finite() && *price > 0.0);
    let host_stop_loss = stop_loss.filter(|price| price.is_finite() && *price > 0.0);
    let pending_take_profit = pending_plan
        .map(|plan| plan.take_profit)
        .filter(|price| price.is_finite() && *price > 0.0);
    let pending_stop_loss = pending_plan
        .map(|plan| plan.stop_loss)
        .filter(|price| price.is_finite() && *price > 0.0);
    let take_profit = host_take_profit.or(pending_take_profit);
    let stop_loss = host_stop_loss.or(pending_stop_loss);
    let tp_source = if host_take_profit.is_some() {
        "host"
    } else if pending_take_profit.is_some() {
        "pending-entry-plan"
    } else {
        "none"
    };
    let repair_exit_plan =
        (host_take_profit.is_none() || host_stop_loss.is_none()).then_some(ModuleUpdatePosition {
            direction,
            enter_price: None,
            take_profit,
            stop_loss,
        });

    state.pending_entry_reason = None;
    state.pending_entry_bar = None;
    state.take_profit_allocation.reset();
    state.pending_exit_context = None;
    state.position_context.begin(
        side,
        entry_price,
        entry_time_ms,
        None,
        PositionLifetime::OneShot,
        Vec::new(),
    );
    state.position_context.strategy = strategy;
    state.position = PositionRuntime {
        side,
        entry_price,
        peak_price: input.price.max(entry_price),
        trough_price: input.price.min(entry_price),
        take_profit: take_profit.unwrap_or_default(),
        stop_loss: stop_loss.unwrap_or_default(),
        stop_loss_reason: if stop_loss.is_some() {
            "provided-by-host".to_string()
        } else {
            String::new()
        },
        exit_tp_reliable: take_profit.is_some(),
        entry_reason: None,
        ..PositionRuntime::default()
    };
    state.mrc_crossing_model.current_entry = None;

    ModuleOutput {
        update_positions: repair_exit_plan.into_iter().collect(),
        debug: format!(
            "new-position observed side={:?} entry={:.8} qty={:.8} strategy={} tp={} tp_source={} sl={}",
            direction,
            entry_price,
            qty,
            strategy.as_str(),
            take_profit
                .map(|price| format!("{price:.8}"))
                .unwrap_or_else(|| "none".to_string()),
            tp_source,
            stop_loss
                .map(|price| format!("{price:.8}"))
                .unwrap_or_else(|| "none".to_string()),
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
    mrc_crossing::record_trade_result(
        &mut state.mrc_crossing_model,
        state.position.entry_reason.as_ref(),
        pnl,
    );
    risk::apply_close_pnl(input, state, pnl);
    if state.position.side == dir_to_position(direction) {
        state
            .position_context
            .on_flat(format!("finish-confirmed direction={:?}", direction));
        state.position = PositionRuntime::default();
        state.take_profit_allocation.reset();
        state.last_exit_event.clear();
        state.pending_entry_bar = None;
        state.mrc_crossing_model.last_plan_key = None;
        state.active_mrc_analysis = None;
        state.active_mrc_context = mrc_prediction::MrcPredictionContext::default();
    }
    let mut cancel_orders = exit::all_tp_marks(direction);
    cancel_orders.extend(entry_order_marks());
    state.exit_progress.reset();
    ModuleOutput {
        cancel_orders,
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
            internal_indicators: BTreeMap::new(),
            klines: BTreeMap::new(),
            positions: ModulePositions {
                long: None,
                short: None,
            },
            market_decision: None,
            state: None,
        }
    }

    #[test]
    fn start_initializes_runtime_configuration() {
        let mut state = EngineState::default();
        let mut values = std::collections::HashMap::new();
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
        assert!(output
            .debug
            .contains("strategy_mode=mixed strategies=[trending,sideways]"));
        assert_eq!(state.cfg.entry_type, EntryType::Market);
        assert_eq!(state.cfg.required_samples(), 5);
        assert_eq!(state.cfg.min_sl_percent(), 0.75);
        assert_eq!(state.cfg.risk_target_percent(), 1.25);
        assert!(output.debug.contains("required_samples=5"));
        assert!(output.debug.contains("min_sl_percent=0.75%"));
        assert!(output.debug.contains("risk_target_percent=1.25%"));
    }
}

fn refresh_klines(
    input: &ModuleInput,
    state: &mut EngineState,
    timeframes: &[i32],
    is_init: bool,
    new_1m_bar: bool,
) -> ModuleOutput {
    let init_input_diagnostics = if is_init {
        format!(
            " {}",
            market_data::render_kline_input_diagnostics(&input.klines)
        )
    } else {
        String::new()
    };

    if new_1m_bar {
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

    let calculation = if is_init {
        state.mrc_calculation.initialize(&state.klines)
    } else {
        state.mrc_calculation.update(&input.klines, &state.klines)
    };
    if let Err(error) = calculation {
        let error = format!(
            "event={} {} {} {}{}",
            if is_init { "Init" } else { "KlineUpdate" },
            timeframes_context(timeframes),
            error,
            market_data::render_kline_history_inventory(&state.klines),
            init_input_diagnostics
        );
        state.last_error = Some(error.clone());
        return ModuleOutput {
            debug: error,
            ..Default::default()
        };
    }

    let snapshot =
        match market_data::build_indicator_snapshot(&state.klines, &state.mrc_calculation) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let error = format!(
                    "event={} {} {} {}{}",
                    if is_init { "Init" } else { "KlineUpdate" },
                    timeframes_context(timeframes),
                    error,
                    market_data::render_kline_history_inventory(&state.klines),
                    init_input_diagnostics
                );
                state.last_error = Some(error.clone());
                return ModuleOutput {
                    debug: error,
                    ..Default::default()
                };
            }
        };
    if let Err(error) = market_data::validate_indicator_snapshot(&snapshot) {
        let error = format!("{error}{init_input_diagnostics}");
        state.last_error = Some(error.clone());
        return ModuleOutput {
            debug: error,
            ..Default::default()
        };
    }
    refresh_last_snapshot(state, &snapshot);
    state.last_error = None;
    state.ticks = state.ticks.saturating_add(1);
    if is_init || new_1m_bar {
        let now_ms = state
            .klines
            .get_prev(TF_1M, 0)
            .map(|kline| kline.time.saturating_add(i64::from(TF_1M) * 1_000).max(0) as u64)
            .filter(|time| *time > 0)
            .unwrap_or(state.last_market_decision_time_ms);
        state.cascade_signal = cascade::update_at(&mut state.cascade, &snapshot.tf1m, now_ms);
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
        debug: if is_init {
            format!(
                "event=Init ta-rs snapshot ready {}{}",
                market_data::render_kline_history_inventory(&state.klines),
                init_input_diagnostics
            )
        } else {
            String::new()
        },
        ..Default::default()
    }
}

fn process_realtime(
    input: &ModuleInput,
    state: &mut EngineState,
    market_decision: &ModuleMarketDecision,
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
            "event=MarketDecision time={} symbol={} missing-cached-snapshot {}",
            market_decision.time,
            market_decision.symbol_key,
            market_data::render_kline_history_inventory(&state.klines)
        );
        state.last_error = Some(error.clone());
        return ModuleOutput {
            debug: error,
            ..Default::default()
        };
    };
    state.last_error = None;
    state.ticks = state.ticks.saturating_add(1);
    if !state.mrc_crossing_model.seeded_from_history {
        seed_mrc_from_kline_history(state, market_decision.time.max(0) as u64);
    }
    let mrc_events = vlines::update_with_events(
        &mut state.vlines,
        &mut state.mrc_crossing,
        snapshot.tf1m.mrc.lines(),
        input.price,
        state.ticks,
        market_decision.time as u64,
    );
    state.mrc_touch.update(
        snapshot.tf1m.mrc.lines(),
        input.price,
        market_decision.time.max(0) as u64,
    );
    let touched_lines = mrc_events
        .iter()
        .map(|event| event.line_idx)
        .collect::<Vec<_>>();
    state.mrc_touch_learning.update_with_events(
        snapshot.tf1m.mrc.lines(),
        input.price,
        market_decision.time.max(0) as u64,
        snapshot.tf5m.mrc.lines(),
        snapshot.tf5m.mrc_signal.slope_sign,
        &touched_lines,
    );
    state.mrc_direction_prediction = mrc_direction_prediction::predict(
        state.mrc_touch.signal,
        mrc_direction_prediction::MrcDirectionContext {
            mean_distance_pct: state
                .mrc_touch
                .signal
                .line(4)
                .map(|line| line.distance_ema_pct)
                .unwrap_or(0.0),
            touch_learning: state.mrc_touch_learning.signal,
            ..Default::default()
        },
    );
    state
        .mrc_risk
        .observe(&snapshot, input.price, market_decision.time as u64);

    let prediction_context = mrc_prediction::MrcPredictionContext {
        cascade_enabled: true,
        ntps: market_decision.ntp_5s,
        ntps_fast_time_ms: market_decision.tdt,
        cascade_direction: state.cascade_signal.dir,
        cascade_speed_pct_per_min: state.cascade_signal.speed_pct_per_min,
        cascade_strength: state.cascade_signal.strength,
        cascade_stretched: state.cascade_signal.stretched,
        cascade_phase: state.cascade_signal.phase,
        cascade_potential_move_pct: state.cascade_signal.potential_move_pct,
        cascade_adverse_move_pct: state.cascade_signal.adverse_move_pct,
        higher_timeframe_slope: snapshot.tf5m.mrc_signal.slope_sign,
        higher_timeframe_bullish: snapshot.tf5m.mrc_signal.bullish_structure,
        higher_timeframe_bearish: snapshot.tf5m.mrc_signal.bearish_structure,
    };
    state.market_mrc_context = prediction_context;
    let mrc_observation = mrc_crossing::observe_with_forecast_and_touch_and_learning_context(
        &mut state.mrc_crossing_model,
        &mrc_events,
        snapshot.tf1m.mrc.lines(),
        &state.mrc_crossing,
        (
            snapshot.tf1m.volume_gain,
            snapshot.tf1m.volume_gain_min,
            snapshot.tf1m.volume_gain_max,
            snapshot.tf1m.buy_sell_gain,
        ),
        input.price,
        market_decision.time as u64,
        prediction_context,
        state.mrc_touch.signal,
        state.mrc_touch_learning.signal,
    );
    if let Some(index) = mrc_observation.selected_candidate_index {
        if let Some(candidate) = mrc_observation.candidates.get(index) {
            state.mrc_direction_prediction = candidate.direction_prediction;
        }
    }

    let selected_analysis = mrc_observation.selected_analysis();
    let pattern = crate::pattern_matching::evaluate(
        state.klines.get(TF_1M),
        market_decision.time.max(0) as u64,
    );
    let detected_trend_direction = state
        .mrc_crossing_model
        .decision_state
        .primary
        .map(|primary| primary.direction);
    let selected_forecast = mrc_observation
        .selected_candidate_index
        .and_then(|index| mrc_observation.candidates.get(index))
        .is_some_and(|candidate| candidate.forecast);
    crate::entry_policy::state::update(state, selected_analysis, prediction_context);
    let movement_window = crate::detection::movement_window::detect(&state.cascade, input.price);
    let stable_sideways_range = if state.cfg.sideways_nonstop {
        state.strategy_runtime.update_stable_sideways_range(
            &state.cascade,
            movement_window.filter(crate::strategies::sideways::is_range),
            input.price,
        )
    } else {
        state.strategy_runtime.reset_stable_sideways_range();
        None
    };
    let entry_decision = strategies::evaluate_entry(
        selected_analysis,
        pattern.current.map(|current| current.kind),
        pattern.forecast,
        detected_trend_direction,
        snapshot.tf1m.mrc,
        snapshot.tf1m.candle,
        input.price,
        input.symbol_info.price_tick_size,
        movement_window,
        stable_sideways_range,
        state.strategy_runtime.last_sideways_movement_start_ms(),
        state.cascade.movement_start_ms,
        state.cascade_signal.dir,
        state.pending_strategy == Some(StrategyKind::Sideways) && state.pending_entry_bar.is_some(),
        state.cfg.trend_only,
        state.cfg.sideways_only,
        state.cfg.sideways_nonstop,
    );
    let routed_strategy = entry_decision.strategy();
    let entry_plan_direction = entry_decision.direction();
    let mut output = ModuleOutput::default();
    let entry_status;
    let mut exit_status = "no-position".to_string();
    let constraint;

    if state.position.side == crate::model::PositionState::Flat {
        let execution = strategies::execute_entry(input, state, entry_decision);
        entry_status = execution.status;
        constraint = execution.constraint;
        output.cancel_orders = execution.cancel_orders;
        output.open_positions.extend(execution.open_position);
        output.place_orders.extend(execution.place_order);
    } else {
        entry_status = "position-open".to_string();
        constraint = None;
        output.cancel_orders = entry_order_marks();
        let sideways_handoff = if state.position_context.strategy == StrategyKind::Trending
            && routed_strategy == StrategyKind::Sideways
        {
            movement_window.and_then(|window| {
                strategies::sideways::exit_plan(
                    state.position.side,
                    input.price,
                    input.symbol_info.price_tick_size,
                    window,
                )
            })
        } else {
            None
        };
        if let Some(plan) = sideways_handoff {
            let original_stop = state.position.stop_loss;
            if let exit::Decision::Update(update) = exit::evaluate(
                state.position.side,
                state.position.entry_price,
                input.price,
                state.position.stop_loss,
                snapshot.tf1m.mrc,
            ) {
                exit::execute(state, update);
            }
            let (stop_loss, sideways_stop_applied) = exit::protect_handoff_stop_loss(
                state.position.side,
                state.position.entry_price,
                state.position.stop_loss,
                plan.stop_loss,
                snapshot.tf1m.mrc,
            )
            .unwrap_or((state.position.stop_loss, false));
            state.position_context.last_action = crate::state::PositionDecisionAction::UpdateExit;
            state.position_context.last_plan_source =
                crate::state::PositionExitPlanSource::DynamicSameSide;
            state.position_context.decision_reason = "sideways-range-handoff".to_string();
            state.position_context.remember_plan(
                crate::state::PositionPlanSnapshot {
                    take_profit: plan.take_profit,
                    stop_loss,
                    tp_reliable: true,
                },
                crate::state::PositionExitPlanSource::DynamicSameSide,
            );
            state.position.take_profit = plan.take_profit;
            state.position.stop_loss = stop_loss;
            state.position.exit_tp_reliable = true;
            if sideways_stop_applied {
                state.position.stop_loss_reason = "sideways-range-boundary".to_string();
            }
            output.update_positions.push(ModuleUpdatePosition {
                direction: if state.position.side == crate::model::PositionState::Long {
                    Direction::Long
                } else {
                    Direction::Short
                },
                enter_price: None,
                take_profit: Some(plan.take_profit),
                stop_loss: (stop_loss != original_stop).then_some(stop_loss),
            });
            exit_status = "sideways-range-handoff".to_string();
        } else {
            match state.position_context.strategy {
                StrategyKind::Trending => match exit::evaluate(
                    state.position.side,
                    state.position.entry_price,
                    input.price,
                    state.position.stop_loss,
                    snapshot.tf1m.mrc,
                ) {
                    exit::Decision::NoPosition => {}
                    exit::Decision::Hold => exit_status = "trending-hold".to_string(),
                    exit::Decision::Update(update) => {
                        output.update_positions.push(exit::execute(state, update));
                        exit_status = update.reason.to_string();
                    }
                },
                StrategyKind::Sideways => {
                    exit_status = "sideways-fixed-range-tp-sl".to_string();
                }
            }
        }
    }

    let entry_analysis = crate::entry_policy::analysis_summary(
        state,
        &snapshot,
        selected_analysis,
        selected_forecast,
        &pattern,
        input.price,
        market_decision.time.max(0) as u64,
        input.symbol_info.price_tick_size,
    );
    state
        .ntps_5s_stats
        .observe(market_decision.time, market_decision.ntp_5s);
    let debug = debug::render(
        state,
        &snapshot,
        &entry_analysis,
        &entry_status,
        &exit_status,
        routed_strategy,
        entry_plan_direction,
        constraint.as_deref(),
        input.symbol_info.price_tick_size,
        input.price,
        market_decision.ntp_5s,
        market_decision.tdt,
    );
    output.debug = debug;
    let report_minute = market_decision.time.max(0) as u64 / 60_000;
    if state.last_workflow_report_minute != Some(report_minute) {
        state.last_workflow_report_minute = Some(report_minute);
        output.workflow_report = Some(serde_json::json!({
            "strategy": "structural_scalping",
            "symbol": input.symbol,
            "trend_only": state.cfg.trend_only,
            "allowed_direction": match state.allowed_direction {
                Direction::Long => "long",
                Direction::Short => "short",
                Direction::Both => "both",
            },
            "mrc_1m_slope_direction": snapshot.tf1m.mrc_signal.slope_sign,
            "position_side": match state.position.side {
                crate::model::PositionState::Long => "long",
                crate::model::PositionState::Short => "short",
                crate::model::PositionState::Flat => "flat",
            },
            "entry_status": entry_status,
            "entry_plan_direction": match entry_plan_direction {
                Some(Direction::Long) => "long",
                Some(Direction::Short) => "short",
                Some(Direction::Both) => "both",
                None => "none",
            },
            "exit_status": exit_status,
            "max_amount": input.max_amount,
            "auto_max_amount": input.auto_max_amount,
            "configured_leverage": input.leverage,
        }));
    }
    output
}
