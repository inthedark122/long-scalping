use crate::detection::{cascade, levels};
use crate::format::tick_size;
use crate::model::{ExitMode, ExitType, IndicatorSnapshot, PositionState, SugInfoSnapshot};
use crate::orders::{tp_limit_order, tp_mark};
use crate::state::{CascadeState, EngineState};
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
    Candidate,
    ActiveReprice,
}

impl ExitPlanContext {
    fn enforces_cascade_minimum(self) -> bool {
        matches!(self, Self::Entry)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StopLoss {
    Structural { level: levels::LevelPoint },
    Sideway { price: f64 },
    SupportBreak { price: f64 },
    MrcCrossing { price: f64 },
}

impl StopLoss {
    pub fn price(self) -> f64 {
        match self {
            Self::Structural { level } => level.value,
            Self::Sideway { price } | Self::SupportBreak { price } => price,
            Self::MrcCrossing { price } => price,
        }
    }

    pub fn structural_level(self) -> Option<levels::LevelPoint> {
        match self {
            Self::Structural { level } => Some(level),
            _ => None,
        }
    }

    pub fn reason(self) -> String {
        match self {
            Self::Structural { level } => format!("structural:{}", level.label()),
            Self::Sideway { .. } => "sideway:block-edge".to_string(),
            Self::SupportBreak { .. } => "support-break".to_string(),
            Self::MrcCrossing { .. } => "mrc-crossing:adjacent-line".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TakeProfit {
    Structural { level: levels::LevelPoint },
    Movement { price: f64 },
    Sideway { price: f64 },
    MrcCrossing { price: f64 },
}

impl TakeProfit {
    pub fn price(self) -> f64 {
        match self {
            Self::Structural { level } => level.value,
            Self::Movement { price } | Self::Sideway { price } | Self::MrcCrossing { price } => {
                price
            }
        }
    }

    pub fn source_label(self) -> &'static str {
        match self {
            Self::Structural { .. } => "structural",
            Self::Movement { .. } => "movement",
            Self::Sideway { .. } => "sideway",
            Self::MrcCrossing { .. } => "mrc-crossing",
        }
    }

    pub fn order_note(self) -> String {
        match self {
            Self::Structural { level } => format!("tp:structural|level={}", level.label()),
            take_profit => format!("tp:{}", take_profit.source_label()),
        }
    }
}

pub fn mrc_crossing_plan(target_price: f64, stop_price: f64) -> Option<ExitPlan> {
    if !target_price.is_finite()
        || !stop_price.is_finite()
        || target_price <= 0.0
        || stop_price <= 0.0
    {
        return None;
    }
    Some(ExitPlan {
        take_profit: TakeProfit::MrcCrossing {
            price: target_price,
        },
        stop_loss: StopLoss::MrcCrossing { price: stop_price },
        tp_reliable: true,
    })
}

fn structural_plan(
    levels: &crate::detection::levels::LevelBook,
    cascade: &CascadeState,
    price: f64,
    side: PositionState,
    context: ExitPlanContext,
    required_samples: u32,
    excluded_stop_losses: &[levels::LevelPoint],
    configured_minimum_percent: f64,
) -> Option<(levels::LevelPoint, levels::LevelPoint)> {
    let take_profit = structural_take_profit_level(
        levels,
        cascade,
        price,
        side,
        context,
        required_samples,
        configured_minimum_percent,
    )?;
    match side {
        PositionState::Long => {
            let sl = structural_stop_loss_level(levels, price, side, excluded_stop_losses)?;
            Some((take_profit, sl))
        }
        PositionState::Short => {
            let sl = structural_stop_loss_level(levels, price, side, excluded_stop_losses)?;
            Some((take_profit, sl))
        }
        PositionState::Flat => None,
    }
}

fn structural_stop_loss_level(
    levels: &levels::LevelBook,
    price: f64,
    side: PositionState,
    excluded: &[levels::LevelPoint],
) -> Option<levels::LevelPoint> {
    let excluded_mrc_kind = |kind| match side {
        PositionState::Long => matches!(
            kind,
            levels::LevelKind::MrcMean
                | levels::LevelKind::MrcDownSmall
                | levels::LevelKind::MrcDownInner
                | levels::LevelKind::MrcDownOuter
        ),
        PositionState::Short => matches!(
            kind,
            levels::LevelKind::MrcMean
                | levels::LevelKind::MrcUpSmall
                | levels::LevelKind::MrcUpInner
                | levels::LevelKind::MrcUpOuter
        ),
        PositionState::Flat => true,
    };

    match side {
        PositionState::Long => levels
            .below
            .iter()
            .copied()
            .filter(|level| {
                level.value < price && !excluded.contains(level) && !excluded_mrc_kind(level.kind)
            })
            .max_by(levels::LevelBook::compare_points),
        PositionState::Short => levels
            .above
            .iter()
            .copied()
            .filter(|level| {
                level.value > price && !excluded.contains(level) && !excluded_mrc_kind(level.kind)
            })
            .min_by(levels::LevelBook::compare_points),
        PositionState::Flat => None,
    }
}

fn cascade_average_percent(
    cascade: &CascadeState,
    price: f64,
    required_samples: u32,
) -> Option<f64> {
    if cascade.samples < required_samples {
        return None;
    }
    cascade::average_size_percent(cascade.avg_size, price)
}

fn effective_minimum_percent(
    cascade: &CascadeState,
    price: f64,
    required_samples: u32,
    configured_minimum_percent: f64,
) -> Option<f64> {
    let configured_minimum_percent = configured_minimum_percent.max(0.0);
    match cascade_average_percent(cascade, price, required_samples) {
        Some(cascade_minimum) => Some(cascade_minimum.max(configured_minimum_percent)),
        None if configured_minimum_percent > 0.0 => Some(configured_minimum_percent),
        None => None,
    }
}

fn structural_take_profit_level(
    levels: &crate::detection::levels::LevelBook,
    cascade: &CascadeState,
    price: f64,
    side: PositionState,
    context: ExitPlanContext,
    required_samples: u32,
    configured_minimum_percent: f64,
) -> Option<levels::LevelPoint> {
    let minimum_percent = if context.enforces_cascade_minimum() {
        effective_minimum_percent(cascade, price, required_samples, configured_minimum_percent)
    } else {
        None
    };
    let qualifies = |level: &levels::LevelPoint| {
        if matches!(level.kind, levels::LevelKind::MrcMean) {
            return false;
        }
        let distance_percent = match side {
            PositionState::Long => (level.value - price) / price * 100.0,
            PositionState::Short => (price - level.value) / price * 100.0,
            PositionState::Flat => return false,
        };
        distance_percent.is_finite()
            && distance_percent > 0.0
            && minimum_percent.is_none_or(|minimum| distance_percent >= minimum)
    };

    match side {
        PositionState::Long => levels
            .above
            .iter()
            .copied()
            .filter(|level| level.value > price)
            .find(qualifies),
        PositionState::Short => levels
            .below
            .iter()
            .copied()
            .filter(|level| level.value < price)
            .rev()
            .find(qualifies),
        PositionState::Flat => None,
    }
}

fn take_profit_meets_cascade_minimum(
    cascade: &CascadeState,
    price: f64,
    side: PositionState,
    take_profit: f64,
    context: ExitPlanContext,
    required_samples: u32,
    configured_minimum_percent: f64,
) -> bool {
    if !context.enforces_cascade_minimum() {
        return true;
    }
    let Some(minimum_percent) =
        effective_minimum_percent(cascade, price, required_samples, configured_minimum_percent)
    else {
        return true;
    };
    let distance_percent = match side {
        PositionState::Long => (take_profit - price) / price * 100.0,
        PositionState::Short => (price - take_profit) / price * 100.0,
        PositionState::Flat => return false,
    };
    distance_percent.is_finite() && distance_percent >= minimum_percent
}

fn clamp_take_profit(
    price: f64,
    side: PositionState,
    raw_tp: f64,
    structural_take_profit: f64,
) -> f64 {
    if !raw_tp.is_finite() || raw_tp <= 0.0 {
        return 0.0;
    }

    let structural_valid = structural_take_profit.is_finite() && structural_take_profit > 0.0;
    match side {
        PositionState::Long => {
            if structural_valid && structural_take_profit > price {
                raw_tp.min(structural_take_profit)
            } else {
                raw_tp
            }
        }
        PositionState::Short => {
            if structural_valid && structural_take_profit < price {
                raw_tp.max(structural_take_profit)
            } else {
                raw_tp
            }
        }
        PositionState::Flat => raw_tp,
    }
}

fn movement_plan(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    cascade: &CascadeState,
    price: f64,
    side: PositionState,
    structural: Option<(levels::LevelPoint, levels::LevelPoint)>,
    required_samples: u32,
) -> Option<ExitPlan> {
    let tp_distance =
        stop_loss::projected_move_distance(snapshot, sug_snapshot, cascade, required_samples)?;
    if !price.is_finite() || price <= 0.0 {
        return None;
    }

    let (structural_take_profit_level, structural_stop_loss_level) = structural?;
    let structural_take_profit = structural_take_profit_level.value;
    let stop_loss = stop_loss::support_break_level(snapshot, side, None, price)
        .map(|price| StopLoss::SupportBreak {
            price: price.max(f64::EPSILON),
        })
        .unwrap_or(StopLoss::Structural {
            level: structural_stop_loss_level,
        });

    let plan = match side {
        PositionState::Long => ExitPlan {
            take_profit: TakeProfit::Movement {
                price: clamp_take_profit(price, side, price + tp_distance, structural_take_profit),
            },
            stop_loss,
            tp_reliable: true,
        },
        PositionState::Short => ExitPlan {
            take_profit: TakeProfit::Movement {
                price: clamp_take_profit(
                    price,
                    side,
                    (price - tp_distance).max(f64::EPSILON),
                    structural_take_profit,
                ),
            },
            stop_loss,
            tp_reliable: true,
        },
        PositionState::Flat => return None,
    };

    if plan.take_profit.price().is_finite()
        && plan.stop_loss.price().is_finite()
        && plan.take_profit.price() > 0.0
        && plan.stop_loss.price() > 0.0
    {
        Some(plan)
    } else {
        None
    }
}

pub fn compute(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    cascade: &CascadeState,
    price: f64,
    side: PositionState,
    context: ExitPlanContext,
    required_samples: u32,
) -> Option<ExitPlan> {
    compute_with_excluded_stop_losses(
        snapshot,
        sug_snapshot,
        cascade,
        price,
        side,
        context,
        required_samples,
        &[],
    )
}

pub fn compute_with_excluded_stop_losses(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    cascade: &CascadeState,
    price: f64,
    side: PositionState,
    context: ExitPlanContext,
    required_samples: u32,
    excluded_stop_losses: &[levels::LevelPoint],
) -> Option<ExitPlan> {
    compute_with_excluded_stop_losses_and_minimum(
        snapshot,
        sug_snapshot,
        cascade,
        price,
        side,
        context,
        required_samples,
        excluded_stop_losses,
        0.0,
    )
}

pub fn compute_with_excluded_stop_losses_and_minimum(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    cascade: &CascadeState,
    price: f64,
    side: PositionState,
    context: ExitPlanContext,
    required_samples: u32,
    excluded_stop_losses: &[levels::LevelPoint],
    configured_minimum_percent: f64,
) -> Option<ExitPlan> {
    if !price.is_finite() || price <= 0.0 {
        return None;
    }

    let levels = levels::structural_levels(snapshot);
    let structural = structural_plan(
        &levels,
        cascade,
        price,
        side,
        context,
        required_samples,
        excluded_stop_losses,
        configured_minimum_percent,
    );

    if let Some(plan) = movement_plan(
        snapshot,
        sug_snapshot,
        cascade,
        price,
        side,
        structural,
        required_samples,
    ) {
        if take_profit_meets_cascade_minimum(
            cascade,
            price,
            side,
            plan.take_profit.price(),
            context,
            required_samples,
            configured_minimum_percent,
        ) {
            return Some(plan);
        }
    }

    match side {
        PositionState::Long => {
            let Some(raw_tp) = structural_take_profit_level(
                &levels,
                cascade,
                price,
                side,
                context,
                required_samples,
                configured_minimum_percent,
            ) else {
                return None;
            };
            let Some(raw_sl) =
                structural_stop_loss_level(&levels, price, side, excluded_stop_losses)
            else {
                return None;
            };

            Some(ExitPlan {
                take_profit: TakeProfit::Structural { level: raw_tp },
                stop_loss: StopLoss::Structural { level: raw_sl },
                tp_reliable: true,
            })
        }
        PositionState::Short => {
            let Some(raw_tp) = structural_take_profit_level(
                &levels,
                cascade,
                price,
                side,
                context,
                required_samples,
                configured_minimum_percent,
            ) else {
                return None;
            };
            let Some(raw_sl) =
                structural_stop_loss_level(&levels, price, side, excluded_stop_losses)
            else {
                return None;
            };

            Some(ExitPlan {
                take_profit: TakeProfit::Structural { level: raw_tp },
                stop_loss: StopLoss::Structural { level: raw_sl },
                tp_reliable: true,
            })
        }
        PositionState::Flat => None,
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

fn structural_stop_loss_note(state: &EngineState) -> String {
    let active = state
        .structural_stop_loss
        .map(|level| format!("{}@{:.4}", level.label(), level.value))
        .unwrap_or_else(|| "na".to_string());
    let processed = if state.processed_structural_stop_losses.is_empty() {
        "na".to_string()
    } else {
        state
            .processed_structural_stop_losses
            .iter()
            .map(|level| format!("{}@{:.4}", level.label(), level.value))
            .collect::<Vec<_>>()
            .join(",")
    };
    format!("active={active}|processed={processed}")
}

pub(crate) fn set_stop_loss(state: &mut EngineState, stop_loss: StopLoss) {
    state.position.stop_loss = stop_loss.price();
    state.position.stop_loss_reason = stop_loss.reason();
    state.structural_stop_loss = stop_loss.structural_level();
    if let Some(level) = state.structural_stop_loss {
        if !state.processed_structural_stop_losses.contains(&level) {
            state.processed_structural_stop_losses.push(level);
        }
    }
}

fn position_exit_plan(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    reference_price: f64,
    side: PositionState,
    context: ExitPlanContext,
) -> Option<ExitPlan> {
    compute_with_excluded_stop_losses_and_minimum(
        snapshot,
        sug_snapshot,
        &state.cascade_tf,
        reference_price,
        side,
        context,
        state.cfg.required_samples(),
        &state.processed_structural_stop_losses,
        state.cfg.min_tp_percent(),
    )
}

type ActiveExitResult = (
    Vec<ModulePlaceOrder>,
    Vec<ModuleUpdatePosition>,
    Vec<String>,
);

fn entry_exit_plan(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    reference_price: f64,
    side: PositionState,
) -> Option<ExitPlan> {
    if let Some(crate::model::EntryReason::MrcCrossing {
        target_price,
        stop_price,
        ..
    }) = state.position.entry_reason.as_ref()
    {
        mrc_crossing_plan(*target_price, *stop_price)
    } else {
        position_exit_plan(
            state,
            snapshot,
            sug_snapshot,
            reference_price,
            side,
            ExitPlanContext::Entry,
        )
    }
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
    sug_snapshot: &SugInfoSnapshot,
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
        let Some(plan) = entry_exit_plan(state, snapshot, sug_snapshot, reference_price, side)
        else {
            return (Vec::new(), Vec::new(), Vec::new());
        };
        state.position.take_profit = plan.take_profit.price();
        set_stop_loss(state, plan.stop_loss);
        state.position.exit_tp_reliable = plan.tp_reliable;
        return (
            vec![tp_limit_order(
                direction,
                qty,
                plan.take_profit.price(),
                &mark,
                Some(&plan.take_profit.order_note()),
            )],
            Vec::new(),
            Vec::new(),
        );
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
        return (
            vec![tp_limit_order(
                direction,
                qty,
                recovery_tp,
                &mark,
                Some(&note),
            )],
            Vec::new(),
            Vec::new(),
        );
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
        return (
            vec![tp_limit_order(
                direction,
                qty,
                recovery_tp,
                &mark,
                Some(&note),
            )],
            Vec::new(),
            vec![mark],
        );
    }

    let preserve_mrc_crossing_exit = matches!(
        state.position.entry_reason,
        Some(crate::model::EntryReason::MrcCrossing { .. })
    );

    // VirtualTrailingExit must refresh its virtual SL on every live
    // evaluation, except for MRC crossing positions, whose learned entry
    // target and adjacent-line stop are fixed for the position lifetime.
    // The TP for other strategies remains on its existing 1m cadence, but
    // the stop needs the current price/indicator state to trail the position
    // before a fast move can cross the stale level.
    let recalculate_virtual_sl = matches!(
        state.position.exit_type,
        ExitType::VirtualTrailingExit { .. }
    ) && !preserve_mrc_crossing_exit;
    let recalculate_tp = state.last_active_tp_recalc_bar != Some(state.bar_1m);

    // Build the active plan on every evaluation for VirtualTrailingExit. This
    // calculation must happen independently of the 1m TP reprice throttle.
    let active_plan = if recalculate_virtual_sl || (recalculate_tp && !preserve_mrc_crossing_exit) {
        position_exit_plan(
            state,
            snapshot,
            sug_snapshot,
            input.price,
            side,
            ExitPlanContext::ActiveReprice,
        )
    } else {
        None
    };

    if !recalculate_virtual_sl && (!recalculate_tp || preserve_mrc_crossing_exit) {
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
            && !preserve_mrc_crossing_exit
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
                    "tp_reliable={}|mode={}|price={:.4}|sl={:.4}|sl_levels={}|mark={}|tick={:.4}",
                    plan.tp_reliable,
                    state.position.exit_mode.as_str(),
                    input.price,
                    state.position.stop_loss,
                    structural_stop_loss_note(state),
                    mark,
                    tick
                ),
            );
            let note = tp_reprice_order_note(
                state,
                tick,
                &format!("{}|reprice", plan.take_profit.order_note()),
            );
            return (
                vec![tp_limit_order(
                    direction,
                    qty,
                    plan.take_profit.price(),
                    &mark,
                    Some(&note),
                )],
                Vec::new(),
                Vec::new(),
            );
        }
    }
    (Vec::new(), Vec::new(), Vec::new())
}

pub(crate) fn ensure_active_exit_order(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    side: PositionState,
) -> ActiveExitResult {
    ensure_trailing_exit_order(input, state, snapshot, sug_snapshot, side)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Candle, MrcBands, SugInfoSnapshot, TimeframeSnapshot};

    fn tf(close: f64) -> TimeframeSnapshot {
        TimeframeSnapshot {
            candle: Candle {
                open: close,
                high: close + 1.0,
                low: close - 1.0,
                close,
            },
            mrc: MrcBands {
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
            zz_dir: 0,
            zz_buy_price: Some(close - 5.0),
            zz_sell_price: Some(close + 5.0),
            ..Default::default()
        }
    }

    #[test]
    fn long_plan_prefers_structural_levels() {
        let snapshot = IndicatorSnapshot {
            tf1m: tf(100.0),
            tf5m: tf(100.0),
            tf15m: tf(100.0),
            tf1h: tf(100.0),
            ..Default::default()
        };
        let sug = SugInfoSnapshot::default();
        let cascade = CascadeState {
            avg_size: 5.0,
            samples: 3,
            ..Default::default()
        };
        let plan = compute(
            &snapshot,
            &sug,
            &cascade,
            100.0,
            PositionState::Long,
            ExitPlanContext::Candidate,
            3,
        )
        .unwrap();
        assert!(plan.take_profit.price() > 100.0);
        assert!(plan.stop_loss.price() < 100.0);
    }

    #[test]
    fn short_plan_prefers_structural_levels() {
        let snapshot = IndicatorSnapshot {
            tf1m: tf(100.0),
            tf5m: tf(100.0),
            tf15m: tf(100.0),
            tf1h: tf(100.0),
            ..Default::default()
        };
        let sug = SugInfoSnapshot::default();
        let cascade = CascadeState {
            avg_size: 5.0,
            samples: 3,
            ..Default::default()
        };
        let plan = compute(
            &snapshot,
            &sug,
            &cascade,
            100.0,
            PositionState::Short,
            ExitPlanContext::Candidate,
            3,
        )
        .unwrap();
        assert!(plan.take_profit.price() < 100.0);
        assert!(plan.stop_loss.price() > 100.0);
    }

    #[test]
    fn structural_stop_skips_mrc_side_and_mean_line() {
        let mut long_snapshot = IndicatorSnapshot {
            tf1m: tf(100.0),
            tf5m: tf(100.0),
            tf15m: tf(100.0),
            tf1h: tf(100.0),
            ..Default::default()
        };
        long_snapshot.tf1m.zz_buy_price = None;
        long_snapshot.tf1m.zz_sell_price = None;
        long_snapshot.tf5m.zz_buy_price = None;
        long_snapshot.tf5m.zz_sell_price = None;
        long_snapshot.tf15m.zz_buy_price = None;
        long_snapshot.tf15m.zz_sell_price = None;
        long_snapshot.tf1m.zz_buy_price = Some(94.0);
        let long_plan = compute(
            &long_snapshot,
            &SugInfoSnapshot::default(),
            &CascadeState::default(),
            100.0,
            PositionState::Long,
            ExitPlanContext::Candidate,
            3,
        )
        .unwrap();
        assert!(long_plan.stop_loss.price() < 100.0);

        let short_snapshot = long_snapshot;
        let mut short_snapshot = short_snapshot;
        short_snapshot.tf1m.zz_sell_price = Some(106.0);
        let short_plan = compute(
            &short_snapshot,
            &SugInfoSnapshot::default(),
            &CascadeState::default(),
            100.0,
            PositionState::Short,
            ExitPlanContext::Candidate,
            3,
        )
        .unwrap();
        assert!(short_plan.stop_loss.price() > 100.0);
    }

    #[test]
    fn no_structural_tp_meets_cascade_minimum() {
        let snapshot = IndicatorSnapshot {
            tf1m: tf(100.0),
            tf5m: tf(100.0),
            tf15m: tf(100.0),
            tf1h: tf(100.0),
            ..Default::default()
        };
        let sug = SugInfoSnapshot {
            fast_candle: Some(crate::model::FastCandleSnapshot {
                candle: Candle {
                    open: 100.0,
                    high: 120.0,
                    low: 80.0,
                    close: 110.0,
                },
                active: true,
            }),
            ..Default::default()
        };
        let cascade = CascadeState {
            avg_size: 50.0,
            samples: 5,
            ..Default::default()
        };

        assert!(compute(
            &snapshot,
            &sug,
            &cascade,
            100.0,
            PositionState::Long,
            ExitPlanContext::Entry,
            3,
        )
        .is_none());
    }

    #[test]
    fn cascade_minimum_selects_next_structural_level() {
        let mut snapshot = IndicatorSnapshot {
            tf1m: tf(100.0),
            tf5m: tf(100.0),
            tf15m: tf(100.0),
            tf1h: tf(100.0),
            ..Default::default()
        };
        snapshot.tf1m.mrc.up_inner = 101.0;
        snapshot.tf1m.mrc.up_small = 105.0;
        snapshot.tf1m.mrc.up_outer = 110.0;
        let cascade = CascadeState {
            avg_size: 5.0,
            samples: 3,
            ..Default::default()
        };

        let plan = compute(
            &snapshot,
            &SugInfoSnapshot::default(),
            &cascade,
            100.0,
            PositionState::Long,
            ExitPlanContext::Entry,
            3,
        )
        .unwrap();

        assert_eq!(plan.take_profit.price(), 105.0);
        assert!(plan
            .take_profit
            .order_note()
            .contains("tp:structural|level="));

        let configured_minimum_plan = compute_with_excluded_stop_losses_and_minimum(
            &snapshot,
            &SugInfoSnapshot::default(),
            &cascade,
            100.0,
            PositionState::Long,
            ExitPlanContext::Entry,
            3,
            &[],
            6.0,
        )
        .unwrap();
        assert_eq!(configured_minimum_plan.take_profit.price(), 110.0);
    }

    #[test]
    fn mrc_crossing_plan_keeps_learned_target_and_adjacent_stop() {
        let plan = mrc_crossing_plan(104.0, 96.0).expect("valid MRC plan");
        assert_eq!(plan.take_profit.price(), 104.0);
        assert_eq!(plan.stop_loss.price(), 96.0);
        assert_eq!(plan.take_profit.source_label(), "mrc-crossing");
        assert_eq!(plan.stop_loss.reason(), "mrc-crossing:adjacent-line");
    }
}
