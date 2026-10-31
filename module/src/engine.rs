use crate::debug;
use crate::detection::{
    cascade, decision, mrc_crossing, mrc_direction_prediction, mrc_pattern_classification,
    mrc_prediction, vlines,
};
use crate::execution;
use crate::exit;
use crate::market_data;
use crate::model::{EntryType, ExitMode, MRC_CROSSING_EXIT_TYPE};
use crate::order_manager;
use crate::orders::{close, entry_order_marks, update_position_stop_loss};
use crate::risk;
use crate::state::{
    dir_to_position, direction_sign, sync_position_state, EngineState, PositionLifetime,
    PositionRuntime,
};
use lte_strategy_bridge::abi::{
    Direction, ModuleDataRequirements, ModuleEvent, ModuleInput, ModuleMarketDecision,
    ModuleOrderStatus, ModuleOutput, OrderRole,
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

fn seed_mrc_from_kline_history(state: &mut EngineState, now_ms: u64) {
    let Ok(history) = market_data::build_mrc_history(&state.klines) else {
        return;
    };
    mrc_crossing::seed_from_history_with_touch(
        &mut state.mrc_crossing_model,
        &mut state.vlines,
        &mut state.mrc_crossing,
        &history,
        now_ms,
        &mut state.mrc_touch,
    );
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
                market_data::KLINE_HISTORY_COUNT,
                market_data::KLINE_HISTORY_COUNT
            ),
            ..Default::default()
        },
        ModuleEvent::Init => {
            state.last_snapshot = None;
            state.klines.init(&input.klines);
            refresh_klines(input, state, &[TF_1M, TF_5M], true, true)
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
        state.mrc_touch.reset();
        state.cascade = crate::detection::cascade::CascadeState::default();
        state.cascade_signal = crate::detection::cascade::CascadeSignal::default();
        if state.position.side == crate::model::PositionState::Flat {
            state.active_mrc_analysis = None;
            state.active_mrc_context =
                crate::detection::mrc_prediction::MrcPredictionContext::default();
        }
        state.market_mrc_analysis = None;
        state.market_mrc_context =
            crate::detection::mrc_prediction::MrcPredictionContext::default();
        state.staircase_progress = crate::state::StaircaseProgress::default();
        state.exit_progress.reset();
        let now_ms = state.last_market_decision_time_ms;
        if now_ms > 0 {
            seed_mrc_from_kline_history(state, now_ms);
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
    let pending_entry_reason = state.pending_entry_reason.clone();
    let context_entry_reason = state.position_context.entry_reason.clone();
    let runtime_entry_reason = state.position.entry_reason.clone();
    let entry_reason = pending_entry_reason
        .clone()
        .or_else(|| context_entry_reason.clone())
        .or_else(|| runtime_entry_reason.clone());
    let input_pending_entry_plan = input
        .state
        .as_ref()
        .and_then(|saved| saved.get("pending_entry_plan"))
        .and_then(|saved| serde_json::from_value(saved.clone()).ok());
    let pending_entry_plan = state.pending_entry_plan.or(input_pending_entry_plan);
    let stored_entry_anchor = pending_entry_plan;
    let event_entry_plan = take_profit
        .zip(stop_loss)
        .map(
            |(take_profit, stop_loss)| crate::state::PositionPlanSnapshot {
                take_profit,
                stop_loss,
                tp_reliable: true,
            },
        )
        .filter(|plan| plan.valid_for(side, entry_price));
    let analysis_entry_plan = state
        .market_mrc_analysis
        .as_ref()
        .filter(|analysis| {
            analysis.trade_direction
                == match side {
                    crate::model::PositionState::Long => 1,
                    crate::model::PositionState::Short => -1,
                    crate::model::PositionState::Flat => 0,
                }
        })
        .and_then(|analysis| {
            let analysis_entry = analysis.effective_entry_price?;
            let tolerance = (input.symbol_info.price_tick_size * 2.0).max(entry_price.abs() * 1e-6);
            if !analysis_entry.is_finite() || (analysis_entry - entry_price).abs() > tolerance {
                return None;
            }
            Some(crate::state::PositionPlanSnapshot {
                take_profit: analysis.target_price?,
                stop_loss: analysis.stop_price?,
                tp_reliable: true,
            })
        })
        .filter(|plan| plan.valid_for(side, entry_price));
    let pending_entry_anchor = stored_entry_anchor
        .filter(|plan| plan.valid_for(side, entry_price))
        .or(event_entry_plan)
        .or_else(|| {
            state
                .position_context
                .entry_anchor
                .filter(|plan| plan.valid_for(side, entry_price))
        })
        .or(analysis_entry_plan);
    state.pending_entry_reason = None;
    state.pending_entry_plan = None;
    let allocation_qty = qty.abs().max(
        input
            .positions
            .long
            .as_ref()
            .map(|position| position.qty.abs())
            .or_else(|| {
                input
                    .positions
                    .short
                    .as_ref()
                    .map(|position| position.qty.abs())
            })
            .unwrap_or(0.0),
    );
    state.take_profit_allocation.begin(side, allocation_qty);
    let entry_time_ms = state.last_market_decision_time_ms;
    state.position_context.begin(
        side,
        entry_price,
        entry_time_ms,
        entry_reason.clone(),
        position_lifetime_for_entry(state, side),
        state
            .market_mrc_analysis
            .as_ref()
            .map(|analysis| analysis.stair_sequence.clone())
            .unwrap_or_default(),
    );
    state
        .position_context
        .seed_entry_anchor_from_reason(entry_reason.as_ref());
    if let Some(anchor) = pending_entry_anchor.filter(|anchor| anchor.valid_for(side, entry_price))
    {
        state.position_context.entry_anchor = Some(anchor);
    }
    state.exit_progress.begin(side, entry_time_ms, entry_price);
    state.position = PositionRuntime {
        side,
        entry_price,
        peak_price: input.price.max(entry_price),
        trough_price: input.price.min(entry_price),
        take_profit: 0.0,
        stop_loss: 0.0,
        stop_loss_reason: String::new(),
        exit_tp_reliable: false,
        exit_mode: ExitMode::Fixed,
        exit_type: MRC_CROSSING_EXIT_TYPE,
        entry_reason: entry_reason.clone(),
    };
    state.mrc_crossing_model.current_entry = None;
    state.pending_entry_bar = None;

    let snapshot = match state.last_snapshot.clone() {
        Some(snapshot) => snapshot,
        None => {
            let has_entry_plan = state.position.entry_reason.as_ref().is_some_and(|reason| {
                matches!(
                    reason,
                    crate::model::EntryReason::MrcCrossing { .. }
                        | crate::model::EntryReason::CandleTrend { .. }
                )
            });
            if has_entry_plan {
                crate::model::IndicatorSnapshot::default()
            } else {
                let error = format!(
                    "event=NewPosition missing-cached-snapshot {}",
                    market_data::render_kline_history_inventory(&state.klines)
                );
                state.last_error = Some(error.clone());
                return ModuleOutput {
                    debug: error,
                    ..Default::default()
                };
            }
        }
    };
    let plan =
        exit::resolve_position_exit_plan(state, &snapshot, entry_price, side).map(|(plan, _)| plan);
    let mut tp_orders = Vec::new();
    let mut cancel_orders = entry_order_marks();
    let mut stop_loss_updates = Vec::new();
    if let Some(plan) = plan {
        state.position_context.set_decision(
            crate::state::PositionDecisionAction::UpdateExit,
            "entry-exit-plan-ready",
        );
        state.position.exit_mode = ExitMode::Fixed;
        state.position.take_profit = plan.take_profit.price();
        exit::set_stop_loss(state, plan.stop_loss);
        if matches!(
            entry_reason.as_ref(),
            Some(
                crate::model::EntryReason::MrcCrossing { .. }
                    | crate::model::EntryReason::CandleTrend { .. }
            )
        ) {
            state.position_context.entry_anchor = Some(crate::state::PositionPlanSnapshot {
                take_profit: plan.take_profit.price(),
                stop_loss: plan.stop_loss.price(),
                tp_reliable: plan.tp_reliable,
            });
            stop_loss_updates.push(update_position_stop_loss(direction, plan.stop_loss.price()));
        }
        let qty = input
            .positions
            .long
            .as_ref()
            .map(|p| p.qty)
            .or_else(|| input.positions.short.as_ref().map(|p| p.qty))
            .unwrap_or(0.0);
        if qty > 0.0 {
            let note = plan.take_profit.order_note();
            let (orders, tp_cancel_orders) = exit::take_profit_orders(
                input,
                state,
                side,
                qty,
                entry_price,
                plan.take_profit.price(),
                Some(&note),
                exit::far_target_for_position(
                    state,
                    &snapshot,
                    entry_price,
                    plan.take_profit.price(),
                ),
            );
            tp_orders = orders;
            cancel_orders.extend(tp_cancel_orders);
        }
    }

    let plan_result = if plan.is_some() {
        format!(
            "new-position-plan=ready live_qty={:.4} tp_orders={} cancel_orders={}",
            input
                .positions
                .long
                .as_ref()
                .map(|position| position.qty)
                .or_else(|| input.positions.short.as_ref().map(|position| position.qty))
                .unwrap_or(0.0),
            tp_orders.len(),
            cancel_orders.len()
        )
    } else if state.position.entry_reason.is_none() {
        state.position_context.set_decision(
            crate::state::PositionDecisionAction::Close,
            "emergency-no-entry-reason",
        );
        "new-position-plan=none reason=missing-entry-reason".to_string()
    } else {
        state.position_context.set_decision(
            crate::state::PositionDecisionAction::Close,
            "emergency-no-exit-plan",
        );
        "new-position-plan=none reason=no-valid-exit-plan".to_string()
    };
    exit::record_exit_event(state, plan_result);

    let emergency_close = plan.is_none()
        && state.position_context.last_plan_source
            == crate::state::PositionExitPlanSource::EmergencyClose;
    let emergency_note = if emergency_close {
        let levels = |reason: Option<&crate::model::EntryReason>| {
            reason.map(|reason| match reason {
                crate::model::EntryReason::MrcCrossing {
                    target_price,
                    stop_price,
                    ..
                }
                | crate::model::EntryReason::CandleTrend {
                    target_price,
                    stop_price,
                    ..
                } => (*target_price, *stop_price),
            })
        };
        let (pending_tp, pending_sl) =
            levels(pending_entry_reason.as_ref()).unwrap_or((f64::NAN, f64::NAN));
        let (context_tp, context_sl) =
            levels(context_entry_reason.as_ref()).unwrap_or((f64::NAN, f64::NAN));
        let (runtime_tp, runtime_sl) =
            levels(runtime_entry_reason.as_ref()).unwrap_or((f64::NAN, f64::NAN));
        let pending_plan_tp = pending_entry_plan
            .map(|plan| plan.take_profit)
            .unwrap_or(f64::NAN);
        let pending_plan_sl = pending_entry_plan
            .map(|plan| plan.stop_loss)
            .unwrap_or(f64::NAN);
        let analysis_plan_tp = analysis_entry_plan
            .map(|plan| plan.take_profit)
            .unwrap_or(f64::NAN);
        let analysis_plan_sl = analysis_entry_plan
            .map(|plan| plan.stop_loss)
            .unwrap_or(f64::NAN);
        let anchor = state.position_context.entry_anchor;
        let anchor_tp = anchor.map(|anchor| anchor.take_profit).unwrap_or(f64::NAN);
        let anchor_sl = anchor.map(|anchor| anchor.stop_loss).unwrap_or(f64::NAN);
        let anchor_valid = anchor.is_some_and(|anchor| anchor.valid_for(side, entry_price));
        format!(
            "close because position-controller-emergency-no-exit-plan side={:?} entry={:.8} event_tp={:.8} event_sl={:.8} pending={} pending_tp={:.8} pending_sl={:.8} pending_plan_tp={:.8} pending_plan_sl={:.8} analysis_plan_tp={:.8} analysis_plan_sl={:.8} context={} context_tp={:.8} context_sl={:.8} runtime={} runtime_tp={:.8} runtime_sl={:.8} anchor_tp={:.8} anchor_sl={:.8} anchor_valid={} mrc_ordered={}",
            side,
            entry_price,
            take_profit.unwrap_or(f64::NAN),
            stop_loss.unwrap_or(f64::NAN),
            pending_entry_reason.is_some(),
            pending_tp,
            pending_sl,
            pending_plan_tp,
            pending_plan_sl,
            analysis_plan_tp,
            analysis_plan_sl,
            context_entry_reason.is_some(),
            context_tp,
            context_sl,
            runtime_entry_reason.is_some(),
            runtime_tp,
            runtime_sl,
            anchor_tp,
            anchor_sl,
            anchor_valid,
            snapshot.tf1m.mrc.ordered()
        )
    } else {
        String::new()
    };

    ModuleOutput {
        close_positions: emergency_close
            .then(|| vec![close(direction, &emergency_note)])
            .unwrap_or_default(),
        place_orders: tp_orders,
        cancel_orders,
        update_positions: stop_loss_updates,
        debug: format!(
            "new-position {:?} {}",
            direction,
            plan.map(|plan| exit::describe(Some(plan)))
                .unwrap_or_else(|| "tp=n/a sl=n/a".to_string())
        ) + " "
            + &state.last_exit_event,
        ..Default::default()
    }
}

fn position_lifetime_for_entry(
    state: &EngineState,
    side: crate::model::PositionState,
) -> PositionLifetime {
    let direction = match side {
        crate::model::PositionState::Long => 1,
        crate::model::PositionState::Short => -1,
        crate::model::PositionState::Flat => 0,
    };
    let recyclable = state.market_mrc_analysis.as_ref().is_some_and(|analysis| {
        analysis.trade_direction == direction
            && matches!(
                analysis.pattern_kind,
                Some(
                    mrc_pattern_classification::MrcPatternKind::StairsUp
                        | mrc_pattern_classification::MrcPatternKind::StairsDown
                )
            )
            && matches!(
                analysis.staircase_lifecycle,
                Some(
                    mrc_pattern_classification::StaircaseLifecycle::Progressing
                        | mrc_pattern_classification::StaircaseLifecycle::Pullback
                )
            )
            && analysis.stair_entry_approved
    });
    if recyclable {
        PositionLifetime::RecyclableStairs
    } else {
        PositionLifetime::OneShot
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
            positions: ModulePositions {
                long: None,
                short: None,
            },
            market_decision: None,
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

fn refresh_klines(
    input: &ModuleInput,
    state: &mut EngineState,
    timeframes: &[i32],
    is_init: bool,
    new_1m_bar: bool,
) -> ModuleOutput {
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

    let snapshot = match market_data::build_indicator_snapshot(&state.klines) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let error = format!(
                "event={} {} {} {}",
                if is_init { "Init" } else { "KlineUpdate" },
                timeframes_context(timeframes),
                error,
                market_data::render_kline_history_inventory(&state.klines)
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
                "event=Init ta-rs snapshot ready {}",
                market_data::render_kline_history_inventory(&state.klines)
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

    let now_ms = market_decision.time.max(0) as u64;
    if state.position.side != crate::model::PositionState::Flat
        && state.exit_progress.side != state.position.side
    {
        state
            .exit_progress
            .begin(state.position.side, now_ms, state.position.entry_price);
    }

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
    state
        .exit_progress
        .observe_activity(market_decision.ntp_5s, snapshot.tf1m.volume_gain);

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

    let entry_selection = crate::entry_policy::evaluate(
        input,
        state,
        &snapshot,
        market_decision,
        &mrc_observation,
        prediction_context,
    );
    let decision = entry_selection.decision;
    let execution = execution::evaluate_with_mrc(input, state, &snapshot, &decision);
    let debug = debug::render(
        state,
        &snapshot,
        &entry_selection.debug,
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
