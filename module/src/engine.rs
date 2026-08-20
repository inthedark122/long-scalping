use crate::debug;
use crate::detection::{
    cascade, decision, mrc_crossing, mrc_pattern_classification, mrc_prediction, vlines,
};
use crate::execution;
use crate::exit;
use crate::market_data;
use crate::model::{EntryType, ExitMode, MRC_CROSSING_EXIT_TYPE};
use crate::order_manager;
use crate::risk;
use crate::state::{
    dir_to_position, direction_sign, sync_position_state, EngineState, PositionRuntime,
};
use lte_strategy_bridge::abi::{
    Direction, ModuleEvent, ModuleInput, ModuleOrderStatus, ModuleOutput, OrderRole,
};
use lte_strategy_bridge::indicator_access::TimeframeSec;
use lte_strategy_bridge::sug_info::SuggestionInfo;

const TF_1M: i64 = TimeframeSec::Tf1m.as_i64();
const TF_5M: i64 = TimeframeSec::Tf5m.as_i64();

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
            refresh_indicators(input, state, &[TF_1M, TF_5M], true)
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
        } => handle_signal(input, direction, code, values, state),
        ModuleEvent::Action { name, values } => handle_action(input, state, name, values),
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
    input: &ModuleInput,
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
        // Restart the market sequence, not the learned ledger. Pattern
        // performance must survive a fresh start signal so the selector can
        // keep improving instead of relearning from zero every session.
        state.mrc_crossing_model.history.clear();
        state.mrc_crossing_model.prediction_history.clear();
        state.mrc_crossing_model.seeded_from_history = false;
        state.mrc_crossing_model.last_line = None;
        state.mrc_crossing_model.last_direction = 0;
        state.mrc_crossing_model.last_event_ms = 0;
        state.mrc_crossing_model.current_entry = None;
        state.mrc_crossing_model.active_pattern = None;
        state.mrc_crossing_model.active_pattern_score = 0.0;
        state.mrc_crossing_model.active_pattern_since_ms = 0;
        state.mrc_crossing_model.last_plan_key = None;
        state.mrc_crossing_model.decision_state.reset();
        state.vlines = [crate::state::VLineState::default(); 9];
        state.mrc_crossing = crate::state::MrcCrossingState::default();
        state.cascade = crate::detection::cascade::CascadeState::default();
        state.cascade_signal = crate::detection::cascade::CascadeSignal::default();
        state.cascade_entry_marks.clear();
        state.cascade_exit_orders = false;
        if let Some(now_ms) = input
            .sug_info
            .as_ref()
            .map(|sug_info| sug_info.time)
            .filter(|time| *time > 0)
            .map(|time| time as u64)
        {
            mrc_crossing::seed_from_history(
                &mut state.mrc_crossing_model,
                &mut state.vlines,
                &mut state.mrc_crossing,
                &state.history,
                now_ms,
            );
        }
        state.cfg_loaded = true;
        ModuleOutput {
            debug: format!(
                "start direction={:?} strategy={} entry={} cap_init={:.2}% trading_amount={:.2}% min_tp_percent={:.2}% min_sl_percent={:.2}% risk_target_percent={:.2}% required_samples={} decision_components=[{}] exit={:?}",
                state.allowed_direction,
                "pattern-engine",
                state.cfg.entry_type.as_str(),
                state.cfg.cap_init,
                state.cfg.trading_amount,
                state.cfg.min_tp_percent(),
                state.cfg.min_sl_percent(),
                state.cfg.risk_target_percent(),
                state.cfg.required_samples(),
                state.cfg.decision_components.join(","),
                MRC_CROSSING_EXIT_TYPE
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
            2 => Some("outward".to_string()),
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
) -> ModuleOutput {
    state.position = PositionRuntime {
        side: dir_to_position(direction),
        entry_price,
        peak_price: input.price.max(entry_price),
        trough_price: input.price.min(entry_price),
        take_profit: 0.0,
        stop_loss: 0.0,
        stop_loss_reason: String::new(),
        exit_tp_reliable: false,
        exit_mode: ExitMode::Fixed,
        exit_type: MRC_CROSSING_EXIT_TYPE,
        entry_reason: state.position.entry_reason.clone(),
    };
    state.mrc_crossing_model.current_entry = None;
    state.pending_entry_bar = None;

    let side = dir_to_position(direction);
    let snapshot = match market_data::build_indicator_snapshot(&state.history, &input.indicators) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let is_mrc = state.position.entry_reason.as_ref().is_some_and(|reason| {
                matches!(reason, crate::model::EntryReason::MrcCrossing { .. })
            });
            if is_mrc {
                state.last_snapshot.unwrap_or_default()
            } else {
                let error = snapshot_error("Init", "tfs=[60,300]", &input.indicators, error);
                state.last_error = Some(error.clone());
                return ModuleOutput {
                    debug: error,
                    ..Default::default()
                };
            }
        }
    };
    let plan = exit::entry_exit_plan(state, &snapshot, entry_price, side);
    let mut tp_orders = Vec::new();
    let mut cancel_orders = Vec::new();
    if let Some(plan) = plan {
        state.position.exit_mode = ExitMode::Fixed;
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
            let note = plan.take_profit.order_note();
            (tp_orders, cancel_orders) = exit::take_profit_orders(
                input,
                state,
                side,
                qty,
                entry_price,
                plan.take_profit.price(),
                Some(&note),
                state.cascade_signal.multi_order_ready(),
            );
        }
    }

    ModuleOutput {
        place_orders: tp_orders,
        cancel_orders,
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
    mrc_crossing::record_trade_result(
        &mut state.mrc_crossing_model,
        state.position.entry_reason.as_ref(),
        pnl,
    );
    risk::apply_close_pnl(input, state, pnl);
    if state.position.side == dir_to_position(direction) {
        state.position = PositionRuntime::default();
        state.pending_entry_bar = None;
        state.mrc_crossing_model.last_plan_key = None;
    }
    let mut cancel_orders = exit::all_tp_marks(direction);
    cancel_orders.extend(std::mem::take(&mut state.cascade_entry_marks));
    state.cascade_exit_orders = false;
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
            positions: ModulePositions {
                long: None,
                short: None,
            },
            sug_info: None,
            state: None,
        }
    }

    #[test]
    fn start_initializes_the_pattern_engine() {
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
        assert!(output.debug.contains("strategy=pattern-engine"));
        assert_eq!(state.cfg.entry_type, EntryType::Market);
        assert_eq!(state.cfg.required_samples(), 5);
        assert_eq!(state.cfg.min_sl_percent(), 0.75);
        assert_eq!(state.cfg.risk_target_percent(), 1.25);
        assert!(output.debug.contains("required_samples=5"));
        assert!(output.debug.contains("min_sl_percent=0.75%"));
        assert!(output.debug.contains("risk_target_percent=1.25%"));
        assert!(output.debug.contains("entry=market"));
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
        let now_ms = input
            .sug_info
            .as_ref()
            .map(|sug_info| sug_info.time.max(0) as u64)
            .unwrap_or_default();
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
    if !state.mrc_crossing_model.seeded_from_history {
        mrc_crossing::seed_from_history(
            &mut state.mrc_crossing_model,
            &mut state.vlines,
            &mut state.mrc_crossing,
            &state.history,
            sug_info.time.max(0) as u64,
        );
    }
    let mrc_events = vlines::update_with_events(
        &mut state.vlines,
        &mut state.mrc_crossing,
        snapshot.tf1m.mrc.lines(),
        input.price,
        state.ticks,
        sug_info.time as u64,
    );
    state
        .mrc_risk
        .observe(&snapshot, input.price, sug_info.time as u64);

    let prediction_context = mrc_prediction::MrcPredictionContext {
        cascade_enabled: true,
        ntps: sug_info.indicator.ntps,
        ntps_fast_time_ms: sug_info.indicator.ntps_fast_time,
        cascade_direction: state.cascade_signal.dir,
        cascade_speed_pct_per_min: state.cascade_signal.speed_pct_per_min,
        cascade_strength: state.cascade_signal.strength,
        cascade_stretched: state.cascade_signal.stretched,
        cascade_phase: state.cascade_signal.phase,
        cascade_potential_move_pct: state.cascade_signal.potential_move_pct,
        cascade_adverse_move_pct: state.cascade_signal.adverse_move_pct,
    };
    let mrc_observation = mrc_crossing::observe_with_forecast_context(
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
        sug_info.time as u64,
        prediction_context,
    );

    state.direction_source = "pattern-selector".to_string();
    let selected_analysis = mrc_observation.selected_analysis();
    let selected_pattern_reason = mrc_observation
        .selected_pattern
        .map(|pattern| pattern.as_str().to_string())
        .unwrap_or_else(|| {
            mrc_observation
                .wait_reason
                .map(|reason| format!("wait:{reason:?}"))
                .unwrap_or_else(|| "no-candidate".to_string())
        });
    state.direction_reason = selected_analysis
        .map(|analysis| {
            format!(
                "{} {}",
                selected_pattern_reason,
                mrc_crossing::line_plan_summary(analysis)
            )
        })
        .unwrap_or(selected_pattern_reason);
    let mrc_reason_direction = mrc_observation
        .entry
        .as_ref()
        .map(mrc_crossing::reason_direction)
        .or_else(|| {
            state
                .mrc_crossing_model
                .current_entry
                .as_ref()
                .map(mrc_crossing::reason_direction)
        });
    let entry_direction = mrc_reason_direction
        .filter(|direction| *direction != 0)
        .map(mrc_crossing::market_state)
        .unwrap_or_else(|| mrc_crossing::market_state(state.mrc_crossing_model.last_direction));
    let entry_gate = {
        let mut gate = mrc_crossing::entry_gate(&state.mrc_crossing_model);
        if let Some(reason) = mrc_observation.entry.clone() {
            let direction = match entry_direction {
                crate::model::MarketState::Long => 1,
                crate::model::MarketState::Short => -1,
                crate::model::MarketState::Sideways => 0,
            };
            let crate::model::EntryReason::MrcCrossing {
                price,
                target_price,
                stop_price,
                pattern_kind,
                ..
            } = &reason;
            let valid = mrc_crossing::valid_price_levels_for_pattern(
                direction,
                *price,
                *target_price,
                *stop_price,
                Some(input.symbol_info.price_tick_size),
                mrc_pattern_classification::MrcPatternKind::from_str(pattern_kind),
            );
            if valid {
                let analysis_note = selected_analysis.map(|event| {
                            format!(
                                "forecast={} pattern={} steps={} stair_sequence={} stair_price_move_pct={:.4} stair_favorable_move_pct={:.4} stair_adverse_move_pct={:.4} stair_lifecycle={} stair_transition={} stair_protected_line={} stair_extreme_line={} cascade_enabled={} cascade_direction={} cascade_speed_pct_per_min={} cascade_phase={} cascade_potential_move_pct={} cascade_adverse_move_pct={} cascade_trade_state={} barrier={}:{} resistance_line={}@{:.6} learned_line={}@{:.6} predicted_direction={} trade_direction={} tp_line={} tp={:.6} sl_line={} sl={:.6} history_scope={} trigger_move_pct={} volume_gain={:.3} volume_gain_min={:.3} volume_gain_max={:.3} buy_sell_gain={:.3}",
                                mrc_observation.forecast.is_some(),
                                event
                                    .pattern_kind
                                    .map(|kind| kind.as_str())
                                    .unwrap_or("none"),
                                event.pattern_steps,
                                if event.stair_sequence.is_empty() {
                                    "na".to_string()
                                } else {
                                    event
                                        .stair_sequence
                                        .iter()
                                        .filter_map(|line| {
                                            mrc_crossing::LINE_LABELS.get(*line as usize).copied()
                                        })
                                        .collect::<Vec<_>>()
                                        .join(">")
                                },
                                event.stair_price_move_pct,
                                event.stair_favorable_move_pct,
                                event.stair_adverse_move_pct,
                                event
                                    .staircase_lifecycle
                                    .map(|value| value.as_str())
                                    .unwrap_or("na"),
                                event
                                    .staircase_transition
                                    .map(|value| value.as_str())
                                    .unwrap_or("na"),
                                event
                                    .staircase_protected_line
                                    .and_then(|line| mrc_crossing::LINE_LABELS.get(line as usize))
                                    .copied()
                                    .unwrap_or("na"),
                                event
                                    .staircase_extreme_line
                                    .and_then(|line| mrc_crossing::LINE_LABELS.get(line as usize))
                                    .copied()
                                    .unwrap_or("na"),
                                event.cascade_enabled,
                                event.cascade_direction,
                                event
                                    .cascade_speed_pct_per_min
                                    .map(|value| format!("{value:.4}"))
                                    .unwrap_or_else(|| "na".to_string()),
                                event.cascade_phase.as_str(),
                                event
                                    .cascade_potential_move_pct
                                    .map(|value| format!("{value:.4}"))
                                    .unwrap_or_else(|| "na".to_string()),
                                event
                                    .cascade_adverse_move_pct
                                    .map(|value| format!("{value:.4}"))
                                    .unwrap_or_else(|| "na".to_string()),
                                event.cascade_trade_state.as_str(),
                                event.barrier_state.as_str(),
                                event
                                    .barrier_line
                                    .map(|line| mrc_crossing::LINE_LABELS[line as usize])
                                    .unwrap_or("na"),
                                event
                                    .barrier_line
                                    .map(|line| mrc_crossing::LINE_LABELS[line as usize])
                                    .unwrap_or("na"),
                                event.barrier_price.unwrap_or(0.0),
                                event
                                    .target_line
                                    .map(|line| mrc_crossing::LINE_LABELS[line as usize])
                                    .unwrap_or("na"),
                                event.predicted_line_price.unwrap_or(0.0),
                                event
                                    .predicted_direction,
                                event.trade_direction,
                                event
                                    .take_profit_line
                                    .map(|line| mrc_crossing::LINE_LABELS[line as usize])
                                    .unwrap_or("na"),
                                event.target_price.unwrap_or(0.0),
                                event
                                    .stop_loss_line
                                    .map(|line| mrc_crossing::LINE_LABELS[line as usize])
                                    .unwrap_or("na"),
                                event.stop_price.unwrap_or(0.0),
                                if event.context_samples > 0 {
                                    "contextual"
                                } else {
                                    "broad-fallback"
                                },
                                event
                                    .effective_move_pct
                                    .map(|value| format!("{value:.4}%"))
                                    .unwrap_or_else(|| "na".to_string()),
                                event.volume_gain,
                                event.volume_gain_min,
                                event.volume_gain_max,
                                event.buy_sell_gain,
                            )
                        });
                let mut notes = vec![
                    "strategy=pattern-engine".to_string(),
                    format!("samples={}", mrc_observation.samples),
                    format!(
                        "selected_pattern={}",
                        mrc_observation
                            .selected_pattern
                            .map(|p| p.as_str())
                            .unwrap_or("none")
                    ),
                    format!(
                        "selected_score={:.4}",
                        mrc_observation.selected_score.unwrap_or(0.0)
                    ),
                ];
                if let Some(analysis_note) = analysis_note {
                    notes.push(analysis_note);
                }
                gate = crate::model::EntryGate::Open {
                    notes,
                    reason: crate::model::EntryOpenReason::Entry(reason),
                };
            } else {
                state.mrc_crossing_model.current_entry = None;
                gate = crate::model::EntryGate::Wait(crate::model::EntryWait {
                    reason: "mrc-invalid-exit-levels".to_string(),
                    solutions: Vec::new(),
                    notes: vec!["MRC target/stop collapse to the entry tick".to_string()],
                });
            }
        } else if let Some(wait_reason) = mrc_observation.wait_reason {
            gate = mrc_crossing::wait_entry_gate(wait_reason);
        }
        gate
    };
    let base_decision = crate::model::TradeDecision {
        market_state: entry_direction,
        entry: entry_gate,
    };
    let decision = base_decision;
    let execution = execution::evaluate_with_mrc(input, state, &snapshot, &decision);
    let debug = debug::render(
        state,
        &snapshot,
        &decision,
        execution
            .as_ref()
            .and_then(|result| result.constraint.as_deref()),
        input.symbol_info.price_tick_size,
    );
    let execution = execution.unwrap_or_default();

    let mut place_orders = execution.entry_orders;
    place_orders.extend(execution.tp_orders);
    ModuleOutput {
        open_positions: execution.open_positions,
        close_positions: execution.close_positions,
        update_positions: execution.update_positions,
        place_orders,
        cancel_orders: execution.cancel_orders,
        debug,
        ..Default::default()
    }
}
