use crate::detection::movement_window;
use crate::detection::mrc_pattern_classification;
use crate::format::{fmt_duration_ms, fmt_price_tick};
use crate::model::{IndicatorSnapshot, PositionState};
use crate::state::EngineState;
use crate::strategies::StrategyKind;

pub fn render(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    entry_analysis: &str,
    entry_status: &str,
    exit_status: &str,
    routed_strategy: StrategyKind,
    constraint: Option<&str>,
    tick_size: f64,
    market_price: f64,
    ntp_5s: i64,
    tdt: Option<i64>,
) -> String {
    let tp_reprice = format_tp_reprice(&state.tp_reprice_history, tick_size);
    let mrc_risk = state
        .mrc_risk
        .latest()
        .map(|risk| risk.summary())
        .unwrap_or_else(|| "na".to_string());
    let active_pattern = state
        .mrc_crossing_model
        .active_pattern
        .and_then(|index| mrc_pattern_classification::MrcPatternKind::ALL.get(index as usize))
        .map(|pattern| pattern.as_str())
        .unwrap_or("none");
    let decision_state = state
        .mrc_crossing_model
        .decision_state
        .primary
        .map(|primary| {
            format!(
                "primary={:?}:dir={}:phase={:?}:best_favorable={:.4}%",
                primary.kind, primary.direction, primary.phase, primary.best_favorable_pct
            )
        })
        .unwrap_or_else(|| "primary=none".to_string());
    let support = state.mrc_crossing_model.decision_state.support;
    let primary_trace = &state.mrc_crossing_model.decision_state.trace;
    let movement_window = movement_window::detect(&state.cascade, market_price)
        .map(|window| {
            format!(
                "moves={} low={} high={} avg_move={} width={:.3}% changes={} price={} movement_start_ms={}",
                window.movements,
                fmt_price_tick(window.low, tick_size),
                fmt_price_tick(window.high, tick_size),
                fmt_price_tick(window.average_movement_size, tick_size),
                window.width_pct,
                window.direction_changes,
                window.price_location.as_str(),
                state.cascade.movement_start_ms
            )
        })
        .unwrap_or_else(|| "na".to_string());
    let stable_sideways_range = state
        .strategy_runtime
        .stable_sideways_range_bounds()
        .map(|(low, high)| {
            format!(
                "low={} high={}",
                fmt_price_tick(low, tick_size),
                fmt_price_tick(high, tick_size)
            )
        })
        .unwrap_or_else(|| "inactive".to_string());
    let active_strategy = if state.position.side == PositionState::Flat {
        "none"
    } else {
        state.position_context.strategy.as_str()
    };
    let mut parts = vec![
        format!("t={}", state.ticks),
        format!(
            "strategy-mode={}",
            if state.cfg.sideways_nonstop {
                "sideways-stable-range-experiment"
            } else if state.cfg.sideways_only {
                "sideways-only"
            } else {
                "mixed"
            }
        ),
        format!(
            "strategies=[trending,sideways,sideways-stable-range-experiment] routed={} active={}",
            routed_strategy.as_str(),
            active_strategy
        ),
        format!(
            "p={}",
            fmt_price_tick(snapshot.tf1m.candle.close, tick_size)
        ),
        format!(
            "position={:?}@{}",
            state.position.side,
            fmt_price_tick(state.position.entry_price, tick_size)
        ),
        format!(
            "selector=active:{} score={:.4} since={}",
            active_pattern,
            state.mrc_crossing_model.active_pattern_score,
            state.mrc_crossing_model.active_pattern_since_ms
        ),
        format!(
            "plan-key={:?} direction-lock={:?}",
            state.mrc_crossing_model.last_plan_key,
            state.mrc_crossing_model.trade_direction_lock
        ),
        format!(
            "decision={} support=[fbr={},pullback={},volume={},trend={},extreme_update={},extreme_failed={}] components=[{}]",
            decision_state,
            support.fast_break_return,
            support.pullback,
            support.volume_climax,
            support.trend,
            support.favorable_extreme_updated,
            support.favorable_extreme_failed,
            state.mrc_crossing_model.decision_components.names().join(",")
        ),
        format!(
            "primary-trace=stairs_up={}/{} stairs_down={}/{} lock={:?} blocked={} selected={:?}:dir={} stair_lifecycle={} stair_transition={} end=[extreme={},protected_swing={}] reset={} reason={}",
            primary_trace.stairs_up_candidates,
            primary_trace.stairs_up_tradeable,
            primary_trace.stairs_down_candidates,
            primary_trace.stairs_down_tradeable,
            primary_trace.direction_lock,
            primary_trace.blocked_by_direction_lock,
            primary_trace.selected_index,
            primary_trace.selected_direction,
            primary_trace
                .staircase_lifecycle
                .map(|value| value.as_str())
                .unwrap_or("na"),
            primary_trace
                .staircase_transition
                .map(|value| value.as_str())
                .unwrap_or("na"),
            primary_trace.ended_by_extreme,
            primary_trace.ended_by_protected_swing,
            primary_trace.direction_lock_reset_primary,
            primary_trace.reason
        ),
        format!(
            "analysis=src:{} why={}",
            state.direction_source, state.direction_reason
        ),
        format!("entry-analysis=[{entry_analysis}]"),
        format!(
            "cascade=dir={}:phase={}:speed={}:potential={}:adverse={} avg_time={}",
            state.cascade_signal.dir,
            state.cascade_signal.phase.as_str(),
            state
                .cascade_signal
                .speed_pct_per_min
                .map(|value| format!("{value:.3}%/min"))
                .unwrap_or_else(|| "na".to_string()),
            state
                .cascade_signal
                .potential_move_pct
                .map(|value| format!("{value:.3}%"))
                .unwrap_or_else(|| "na".to_string()),
            state
                .cascade_signal
                .adverse_move_pct
                .map(|value| format!("{value:.3}%"))
                .unwrap_or_else(|| "na".to_string()),
            state
                .cascade_signal
                .average_duration_ms
                .map(|value| fmt_duration_ms(value as u64))
                .unwrap_or_else(|| "na".to_string()),
        ),
        format!("movement-window=[{movement_window}]"),
        format!("stable-sideways-range=[{stable_sideways_range}]"),
        format!(
            "market-activity=ntp_5s:{} tdt:{}",
            ntp_5s,
            tdt.map(|value| value.to_string())
                .unwrap_or_else(|| "na".to_string())
        ),
        format!("volume=[{}]", state.volume_gain.summary()),
        format!("mrc_risk=[{mrc_risk}]"),
        format!("mrc_cross=[{}]", format_mrc_crossing(state)),
        format!("mrc_touch=[{}]", state.mrc_touch.signal.summary()),
        format!(
            "mrc_touch_learn=[{}]",
            state.mrc_touch_learning.signal.summary()
        ),
        format!(
            "mrc_predict=[{}]",
            state.mrc_direction_prediction.summary()
        ),
    ];
    parts.push(format!("entry={entry_status} exit={exit_status}"));
    if state.position.side != PositionState::Flat {
        parts.push(format!("tp={}", format_take_profit(state, tick_size)));
    }
    if state.position.side != PositionState::Flat {
        let future_route = state
            .market_mrc_analysis
            .as_ref()
            .map(|analysis| match analysis.trade_direction {
                1 => "long",
                -1 => "short",
                _ => "none",
            })
            .unwrap_or("none");
        parts.push(format!(
            "position-control=strategy={} action={} exit-source={} future-route={} lifetime={} reentry={} reason={}",
            state.position_context.strategy.as_str(),
            state.position_context.last_action.as_str(),
            state.position_context.last_plan_source.as_str(),
            future_route,
            state.position_context.lifetime.as_str(),
            if state.position_context.reentry_allowed {
                "allowed"
            } else {
                "blocked"
            },
            state.position_context.reentry_reason
        ));
    }
    if state.position.side != PositionState::Flat
        && state.position.stop_loss > 0.0
        && !state.position.stop_loss_reason.is_empty()
    {
        parts.push(format!(
            "vsl={} reason={}",
            fmt_price_tick(state.position.stop_loss, tick_size),
            state.position.stop_loss_reason,
        ));
    }
    if let (Some(strategy), Some(plan)) = (state.pending_strategy, state.pending_entry_plan) {
        if state.pending_entry_bar.is_some() && plan.take_profit > 0.0 && plan.stop_loss > 0.0 {
            parts.push(format!(
                "pending-plan=strategy={} tp={} sl={}",
                strategy.as_str(),
                fmt_price_tick(plan.take_profit, tick_size),
                fmt_price_tick(plan.stop_loss, tick_size),
            ));
        }
    }
    if state.position.side != PositionState::Flat {
        parts.push(format!(
            "exit={:?} mode={:?}",
            state.position.exit_type, state.position.exit_mode
        ));
        if !state.last_exit_event.is_empty() {
            parts.push(format!("exit-event=[{}]", state.last_exit_event));
        }
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

fn format_take_profit(state: &EngineState, tick_size: f64) -> String {
    let price = state.position.take_profit;
    if !price.is_finite() || price <= 0.0 {
        return "none".to_string();
    }

    let source = match state.position_context.strategy {
        StrategyKind::Trending => "mrc",
        StrategyKind::Sideways => "movement-window",
    };
    format!("{}:{source}", fmt_price_tick(price, tick_size))
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
        .map(|index| {
            let label = mrc_line_label(crossing.sequence[index]);
            if index + 1 >= crossing.sequence_len {
                return label.to_string();
            }
            let current_ms = crossing.sequence_times_ms[index];
            let next_ms = crossing.sequence_times_ms[index + 1];
            if current_ms == 0 || next_ms <= current_ms {
                format!("{label}|na")
            } else {
                format!("{label}|{}", format_sequence_duration(next_ms - current_ms))
            }
        })
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
            "{}:+{:.2}%:{:+.2}%",
            direction,
            following_pct.max(0.0),
            opposite_pct.min(0.0)
        )
    } else {
        "na".to_string()
    };
    let tracked_line = crossing
        .movement_target_line
        .map(mrc_line_label)
        .unwrap_or("na");
    let movement_age = crossing
        .sequence_len
        .checked_sub(1)
        .map(|index| crossing.sequence_times_ms[index])
        .filter(|started| *started > 0 && state.mrc_touch.signal.now_ms >= *started)
        .map(|started| format_sequence_duration(state.mrc_touch.signal.now_ms - started))
        .unwrap_or_else(|| "na".to_string());
    let next_progress = if crossing.movement_start > 0.0
        && crossing.movement_target > 0.0
        && crossing.last_price.is_some()
        && crossing.movement_direction != 0
    {
        let current = crossing.last_price.unwrap_or(crossing.movement_start);
        let distance = if crossing.movement_direction > 0 {
            crossing.movement_target - crossing.movement_start
        } else {
            crossing.movement_start - crossing.movement_target
        };
        if distance > 0.0 {
            let progress = if crossing.movement_direction > 0 {
                (current - crossing.movement_start) / distance * 100.0
            } else {
                (crossing.movement_start - current) / distance * 100.0
            };
            format!("{:.2}%", progress.clamp(0.0, 100.0))
        } else {
            "na".to_string()
        }
    } else {
        "na".to_string()
    };
    let count = state
        .vlines
        .get(crossing.last_line.unwrap_or(255) as usize)
        .map(|line| {
            if line.cross_count <= 1 {
                "first"
            } else {
                "repeat"
            }
        })
        .unwrap_or("na");
    format!(
        "[last={}:{}:{} track={} seq={} no_pullback={} revert=long:{}:short:{} move={} age={} next={}]",
        latest,
        direction,
        count,
        tracked_line,
        if sequence.is_empty() { "na" } else { &sequence },
        crossing.no_pullback_count,
        crossing
            .reverted_long_line
            .map(mrc_line_label)
            .unwrap_or("na"),
        crossing
            .reverted_short_line
            .map(mrc_line_label)
            .unwrap_or("na"),
        movement,
        movement_age,
        next_progress
    )
}

fn format_sequence_duration(duration_ms: u64) -> String {
    let total_seconds = duration_ms / 1_000;
    if total_seconds < 60 {
        return format!("{}s", total_seconds);
    }
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    if seconds == 0 {
        format!("{minutes}m")
    } else {
        format!("{minutes}m{seconds}s")
    }
}

fn format_tp_reprice(
    history: &crate::observer::TpRepriceHistory,
    tick_size: f64,
) -> Option<String> {
    history.summary(tick_size)
}
