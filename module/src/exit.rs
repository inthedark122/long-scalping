use crate::format::tick_size;
use crate::guard;
use crate::model::{EntryReason, ExitMode, ExitType, IndicatorSnapshot, PositionState};
use crate::orders::{tp_limit_order, tp_mark};
use crate::state::EngineState;
use crate::stop_loss;
use lte_strategy_bridge::abi::{Direction, ModuleInput, ModulePlaceOrder, ModuleUpdatePosition};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExitPlan {
    pub take_profit: TakeProfit,
    pub stop_loss: StopLoss,
    pub tp_reliable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitPlanContext {
    Entry,
    ActiveReprice,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StopLoss {
    Fixed { price: f64 },
}

impl StopLoss {
    pub fn price(self) -> f64 {
        match self {
            Self::Fixed { price } => price,
        }
    }

    pub fn reason(self) -> String {
        match self {
            Self::Fixed { .. } => "fixed".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TakeProfit {
    Fixed { price: f64 },
}

impl TakeProfit {
    pub fn price(self) -> f64 {
        match self {
            Self::Fixed { price } => price,
        }
    }

    pub fn source_label(self) -> &'static str {
        match self {
            Self::Fixed { .. } => "fixed",
        }
    }

    pub fn order_note(self) -> String {
        format!("tp:{}", self.source_label())
    }
}

fn position_qty(input: &ModuleInput, side: PositionState) -> f64 {
    let position = match side {
        PositionState::Long => input.positions.long.as_ref(),
        PositionState::Short => input.positions.short.as_ref(),
        PositionState::Flat => None,
    };
    position
        .map(|position| {
            if position.qty != 0.0 {
                position.qty.abs()
            } else if position.notional > 0.0 {
                (position.notional / input.price.max(f64::EPSILON)).max(0.0)
            } else {
                0.0
            }
        })
        .unwrap_or(0.0)
}

pub(crate) fn virtual_stop_loss_hit_with_activation(
    state: &EngineState,
    side: PositionState,
    price: f64,
) -> bool {
    stop_loss::virtual_stop_loss_hit(side, price, state.position.stop_loss)
}

fn record_tp_reprice(
    state: &mut EngineState,
    input: &ModuleInput,
    side: PositionState,
    previous_tp: f64,
    new_tp: f64,
    reason: &str,
    source: &str,
    virtual_sl_hit: bool,
    extra: String,
) {
    let Some(detected_at_ms) = input.sug_info.as_ref().map(|sug_info| sug_info.time as u64) else {
        return;
    };
    state.tp_reprice_history.observe(
        detected_at_ms,
        input.price,
        previous_tp,
        new_tp,
        reason.to_string(),
        source.to_string(),
        state.position.exit_mode,
        virtual_sl_hit,
        format!("side={:?}|{}", side, extra),
    );
}

fn tp_reprice_order_note(state: &EngineState, tick_size: f64, prefix: &str) -> String {
    format!(
        "{}|tp_reprice={}",
        prefix,
        state
            .tp_reprice_history
            .summary(tick_size)
            .unwrap_or_else(|| "none".to_string())
    )
}

pub(crate) fn set_stop_loss(state: &mut EngineState, stop_loss: StopLoss) {
    state.position.stop_loss = stop_loss.price();
    state.position.stop_loss_reason = stop_loss.reason();
}

fn position_exit_plan(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    reference_price: f64,
    side: PositionState,
    _context: ExitPlanContext,
) -> Option<ExitPlan> {
    let plan = match state.position.entry_reason.as_ref() {
        Some(reason @ EntryReason::MrcCrossing { .. }) => {
            let levels = if snapshot.tf1m.mrc.ordered() {
                crate::detection::mrc_crossing::exit_levels(
                    reason,
                    snapshot.tf1m.mrc.lines(),
                    reference_price,
                )
            } else {
                match reason {
                    EntryReason::MrcCrossing {
                        target_price,
                        stop_price,
                        ..
                    } if target_price.is_finite()
                        && stop_price.is_finite()
                        && *target_price > 0.0
                        && *stop_price > 0.0 =>
                    {
                        Some((*target_price, *stop_price))
                    }
                    _ => None,
                }
            };
            levels.map(|(target_price, stop_price)| ExitPlan {
                take_profit: TakeProfit::Fixed {
                    price: target_price,
                },
                stop_loss: StopLoss::Fixed { price: stop_price },
                tp_reliable: true,
            })
        }
        None => None,
    };
    plan.and_then(|plan| {
        guard::minimum_exit_distances::enforce_minimum_exit_distances(
            plan,
            reference_price,
            side,
            state.cfg.min_tp_percent(),
            state.cfg.min_sl_percent(),
        )
    })
}

type ActiveExitResult = (
    Vec<ModulePlaceOrder>,
    Vec<ModuleUpdatePosition>,
    Vec<String>,
);

const CASCADE_TP_WEIGHTS: [f64; 3] = [0.40, 0.30, 0.30];

fn round_qty(qty: f64, step: f64) -> f64 {
    if !qty.is_finite() || qty <= 0.0 {
        return 0.0;
    }
    if step.is_finite() && step > 0.0 {
        ((qty / step).round() * step).max(0.0)
    } else {
        qty
    }
}

fn tp_plan_qty(input: &ModuleInput, total_qty: f64) -> [f64; 3] {
    let total_qty = total_qty.max(0.0);
    let step = input.symbol_info.qty_step_size;
    let first = round_qty(total_qty * CASCADE_TP_WEIGHTS[0], step).min(total_qty);
    let second = round_qty(
        (total_qty - first).max(0.0) * CASCADE_TP_WEIGHTS[1]
            / (CASCADE_TP_WEIGHTS[1] + CASCADE_TP_WEIGHTS[2]),
        step,
    )
    .min((total_qty - first).max(0.0));
    let last = (total_qty - first - second).max(0.0);
    let mut plan = [first, second, last];

    // A leg below the exchange minimum cannot be placed. Merge it into the
    // next leg so the position is still fully allocated whenever possible.
    if let Some(minimum) = input.symbol_info.min_order_qty.filter(|value| *value > 0.0) {
        if plan[0] > 0.0 && plan[0] < minimum {
            plan[1] += plan[0];
            plan[0] = 0.0;
        }
        if plan[1] > 0.0 && plan[1] < minimum {
            plan[2] += plan[1];
            plan[1] = 0.0;
        }
        if plan[2] > 0.0 && plan[2] < minimum {
            if plan[1] >= minimum {
                plan[1] += plan[2];
                plan[2] = 0.0;
            } else if plan[0] >= minimum {
                plan[0] += plan[2];
                plan[2] = 0.0;
            }
        }
    }
    plan
}

fn live_tp_quantities(
    input: &ModuleInput,
    state: &mut EngineState,
    side: PositionState,
    live_qty: f64,
) -> Vec<(usize, f64)> {
    state.take_profit_allocation.begin(side, live_qty);
    if !state.take_profit_allocation.has_plan(side) {
        let total_qty = state.take_profit_allocation.total_qty.max(live_qty);
        state.take_profit_allocation.planned_qty = tp_plan_qty(input, total_qty);
    }

    let allocation = state.take_profit_allocation;
    let mut outstanding = [0.0; 3];
    for (index, planned) in allocation.planned_qty.iter().copied().enumerate() {
        outstanding[index] = (planned - allocation.filled_qty[index].max(0.0)).max(0.0);
    }
    let outstanding_total: f64 = outstanding.iter().sum();
    if outstanding_total <= 0.0 {
        return Vec::new();
    }

    // Order updates normally identify the filled leg. If one was missed or
    // the module was restarted before it arrived, cap the replacement orders
    // to the live position without guessing whether TP1 or TP2 filled.
    let live_qty = live_qty.max(0.0).min(outstanding_total);
    let scale = if live_qty + f64::EPSILON < outstanding_total {
        live_qty / outstanding_total
    } else {
        1.0
    };
    let candidates = outstanding.map(|qty| qty * scale);
    let active_indices: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, qty)| (*qty > f64::EPSILON).then_some(index))
        .collect();
    let Some(&last_index) = active_indices.last() else {
        return Vec::new();
    };

    let step = input.symbol_info.qty_step_size;
    let minimum = input.symbol_info.min_order_qty.filter(|value| *value > 0.0);
    let mut remaining = live_qty;
    let mut orders = Vec::with_capacity(active_indices.len());
    for index in active_indices {
        let candidate = candidates[index].min(remaining);
        let order_qty = if index == last_index {
            // The final active leg receives the actual remainder, including
            // any rounding residue from the earlier legs.
            round_qty(remaining, step).min(remaining)
        } else {
            round_qty(candidate, step).min(remaining)
        };
        if order_qty <= f64::EPSILON {
            continue;
        }
        if minimum.is_some_and(|minimum| order_qty < minimum) {
            // Carry a too-small non-final leg into the final leg. The final
            // leg is handled below and will receive this untouched remainder.
            continue;
        }
        remaining = (remaining - order_qty).max(0.0);
        orders.push((index, order_qty));
    }

    // If rounding left a tiny final remainder, add it to the last valid TP
    // rather than creating an exchange-rejected dust order.
    if remaining > f64::EPSILON {
        if let Some((_, last_qty)) = orders.last_mut() {
            let adjusted = round_qty(*last_qty + remaining, step);
            if minimum.is_none_or(|minimum| adjusted >= minimum) {
                *last_qty = adjusted;
            }
        }
    }
    orders
}

pub(crate) fn all_tp_marks(direction: Direction) -> Vec<String> {
    let base = tp_mark(direction);
    std::iter::once(base.clone())
        .chain((1..=3).map(move |index| format!("{base}-{index}")))
        .collect()
}

pub(crate) fn take_profit_orders(
    input: &ModuleInput,
    state: &mut EngineState,
    side: PositionState,
    qty: f64,
    entry_price: f64,
    target_price: f64,
    note: Option<&str>,
    multi: bool,
) -> (Vec<ModulePlaceOrder>, Vec<String>) {
    let direction = match side {
        PositionState::Long => Direction::Long,
        PositionState::Short => Direction::Short,
        PositionState::Flat => return (Vec::new(), Vec::new()),
    };
    if qty <= 0.0 || entry_price <= 0.0 || target_price <= 0.0 {
        return (Vec::new(), Vec::new());
    }
    let base_mark = tp_mark(direction);
    if !multi {
        let cancel_orders = if state.cascade_exit_orders {
            (1..=3)
                .map(|index| format!("{base_mark}-{index}"))
                .collect()
        } else {
            Vec::new()
        };
        state.cascade_exit_orders = false;
        return (
            vec![tp_limit_order(
                direction,
                qty,
                target_price,
                &base_mark,
                note,
            )],
            cancel_orders,
        );
    }

    let order_quantities = live_tp_quantities(input, state, side, qty);
    if order_quantities.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let tick = tick_size(input).max(f64::EPSILON);
    let mut prices = Vec::with_capacity(3);
    for fraction in [0.40, 0.70, 1.00] {
        let price = entry_price + (target_price - entry_price) * fraction;
        if price.is_finite()
            && price > 0.0
            && prices
                .last()
                .is_none_or(|last: &f64| (price - *last).abs() > tick * 0.5)
        {
            prices.push(price);
        }
    }
    if prices.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut orders = Vec::with_capacity(order_quantities.len());
    for (index, order_qty) in order_quantities {
        let Some(price) = prices.get(index).copied() else {
            continue;
        };
        let mark = format!("{base_mark}-{}", index + 1);
        orders.push(tp_limit_order(direction, order_qty, price, &mark, note));
    }
    if orders.is_empty() {
        return (Vec::new(), Vec::new());
    }
    state.cascade_exit_orders = true;
    let mut cancel_orders = vec![base_mark.clone()];
    for index in 0..3 {
        let mark = format!("{base_mark}-{}", index + 1);
        if !orders.iter().any(|order| order.mark == mark) {
            cancel_orders.push(mark);
        }
    }
    (orders, cancel_orders)
}

pub(crate) fn entry_exit_plan(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    reference_price: f64,
    side: PositionState,
) -> Option<ExitPlan> {
    position_exit_plan(
        state,
        snapshot,
        reference_price,
        side,
        ExitPlanContext::Entry,
    )
}

fn recovery_ticks(exit_type: ExitType) -> u32 {
    match exit_type {
        ExitType::LevelExit => 2,
        ExitType::VirtualTrailingExit { ticks } => ticks.max(1),
    }
}

fn ensure_trailing_exit_order(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    side: PositionState,
) -> ActiveExitResult {
    let direction = match side {
        PositionState::Long => Direction::Long,
        PositionState::Short => Direction::Short,
        PositionState::Flat => Direction::Both,
    };
    let mark = tp_mark(direction);
    let qty = position_qty(input, side);
    if qty <= 0.0 {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    if state.position.take_profit <= 0.0 || state.position.stop_loss <= 0.0 {
        let reference_price = if state.position.entry_price > 0.0 {
            state.position.entry_price
        } else {
            input.price
        };
        let Some(plan) = entry_exit_plan(state, snapshot, reference_price, side) else {
            return (Vec::new(), Vec::new(), Vec::new());
        };
        state.position.exit_mode = ExitMode::Fixed;
        state.position.take_profit = plan.take_profit.price();
        set_stop_loss(state, plan.stop_loss);
        state.position.exit_tp_reliable = plan.tp_reliable;
        let (tp_orders, cancel_orders) = take_profit_orders(
            input,
            state,
            side,
            qty,
            reference_price,
            plan.take_profit.price(),
            Some(&plan.take_profit.order_note()),
            state.cascade_signal.multi_order_ready(),
        );
        return (tp_orders, Vec::new(), cancel_orders);
    }

    if state.position.exit_mode == ExitMode::TrailAfterVirtualSl {
        let previous_tp = state.position.take_profit;
        let tick = tick_size(input);
        let Some(recovery_tp) = stop_loss::trailing_recovery_take_profit_with_ticks(
            input.price,
            side,
            tick,
            recovery_ticks(state.position.exit_type),
        ) else {
            return (Vec::new(), Vec::new(), Vec::new());
        };

        // Keep the same reduce-only order anchored to the latest market price.
        // The host upserts by mark, so returning this order on every changed
        // price amends the existing recovery order instead of leaving it at
        // the price from the initial virtual-SL crossing.
        if (recovery_tp - previous_tp).abs() <= f64::EPSILON {
            return (Vec::new(), Vec::new(), Vec::new());
        }
        state.position.take_profit = recovery_tp;
        record_tp_reprice(
            state,
            input,
            side,
            previous_tp,
            recovery_tp,
            "trail-reprice",
            state.position.exit_type.as_str(),
            true,
            format!(
                "mode={}|tp_mark={}|price={:.4}|sl={:.4}|tick={:.4}|ticks={}",
                state.position.exit_type.as_str(),
                mark,
                input.price,
                state.position.stop_loss,
                tick,
                recovery_ticks(state.position.exit_type)
            ),
        );
        let note = tp_reprice_order_note(state, tick, "tp:trail-reprice");
        let (tp_orders, cancel_orders) = take_profit_orders(
            input,
            state,
            side,
            qty,
            input.price,
            recovery_tp,
            Some(&note),
            false,
        );
        return (tp_orders, Vec::new(), cancel_orders);
    }

    let previous_stop_loss = state.position.stop_loss;
    if virtual_stop_loss_hit_with_activation(state, side, input.price) {
        let previous_tp = state.position.take_profit;
        let tick = tick_size(input);
        let Some(recovery_tp) = stop_loss::trailing_recovery_take_profit_with_ticks(
            input.price,
            side,
            tick,
            recovery_ticks(state.position.exit_type),
        ) else {
            return (Vec::new(), Vec::new(), Vec::new());
        };
        state.position.take_profit = recovery_tp;
        state.position.exit_mode = ExitMode::TrailAfterVirtualSl;
        record_tp_reprice(
            state,
            input,
            side,
            previous_tp,
            recovery_tp,
            "virtual-sl-cross",
            state.position.exit_type.as_str(),
            true,
            format!(
                "mode={}|tp_mark={}|price={:.4}|sl={:.4}|tick={:.4}|ticks={}",
                state.position.exit_type.as_str(),
                mark,
                input.price,
                previous_stop_loss,
                tick,
                recovery_ticks(state.position.exit_type)
            ),
        );
        let note = tp_reprice_order_note(state, tick, "tp:virtual-sl-cross");
        let (tp_orders, mut cancel_orders) = take_profit_orders(
            input,
            state,
            side,
            qty,
            input.price,
            recovery_tp,
            Some(&note),
            false,
        );
        cancel_orders.push(mark);
        return (tp_orders, Vec::new(), cancel_orders);
    }

    if state.position.exit_mode == ExitMode::Fixed {
        let multi = state.cascade_signal.multi_order_ready();
        if state.cascade_exit_orders != multi {
            let (tp_orders, cancel_orders) = take_profit_orders(
                input,
                state,
                side,
                qty,
                state.position.entry_price,
                state.position.take_profit,
                Some("tp:fixed"),
                multi,
            );
            return (tp_orders, Vec::new(), cancel_orders);
        }
        return (Vec::new(), Vec::new(), Vec::new());
    }

    // Recalculate the MRC plan on its normal cadence.
    let recalculate_virtual_sl = matches!(
        state.position.exit_type,
        ExitType::VirtualTrailingExit { .. }
    );
    let recalculate_tp = state.last_active_tp_recalc_bar != Some(state.bar_1m);

    // Build the active plan on every evaluation for VirtualTrailingExit. This
    // calculation must happen independently of the 1m TP reprice throttle.
    let active_plan = if recalculate_virtual_sl || recalculate_tp {
        position_exit_plan(
            state,
            snapshot,
            input.price,
            side,
            ExitPlanContext::ActiveReprice,
        )
    } else {
        None
    };

    if !recalculate_virtual_sl && !recalculate_tp {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    if recalculate_tp {
        state.last_active_tp_recalc_bar = Some(state.bar_1m);
    }

    if let Some(plan) = active_plan {
        let candidate_stop_loss = plan.stop_loss.price();
        let favorable = match side {
            PositionState::Long => candidate_stop_loss > previous_stop_loss,
            PositionState::Short => candidate_stop_loss < previous_stop_loss,
            PositionState::Flat => false,
        };
        if favorable {
            set_stop_loss(state, plan.stop_loss);
            state.position.exit_tp_reliable = plan.tp_reliable;
        }
        if recalculate_tp
            && (plan.take_profit.price() - state.position.take_profit).abs() > f64::EPSILON
        {
            let previous_tp = state.position.take_profit;
            state.position.take_profit = plan.take_profit.price();
            let tick = tick_size(input);
            record_tp_reprice(
                state,
                input,
                side,
                previous_tp,
                plan.take_profit.price(),
                "tp-reprice",
                plan.take_profit.source_label(),
                false,
                format!(
                    "tp_reliable={}|mode={}|price={:.4}|sl={:.4}|mark={}|tick={:.4}",
                    plan.tp_reliable,
                    state.position.exit_mode.as_str(),
                    input.price,
                    state.position.stop_loss,
                    mark,
                    tick
                ),
            );
            let note = tp_reprice_order_note(
                state,
                tick,
                &format!("{}|reprice", plan.take_profit.order_note()),
            );
            let (tp_orders, cancel_orders) = take_profit_orders(
                input,
                state,
                side,
                qty,
                state.position.entry_price,
                plan.take_profit.price(),
                Some(&note),
                state.cascade_signal.multi_order_ready(),
            );
            return (tp_orders, Vec::new(), cancel_orders);
        }
    }
    (Vec::new(), Vec::new(), Vec::new())
}

pub(crate) fn ensure_active_exit_order(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    side: PositionState,
) -> ActiveExitResult {
    ensure_trailing_exit_order(input, state, snapshot, side)
}

pub fn describe(plan: Option<ExitPlan>) -> String {
    match plan {
        Some(plan) => format!(
            "tp={:.4} sl={:.4} src={} tp_note={}",
            plan.take_profit.price(),
            plan.stop_loss.price(),
            plan.take_profit.source_label(),
            if plan.tp_reliable {
                "reliable"
            } else {
                "tight"
            }
        ),
        None => "tp=n/a sl=n/a".to_string(),
    }
}
