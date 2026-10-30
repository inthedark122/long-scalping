use crate::detection::mrc_pattern_classification;
use crate::exit;
use crate::format::{fmt_duration_f64, fmt_duration_ms, fmt_price_tick, tick_size};
use crate::model::IndicatorSnapshot;
use crate::model::{EntryGate, EntryReason, EntryType, MarketState, PositionState, TradeDecision};
use crate::orders::{close, limit_open, market_open, update_position};
use crate::risk::{entry_budget, EntrySizing};
use crate::state::{sync_position_state, EngineState, PositionPlanSnapshot};
use lte_strategy_bridge::abi::{
    Direction, ModuleClosePosition, ModuleInput, ModuleOpenPosition, ModulePlaceOrder,
    ModulePositionSummary, ModuleUpdatePosition,
};

// Keep Default available for type compatibility only. Never construct an
// empty execution result with a default value; no action is represented by
// `Option::None`.
#[derive(Debug, Default)]
pub struct ExecutionResult {
    pub open_positions: Vec<ModuleOpenPosition>,
    pub entry_orders: Vec<ModulePlaceOrder>,
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
    sizing: EntrySizing,
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
                EntryReason::MrcCrossing {
                        price,
                        market_price,
                        mrc_min,
                        mrc_max,
                        target_price,
                        stop_price,
                        source_line,
                        target_line,
                        pattern_kind,
                        pattern,
                        pattern_elapsed_ms,
                        pattern_samples,
                        context_samples,
                        no_pullback_count,
                        reverted_line,
                        target_samples,
                        movement_samples,
                        next_line_crosses,
                        support_resistance_moves,
                        price_only_moves,
                        successful_moves,
                        failed_moves,
                        movement_success_rate,
                        expected_value_pct,
                        expected_price_low,
                        expected_price_high,
                        estimated_time_ms,
                        favorable_move_pct,
                        target_is_movement,
                    ..
                } => format!(
                    "{} pattern={} line_price={:.4} market_price={:.4} mrc_range={:.4}..{:.4} source_line={} target_reference_line={} target_source={} target_move=+{:.3}% target={:.4} stop={:.4} cycle={} context_samples={} no_pullback={} reverted_line={} elapsed={} transition_samples={} target_samples={} movement_samples={} next_line_crosses={} support_resistance={} price_only_moves={} movement_wins={} movement_losses={} movement_success_rate={:.3} expected_move_pct={:.3} expected_price={:.4}..{:.4} projection={} eta={}",
                    reason.label(),
                    pattern_kind,
                    price,
                    market_price,
                    mrc_min,
                    mrc_max,
                    crate::detection::mrc_crossing::LINE_LABELS[*source_line as usize],
                    crate::detection::mrc_crossing::LINE_LABELS[*target_line as usize],
                    if *target_is_movement { "entry-movement" } else { "mrc-line" },
                    favorable_move_pct,
                    target_price,
                    stop_price,
                    pattern
                        .iter()
                        .map(|line| crate::detection::mrc_crossing::LINE_LABELS[*line as usize])
                        .collect::<Vec<_>>()
                        .join(">"),
                    context_samples,
                    no_pullback_count,
                    reverted_line
                        .map(|line| crate::detection::mrc_crossing::LINE_LABELS[line as usize])
                        .unwrap_or("na"),
                    pattern_elapsed_ms
                        .iter()
                        .map(|value| fmt_duration_ms(*value))
                        .collect::<Vec<_>>()
                        .join(","),
                    pattern_samples,
                    target_samples,
                    movement_samples,
                    next_line_crosses,
                    support_resistance_moves,
                    price_only_moves,
                    successful_moves,
                    failed_moves,
                    movement_success_rate,
                    expected_value_pct,
                    expected_price_low,
                    expected_price_high,
                    crate::detection::mrc_crossing::price_projection(
                        crate::detection::mrc_crossing::reason_direction(&reason),
                        *mrc_min,
                        *mrc_max,
                        *expected_price_low,
                        *expected_price_high,
                    ),
                    fmt_duration_f64(*estimated_time_ms)
                ),
            })
            .unwrap_or_else(|| "none".to_string()),
        EntryGate::Wait(_) => "none".to_string(),
    };
    let reason_price = entry_reason(decision)
        .map(EntryReason::price)
        .unwrap_or(market_price);
    let (plan_handoff_tp, plan_handoff_sl) = state
        .pending_entry_plan
        .map(|plan| (plan.take_profit, plan.stop_loss))
        .unwrap_or((f64::NAN, f64::NAN));
    format!(
        "open {} because {} entry={} reason={} reason_price={} order_price={} market_price={} amount={:.4} cap_init={:.2}% trading_amount={:.2}% budget={:.4} cascade_dir={} cascade_phase={} cascade_speed={} cascade_potential={} cascade_adverse={} cascade_avg_time={} {} plan_handoff_tp={:.8} plan_handoff_sl={:.8}",
        side,
        entry_note,
        state.cfg.entry_type.as_str(),
        reason_note,
        fmt_price_tick(reason_price, tick_size),
        fmt_price_tick(order_price, tick_size),
        fmt_price_tick(market_price, tick_size),
        amount,
        state.cfg.cap_init,
        state.cfg.trading_amount,
        state.trade_budget,
        state.cascade_signal.dir,
        state.cascade_signal.phase.as_str(),
        state
            .cascade_signal
            .speed_pct_per_min
            .map(|value| format!("{value:.4}%/min"))
            .unwrap_or_else(|| "na".to_string()),
        state
            .cascade_signal
            .potential_move_pct
            .map(|value| format!("{value:.4}%"))
            .unwrap_or_else(|| "na".to_string()),
        state
            .cascade_signal
            .adverse_move_pct
            .map(|value| format!("{value:.4}%"))
            .unwrap_or_else(|| "na".to_string()),
        state
            .cascade_signal
            .average_duration_ms
            .map(fmt_duration_f64)
            .unwrap_or_else(|| "na".to_string()),
        sizing.diagnostics(),
        plan_handoff_tp,
        plan_handoff_sl
    )
}

fn pre_entry_exit_plan(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    reference_price: f64,
    side: PositionState,
) -> Option<exit::ExitPlan> {
    exit::entry_exit_plan(state, snapshot, reference_price, side)
}

fn active_position_result(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    side: PositionState,
) -> ExecutionResult {
    let (tp_orders, update_positions, cancel_orders) =
        exit::ensure_active_exit_order(input, state, snapshot, side);
    let close_requested =
        state.position_context.last_action == crate::state::PositionDecisionAction::Close;
    let direction = match side {
        PositionState::Long => Direction::Long,
        PositionState::Short => Direction::Short,
        PositionState::Flat => Direction::Both,
    };
    let close_positions = close_requested
        .then(|| {
            vec![close(
                direction,
                &close_reason(
                    &format!(
                        "position-controller-{}",
                        state.position_context.decision_reason
                    ),
                    state,
                    input.price,
                    tick_size(input),
                ),
            )]
        })
        .unwrap_or_default();
    ExecutionResult {
        tp_orders,
        update_positions,
        cancel_orders: if close_requested {
            let mut cancel_orders = cancel_orders;
            cancel_orders.extend(exit::all_tp_marks(direction));
            cancel_orders.sort();
            cancel_orders.dedup();
            cancel_orders
        } else {
            cancel_orders
        },
        close_positions,
        ..Default::default()
    }
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

fn remember_pending_entry(state: &mut EngineState, reason: Option<EntryReason>) {
    let plan = reason.as_ref().and_then(|reason| match reason {
        EntryReason::MrcCrossing {
            target_price,
            stop_price,
            ..
        } if target_price.is_finite()
            && stop_price.is_finite()
            && *target_price > 0.0
            && *stop_price > 0.0 =>
        {
            Some(PositionPlanSnapshot {
                take_profit: *target_price,
                stop_loss: *stop_price,
                tp_reliable: true,
            })
        }
        _ => None,
    });

    state.position.entry_reason = reason.clone();
    state.position_context.entry_reason = reason.clone();
    state.position_context.entry_anchor = plan;
    state.position_context.last_valid_plan = None;
    state.pending_entry_plan = plan;
    state.pending_entry_reason = reason;
}

fn order_entry_price(input: &ModuleInput, side: PositionState, reason_price: f64) -> f64 {
    let tick = tick_size(input);
    match side {
        PositionState::Long if input.price < reason_price => (input.price - 2.0 * tick).max(tick),
        PositionState::Short if input.price > reason_price => input.price + 2.0 * tick,
        _ => reason_price,
    }
}

fn pending_limit_entry_expired(
    input: &ModuleInput,
    state: &EngineState,
    side: PositionState,
) -> bool {
    if state.cfg.entry_type != EntryType::Limit {
        return false;
    }
    let Some(signal_bar) = state.pending_entry_bar else {
        return false;
    };
    if state.bar_1m <= signal_bar {
        return false;
    }
    pending_position(input, side)
        .map(|position| !has_active_position(Some(position)))
        .unwrap_or(false)
}

fn recyclable_reentry_block_reason(state: &EngineState, side: PositionState) -> Option<String> {
    if state.position_context.side != PositionState::Flat
        || state.position_context.lifetime != crate::state::PositionLifetime::RecyclableStairs
    {
        return None;
    }
    let direction = match side {
        PositionState::Long => 1,
        PositionState::Short => -1,
        PositionState::Flat => return None,
    };
    let now_ms = state.last_market_decision_time_ms;
    if state.position_context.cycle_direction != direction
        || state.position_context.cycle_is_valid(now_ms)
    {
        return None;
    }
    let same_cycle =
        state.market_mrc_analysis.as_ref().is_some_and(|analysis| {
            analysis.trade_direction == direction
                && matches!(
                analysis.pattern_kind,
                Some(
                    crate::detection::mrc_pattern_classification::MrcPatternKind::StairsUp
                        | crate::detection::mrc_pattern_classification::MrcPatternKind::StairsDown
                )
            ) && matches!(
                analysis.staircase_lifecycle,
                Some(
                    crate::detection::mrc_pattern_classification::StaircaseLifecycle::Progressing
                        | crate::detection::mrc_pattern_classification::StaircaseLifecycle::Pullback
                )
            ) && !state.position_context.cycle_pattern.is_empty()
                && analysis.stair_sequence == state.position_context.cycle_pattern
        });
    same_cycle.then(|| {
        format!(
            "reentry-blocked lifetime={} entries={} reason=cycle-limit",
            state.position_context.lifetime.as_str(),
            state.position_context.consecutive_entries
        )
    })
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
    if state.cfg.entry_type != EntryType::Limit {
        return None;
    }
    let direction = match side {
        PositionState::Long => Direction::Long,
        PositionState::Short => Direction::Short,
        PositionState::Flat => return None,
    };
    let reason_price = state
        .position
        .entry_reason
        .as_ref()
        .map(|reason| reason.price())
        .filter(|price| *price > 0.0)
        .unwrap_or(input.price);
    let desired_price = order_entry_price(input, side, reason_price);
    if (state.position.entry_price - desired_price).abs() <= f64::EPSILON {
        return None;
    }
    state.position.entry_price = desired_price;
    Some(update_position(direction, Some(desired_price)))
}

pub fn evaluate_with_mrc(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    decision: &TradeDecision,
) -> Option<ExecutionResult> {
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
        let mut cancel_orders = exit::all_tp_marks(Direction::Long);
        cancel_orders.extend(exit::all_tp_marks(Direction::Short));
        state.exit_progress.reset();
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
            cancel_orders,
            ..Default::default()
        });
    }

    let entry_side = match decision.market_state {
        MarketState::Long => PositionState::Long,
        MarketState::Short => PositionState::Short,
        MarketState::Sideways => PositionState::Flat,
    };
    let decision_is_open = matches!(&decision.entry, EntryGate::Open { .. });
    if let Some(side) = [PositionState::Long, PositionState::Short]
        .into_iter()
        .find(|side| pending_limit_entry_expired(input, state, *side))
    {
        let direction = match side {
            PositionState::Long => Direction::Long,
            PositionState::Short => Direction::Short,
            PositionState::Flat => Direction::Both,
        };
        let reason = close_reason(
            "limit-entry-expired-after-one-candle",
            state,
            input.price,
            tick_size(input),
        );
        state.mrc_crossing_model.current_entry = None;
        state.pending_entry_bar = None;
        // Cancel the expired order first. The host still reports the old
        // pending order in this evaluation, so opening a replacement in the
        // same result can associate the new MRC exit levels with a different
        // eventual fill. The next evaluation will process a fresh signal
        // after the cancellation has been applied.
        return Some(ExecutionResult {
            close_positions: vec![close(direction, &reason)],
            ..Default::default()
        });
    }

    let finish = |result: ExecutionResult| Some(result);

    if let Some(long_position) = input.positions.long.as_ref() {
        if !has_active_position(Some(long_position)) {
            if matches!(&decision.entry, EntryGate::Wait(_)) {
                return None;
            }
            let reason = entry_reason(decision).cloned();
            state.position.entry_reason = reason.clone();
            if matches!(
                state.position.entry_reason,
                Some(EntryReason::MrcCrossing { .. })
            ) {
                state.mrc_crossing_model.current_entry = None;
            }
            let update = created_position_update(input, state, PositionState::Long);
            if update.is_some() {
                remember_pending_entry(state, reason);
                state.pending_entry_bar =
                    (state.cfg.entry_type == EntryType::Limit).then_some(state.bar_1m);
            }
            return finish(ExecutionResult {
                update_positions: update.into_iter().collect(),
                ..Default::default()
            });
        } else {
            return finish(active_position_result(
                input,
                state,
                snapshot,
                PositionState::Long,
            ));
        }
    }

    if let Some(short_position) = input.positions.short.as_ref() {
        if !has_active_position(Some(short_position)) {
            if matches!(&decision.entry, EntryGate::Wait(_)) {
                return None;
            }
            let reason = entry_reason(decision).cloned();
            state.position.entry_reason = reason.clone();
            if matches!(
                state.position.entry_reason,
                Some(EntryReason::MrcCrossing { .. })
            ) {
                state.mrc_crossing_model.current_entry = None;
            }
            let update = created_position_update(input, state, PositionState::Short);
            if update.is_some() {
                remember_pending_entry(state, reason);
                state.pending_entry_bar =
                    (state.cfg.entry_type == EntryType::Limit).then_some(state.bar_1m);
            }
            return finish(ExecutionResult {
                update_positions: update.into_iter().collect(),
                ..Default::default()
            });
        } else {
            return finish(active_position_result(
                input,
                state,
                snapshot,
                PositionState::Short,
            ));
        }
    }

    let direction_allowed = matches!(
        (state.allowed_direction, entry_side),
        (Direction::Both, _)
            | (Direction::Long, PositionState::Long)
            | (Direction::Short, PositionState::Short)
    );
    if !direction_allowed {
        return finish(ExecutionResult {
            constraint: Some(format!(
                "entry-direction-guard:allowed={:?}:requested={:?}",
                state.allowed_direction, entry_side
            )),
            ..Default::default()
        });
    }

    let locked_direction = state.mrc_crossing_model.trade_direction_lock;
    let requested_direction = match entry_side {
        PositionState::Long => Some(1),
        PositionState::Short => Some(-1),
        PositionState::Flat => None,
    };
    if decision_is_open {
        if let (Some(locked_direction), Some(requested_direction)) =
            (locked_direction, requested_direction)
        {
            if locked_direction != requested_direction {
                return finish(ExecutionResult {
                    constraint: Some(format!(
                        "trade-direction-lock:locked={locked_direction}:requested={requested_direction}:rearm=same-direction"
                    )),
                    ..Default::default()
                });
            }
        }
        if let Some(reason) = recyclable_reentry_block_reason(state, entry_side) {
            return finish(ExecutionResult {
                constraint: Some(reason),
                ..Default::default()
            });
        }
    }

    if matches!(&decision.entry, EntryGate::Wait(_)) {
        return None;
    }

    let base_amount = entry_budget(input, state);
    let leverage = (input.leverage as f64).max(1.0);
    if base_amount <= 0.0 {
        return finish(ExecutionResult {
            constraint: Some("budget-too-small".to_string()),
            ..Default::default()
        });
    }
    if let Some(min_notional) = input.symbol_info.min_notional.filter(|value| *value > 0.0) {
        if base_amount * leverage < min_notional {
            return finish(ExecutionResult {
                constraint: Some("budget-too-small".to_string()),
                ..Default::default()
            });
        }
    }

    let reason = entry_reason(decision).cloned();
    let mrc_entry = matches!(reason.as_ref(), Some(EntryReason::MrcCrossing { .. }));
    if let Some(EntryReason::MrcCrossing {
        target_price,
        stop_price,
        pattern_kind,
        target_is_movement,
        ..
    }) = reason.as_ref()
    {
        let direction = match entry_side {
            PositionState::Long => 1,
            PositionState::Short => -1,
            PositionState::Flat => 0,
        };
        let entry_price = reason
            .as_ref()
            .map(EntryReason::price)
            .unwrap_or(input.price);
        let valid = if *target_is_movement {
            crate::detection::mrc_crossing::valid_movement_price_levels(
                direction,
                entry_price,
                *target_price,
                *stop_price,
                Some(input.symbol_info.price_tick_size),
            )
        } else {
            crate::detection::mrc_crossing::valid_price_levels_for_pattern(
                direction,
                entry_price,
                *target_price,
                *stop_price,
                Some(input.symbol_info.price_tick_size),
                mrc_pattern_classification::MrcPatternKind::from_str(pattern_kind),
            )
        };
        if !valid {
            return finish(ExecutionResult {
                constraint: Some("mrc-invalid-exit-levels".to_string()),
                ..Default::default()
            });
        }
    }
    let entry_price = entry_reason(decision)
        .map(|reason| reason.price())
        .unwrap_or(input.price);
    if decision_is_open {
        state.position.entry_reason = reason.clone();
    }
    if let Some(pending_position) =
        pending_position(input, entry_side).filter(|position| !has_active_position(Some(position)))
    {
        if !entry_price.is_finite() || entry_price <= 0.0 {
            return None;
        }
        if state.cfg.entry_type == EntryType::Limit {
            let order_price = if mrc_entry {
                entry_price
            } else {
                order_entry_price(input, entry_side, entry_price)
            };
            if (pending_position.entry_price - order_price).abs() > f64::EPSILON {
                let direction = match entry_side {
                    PositionState::Long => Direction::Long,
                    PositionState::Short => Direction::Short,
                    PositionState::Flat => Direction::Both,
                };
                if state.pending_entry_reason.is_none() || state.pending_entry_plan.is_none() {
                    let pending_reason = state
                        .pending_entry_reason
                        .clone()
                        .or_else(|| state.position.entry_reason.clone())
                        .or_else(|| state.position_context.entry_reason.clone());
                    if pending_reason.is_some() {
                        remember_pending_entry(state, pending_reason);
                    }
                }
                return finish(ExecutionResult {
                    update_positions: vec![update_position(direction, Some(order_price))],
                    ..Default::default()
                });
            }
        }
        return None;
    }

    state.position.entry_reason = reason.clone();
    if mrc_entry {
        state.mrc_crossing_model.current_entry = None;
    }
    state.pending_entry_bar = (state.cfg.entry_type == EntryType::Limit).then_some(state.bar_1m);

    let order_price = if mrc_entry {
        entry_price
    } else {
        order_entry_price(input, entry_side, entry_price)
    };
    let sizing_reference_price = if state.cfg.entry_type == EntryType::Market {
        input.price
    } else {
        order_price
    };
    let projected_stop_loss = if state.cfg.risk_target_percent() > 0.0 {
        pre_entry_exit_plan(state, snapshot, sizing_reference_price, entry_side)
            .map(|plan| plan.stop_loss.price())
    } else {
        None
    };
    let sizing = match crate::risk::size_entry(
        input,
        state,
        entry_side,
        base_amount,
        sizing_reference_price,
        projected_stop_loss,
    ) {
        Ok(sizing) => sizing,
        Err(constraint) => {
            return finish(ExecutionResult {
                constraint: Some(constraint.to_string()),
                ..Default::default()
            });
        }
    };
    let amount = sizing.amount;
    if amount <= 0.0 {
        return finish(ExecutionResult {
            constraint: Some("risk-adjusted-size-too-small".to_string()),
            ..Default::default()
        });
    }
    if let Some(min_notional) = input.symbol_info.min_notional.filter(|value| *value > 0.0) {
        if amount * leverage < min_notional {
            return finish(ExecutionResult {
                constraint: Some("risk-adjusted-below-min-notional".to_string()),
                ..Default::default()
            });
        }
    }
    remember_pending_entry(state, reason.clone());
    let (entry_take_profit, entry_stop_loss) = state
        .pending_entry_plan
        .filter(|plan| plan.valid_for(entry_side, sizing_reference_price))
        .map(|plan| (Some(plan.take_profit), Some(plan.stop_loss)))
        .unwrap_or((None, None));
    let result = match decision.market_state {
        MarketState::Long => ExecutionResult {
            open_positions: vec![if state.cfg.entry_type == EntryType::Market {
                market_open(
                    Direction::Long,
                    amount,
                    input.price,
                    entry_take_profit,
                    entry_stop_loss,
                    &open_note(
                        Direction::Long,
                        amount,
                        input.price,
                        input.price,
                        tick_size(input),
                        decision,
                        state,
                        sizing,
                    ),
                )
            } else {
                limit_open(
                    Direction::Long,
                    amount,
                    order_price,
                    entry_take_profit,
                    entry_stop_loss,
                    &open_note(
                        Direction::Long,
                        amount,
                        order_price,
                        input.price,
                        tick_size(input),
                        decision,
                        state,
                        sizing,
                    ),
                )
            }],
            ..Default::default()
        },
        MarketState::Short => ExecutionResult {
            open_positions: vec![if state.cfg.entry_type == EntryType::Market {
                market_open(
                    Direction::Short,
                    amount,
                    input.price,
                    entry_take_profit,
                    entry_stop_loss,
                    &open_note(
                        Direction::Short,
                        amount,
                        input.price,
                        input.price,
                        tick_size(input),
                        decision,
                        state,
                        sizing,
                    ),
                )
            } else {
                limit_open(
                    Direction::Short,
                    amount,
                    order_price,
                    entry_take_profit,
                    entry_stop_loss,
                    &open_note(
                        Direction::Short,
                        amount,
                        order_price,
                        input.price,
                        tick_size(input),
                        decision,
                        state,
                        sizing,
                    ),
                )
            }],
            ..Default::default()
        },
        _ => return None,
    };
    finish(result)
}
