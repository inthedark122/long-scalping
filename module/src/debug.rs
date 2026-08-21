use crate::detection::decision::{PrimaryRuleTrace, SupportEvidence};
use crate::detection::mrc_pattern_classification;
use crate::format::fmt_price_tick;
use crate::model::{EntryGate, EntryOpenReason, IndicatorSnapshot, PositionState, TradeDecision};
use crate::state::EngineState;

pub fn render(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    decision: &TradeDecision,
    constraint: Option<&str>,
    tick_size: f64,
) -> String {
    let tp_reprice = format_tp_reprice(&state.tp_reprice_history, tick_size);
    let mrc_risk = state
        .mrc_risk
        .latest()
        .map(|risk| risk.level.as_str())
        .unwrap_or("na");
    let active_pattern = state
        .mrc_crossing_model
        .active_pattern
        .and_then(|index| mrc_pattern_classification::MrcPatternKind::ALL.get(index as usize))
        .map(|pattern| pattern.as_str())
        .unwrap_or("none");
    let support = state.mrc_crossing_model.decision_state.support;
    let primary_trace = &state.mrc_crossing_model.decision_state.trace;
    let mut parts = vec![
        format!("t={}", state.ticks),
        format!("entry={}", state.cfg.entry_type.as_str()),
        format!(
            "p={}",
            fmt_price_tick(snapshot.tf1m.candle.close, tick_size)
        ),
        format!(
            "position={:?}@{}",
            state.position.side,
            fmt_price_tick(state.position.entry_price, tick_size)
        ),
        format!("pattern={active_pattern}"),
        format!("support={}", format_support(support)),
        format_primary_trace(primary_trace),
        format!("dir={:?}", decision.market_state),
        format_route(state),
        format!(
            "cascade={}/{} state={}",
            state.cascade_signal.dir,
            state.cascade_signal.phase.as_str(),
            state.selected_cascade_trade_state.as_str(),
        ),
        format!("risk={mrc_risk}"),
        format!("mrc={}", format_mrc_crossing(state)),
    ];
    if state.position.side == PositionState::Flat {
        parts.push(format!(
            "exec={}",
            format_execution_text(decision, tick_size)
        ));
    }
    if state.position.side != PositionState::Flat {
        parts.push(format!("tp={}", format_take_profit(state, tick_size)));
    }
    if state.position.side != PositionState::Flat
        && state.position.stop_loss > 0.0
        && !state.position.stop_loss_reason.is_empty()
    {
        parts.push(format!(
            "sl={} reason={}",
            fmt_price_tick(state.position.stop_loss, tick_size),
            state.position.stop_loss_reason,
        ));
    }
    if state.position.side != PositionState::Flat {
        parts.push(format!(
            "exit={:?} mode={:?}",
            state.position.exit_type, state.position.exit_mode
        ));
    }
    if let Some(constraint) = constraint {
        parts.push(format!("constraint={constraint}"));
    }
    if let Some(tp_reprice) = tp_reprice {
        parts.push(format!("tp_reprice={tp_reprice}"));
    }
    if let Some(error) = state.last_error.as_deref() {
        parts.push(format!("error={error}"));
    }
    parts.join(" ")
}

fn format_route(state: &EngineState) -> String {
    if state.direction_reason.starts_with("line_plans[") {
        state.direction_reason.clone()
    } else {
        format!("reason={}", state.direction_reason)
    }
}

fn format_support(support: SupportEvidence) -> String {
    let mut active = Vec::new();
    if support.fast_break_return {
        active.push("fbr");
    }
    if support.pullback {
        active.push("pullback");
    }
    if support.volume_climax {
        active.push("volume");
    }
    if support.trend {
        active.push("trend");
    }
    if support.favorable_extreme_updated {
        active.push("ext+");
    }
    if support.favorable_extreme_failed {
        active.push("ext-");
    }
    if active.is_empty() {
        "none".to_string()
    } else {
        active.join("|")
    }
}

fn format_primary_trace(trace: &PrimaryRuleTrace) -> String {
    let lifecycle = trace
        .staircase_lifecycle
        .map(|value| value.as_str())
        .unwrap_or("na");
    let transition = trace
        .staircase_transition
        .map(|value| value.as_str())
        .unwrap_or("na");
    let end = match (trace.ended_by_extreme, trace.ended_by_protected_swing) {
        (false, false) => "none",
        (true, false) => "extreme",
        (false, true) => "swing",
        (true, true) => "extreme+swing",
    };
    let lock = match (trace.direction_lock, trace.blocked_by_direction_lock) {
        (Some(direction), blocked) => format!(" lock={direction} blocked={blocked}"),
        (None, blocked) if blocked > 0 => format!(" blocked={blocked}"),
        _ => String::new(),
    };
    let reset = if trace.direction_lock_reset_primary {
        " reset=true"
    } else {
        ""
    };
    format!(
        "trace=u{}/{} d{}/{} dir={} life={} trans={} end={}{}{} why={}",
        trace.stairs_up_candidates,
        trace.stairs_up_tradeable,
        trace.stairs_down_candidates,
        trace.stairs_down_tradeable,
        trace.selected_direction,
        lifecycle,
        transition,
        end,
        lock,
        reset,
        trace.reason,
    )
}

fn format_execution_text(decision: &TradeDecision, tick_size: f64) -> String {
    let direction = match decision.market_state {
        crate::model::MarketState::Long => "long",
        crate::model::MarketState::Short => "short",
        crate::model::MarketState::Sideways => "both",
    };
    match &decision.entry {
        EntryGate::Wait(wait) => format!(
            "entry-wait reason={} expect={}",
            wait.reason,
            wait.solutions
                .first()
                .map(format_entry_wait_solution)
                .unwrap_or_else(|| "wait".to_string())
        ),
        EntryGate::Open { reason, .. } => match reason {
            EntryOpenReason::Entry(reason) => format!(
                "open:{} dir={} price={}",
                reason.label(),
                direction,
                fmt_price_tick(reason.price(), tick_size)
            ),
        },
    }
}

fn format_entry_wait_solution(solution: &crate::model::EntryWaitSolution) -> String {
    match solution {
        crate::model::EntryWaitSolution::MrcCrossing => "mrc-crossing".to_string(),
        crate::model::EntryWaitSolution::MrcDirectionalTarget => "directional-target".to_string(),
        crate::model::EntryWaitSolution::WaitForSamples { required, current } => {
            format!("samples:{current}/{required}")
        }
    }
}

fn format_take_profit(state: &EngineState, tick_size: f64) -> String {
    let price = state.position.take_profit;
    if !price.is_finite() || price <= 0.0 {
        return "none".to_string();
    }

    format!("{}:mrc", fmt_price_tick(price, tick_size))
}

fn mrc_line_label(index: u8) -> &'static str {
    match index {
        0 => "db",
        1 => "do",
        2 => "ds",
        3 => "di",
        4 => "m",
        5 => "ui",
        6 => "us",
        7 => "uo",
        8 => "ub",
        _ => "na",
    }
}

fn format_mrc_crossing(state: &EngineState) -> String {
    let crossing = &state.mrc_crossing;
    let latest = crossing.last_line.map(mrc_line_label).unwrap_or("na");
    let direction = match crossing.last_direction {
        1 => "long",
        -1 => "short",
        _ => "na",
    };
    let sequence = (0..crossing.sequence_len)
        .map(|index| mrc_line_label(crossing.sequence[index]))
        .collect::<Vec<_>>()
        .join(">");
    let movement = if crossing.movement_start > 0.0
        && crossing.movement_high > 0.0
        && crossing.movement_low > 0.0
        && crossing.movement_direction != 0
    {
        let (following_pct, opposite_pct) = if crossing.movement_direction > 0 {
            (
                (crossing.movement_high - crossing.movement_start) / crossing.movement_start
                    * 100.0,
                (crossing.movement_low - crossing.movement_start) / crossing.movement_start * 100.0,
            )
        } else {
            (
                (crossing.movement_start - crossing.movement_low) / crossing.movement_start * 100.0,
                (crossing.movement_start - crossing.movement_high) / crossing.movement_start
                    * 100.0,
            )
        };
        format!(
            "{}:+{:.2}%/-{:.2}%",
            match crossing.movement_direction {
                1 => "long",
                -1 => "short",
                _ => "na",
            },
            following_pct.max(0.0),
            opposite_pct.abs()
        )
    } else {
        "na".to_string()
    };
    format!(
        "{}:{} seq={} move={}",
        latest,
        direction,
        if sequence.is_empty() { "na" } else { &sequence },
        movement,
    )
}

fn format_tp_reprice(
    history: &crate::observer::TpRepriceHistory,
    tick_size: f64,
) -> Option<String> {
    history.summary(tick_size)
}
