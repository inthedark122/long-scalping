use crate::detection::{cascade, cascade::CascadeSignal, levels, sideways};
use crate::format::fmt_price_tick;
use crate::model::{
    EntryGate, EntryOpenReason, IndicatorSnapshot, PositionState, RegimeSnapshot, TradeDecision,
};
use crate::state::EngineState;

pub fn render(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    regime: &RegimeSnapshot,
    cascade_tf: &CascadeSignal,
    decision: &TradeDecision,
    constraint: Option<&str>,
    tick_size: f64,
) -> String {
    let cascade_tf_text = format_cascade(cascade_tf, &state.cascade_tf, snapshot.tf1m.candle.close);
    let sideway = sideways::analyze(state, &snapshot.tf1m);
    let pending_entry = state
        .movement_entry_pending
        .as_ref()
        .map(|pending| {
            format!(
                "{:?}:armed={} price={} reason={}",
                pending.direction,
                pending.armed_tick,
                fmt_price_tick(pending.price, tick_size),
                pending.reason
            )
        })
        .unwrap_or_else(|| "na".to_string());
    let tp_reprice = format_tp_reprice(&state.tp_reprice_history, tick_size);
    let mrc_risk = state
        .mrc_risk
        .latest()
        .map(|risk| risk.summary())
        .unwrap_or_else(|| "na".to_string());
    // Direction debug: base/final are the regime and final refined direction;
    // src identifies the actual decision stage that supplied the final value.
    // For the base regime, why=5m[...]|15m[...] uses L/S for Long/Short and
    // m/z for failed MRC and ZigZag conditions. If a later
    // stage changes direction, why is replaced by that stage's runtime values.
    let mut parts = vec![
        format!("t={}", state.ticks),
        format!("strategy={}", state.cfg.strategy_mode.as_str()),
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
        format!("5m={:?}", regime.primary),
        format!("15m={:?}", regime.fallback),
        format!("15m_fb={}", regime.fallback_used),
        format!("pending={}", pending_entry),
        format!(
            "sideway={}",
            if sideway.active {
                format!(
                    "on:{:?} bias={} block={}-{} {}",
                    sideway.source,
                    sideway.price_bias,
                    fmt_price_tick(sideway.block_low, tick_size),
                    fmt_price_tick(sideway.block_high, tick_size),
                    sideway.reason
                )
            } else {
                format!("off:{:?} {}", sideway.source, sideway.reason)
            }
        ),
        format!(
            "dir=base={:?}:final={:?}:src={}:why={}",
            regime.confirmed, decision.market_state, state.direction_source, state.direction_reason
        ),
        format!(
            "regime=[{:?}:{}]",
            regime.confirmed,
            if decision.market_state == regime.confirmed {
                "follow"
            } else {
                "break"
            }
        ),
        format!("entry_side={}", format_entry_side(regime.confirmed)),
        format!("dir_hist=[{}]", state.direction_history.summary(tick_size)),
        format!("casTf=[{}]", cascade_tf_text),
        format!("volume=[{}]", state.volume_gain.summary()),
        format!("mrc_risk=[{mrc_risk}]"),
        format!(
            "mrc_cross=[{}]",
            format_mrc_crossing(state, snapshot, decision, tick_size)
        ),
    ];
    if state.position.side == PositionState::Flat {
        parts.push(format!(
            "exec={}",
            format_execution_text(decision, tick_size)
        ));
    }
    if state.position.side != PositionState::Flat {
        parts.push(format!(
            "tp={}",
            format_take_profit(state, snapshot, tick_size)
        ));
    }
    if state.position.side != PositionState::Flat
        && state.position.stop_loss > 0.0
        && !state.position.stop_loss_reason.is_empty()
    {
        let processed_structural_sl = if state.processed_structural_stop_losses.is_empty() {
            "na".to_string()
        } else {
            state
                .processed_structural_stop_losses
                .iter()
                .map(|level| {
                    format!(
                        "{}@{}",
                        level.label(),
                        fmt_price_tick(level.value, tick_size)
                    )
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        parts.push(format!(
            "sl={} reason={} processed_sl={}",
            fmt_price_tick(state.position.stop_loss, tick_size),
            state.position.stop_loss_reason,
            processed_structural_sl,
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
                .map(|solution| format_entry_wait_solution(solution, tick_size))
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

fn format_entry_wait_solution(
    solution: &crate::model::EntryWaitSolution,
    tick_size: f64,
) -> String {
    match solution {
        crate::model::EntryWaitSolution::MrcCrossing => "mrc-crossing".to_string(),
        crate::model::EntryWaitSolution::MrcDirectionalTarget => "directional-target".to_string(),
        crate::model::EntryWaitSolution::WaitForSamples { required, current } => {
            format!("samples:{current}/{required}")
        }
        crate::model::EntryWaitSolution::WaitForDirectionBreak {
            source,
            direction,
            break_price,
        } => format!(
            "dir-break:{}:{:?}@{}",
            source,
            direction,
            fmt_price_tick(*break_price, tick_size)
        ),
        crate::model::EntryWaitSolution::WaitForConfirmation {
            source,
            direction,
            armed_tick,
            price,
        } => format!(
            "confirm:{}:{:?}:armed={}:price={}",
            source,
            direction,
            armed_tick,
            fmt_price_tick(*price, tick_size)
        ),
        crate::model::EntryWaitSolution::ReclaimLevel {
            source,
            level,
            band_low,
            band_high,
        } => format!(
            "reclaim:{}:{}[{}..{}]",
            source,
            fmt_price_tick(*level, tick_size),
            fmt_price_tick(*band_low, tick_size),
            fmt_price_tick(*band_high, tick_size)
        ),
        crate::model::EntryWaitSolution::FastCandle {
            phase,
            fast_candle_pct,
            candle_pct,
        } => format!(
            "fast-candle:{:?} fc={:.2}% candle={:.2}%",
            phase, fast_candle_pct, candle_pct
        ),
    }
}

fn format_entry_side(direction: crate::model::MarketState) -> &'static str {
    match direction {
        crate::model::MarketState::Long => "Long",
        crate::model::MarketState::Short => "Short",
        crate::model::MarketState::Sideways => "Flat",
    }
}

fn format_take_profit(state: &EngineState, snapshot: &IndicatorSnapshot, tick_size: f64) -> String {
    let price = state.position.take_profit;
    if !price.is_finite() || price <= 0.0 {
        return "none".to_string();
    }

    let source = match state.position.side {
        PositionState::Long => levels::structural_levels(snapshot)
            .above
            .iter()
            .find(|level| (level.value - price).abs() <= tick_size.max(f64::EPSILON) * 0.5)
            .map(|level| level.label()),
        PositionState::Short => levels::structural_levels(snapshot)
            .below
            .iter()
            .find(|level| (level.value - price).abs() <= tick_size.max(f64::EPSILON) * 0.5)
            .map(|level| level.label()),
        PositionState::Flat => None,
    }
    .unwrap_or_else(|| {
        if matches!(
            state.position.entry_reason.as_ref(),
            Some(crate::model::EntryReason::SidewayScalp { .. })
        ) {
            "sideway".to_string()
        } else {
            "derived".to_string()
        }
    });

    format!("{}:{}", fmt_price_tick(price, tick_size), source)
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

fn format_mrc_crossing(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    decision: &TradeDecision,
    tick_size: f64,
) -> String {
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
            "{}:+{:.2}%:-{:.2}%",
            direction,
            following_pct.max(0.0),
            opposite_pct.min(0.0)
        )
    } else {
        "na".to_string()
    };
    let tracked_line = tracked_mrc_line(decision, snapshot, tick_size);
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
        "[last={}:{}:{} track={} seq={} no_pullback={} revert=long:{}:short:{} move={} next={}]",
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
        next_progress
    )
}

fn tracked_mrc_line(
    decision: &TradeDecision,
    snapshot: &IndicatorSnapshot,
    tick_size: f64,
) -> &'static str {
    let EntryGate::Wait(wait) = &decision.entry else {
        return "na";
    };
    let Some(crate::model::EntryWaitSolution::ReclaimLevel { source, level, .. }) =
        wait.solutions.first()
    else {
        return "na";
    };
    if source != "mrc-window" {
        return "na";
    }
    let tolerance = tick_size.max(f64::EPSILON) * 2.0;
    snapshot
        .tf1m
        .mrc
        .lines()
        .iter()
        .enumerate()
        .find(|(_, line)| (**line - *level).abs() <= tolerance)
        .map(|(index, _)| mrc_line_label(index as u8))
        .unwrap_or("na")
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

fn format_cascade(
    cascade: &CascadeSignal,
    state: &crate::state::CascadeState,
    price: f64,
) -> String {
    let movement_pct = cascade::average_size_percent(cascade.completed_size, price).unwrap_or(0.0);
    let cascade_avg_pct = cascade::average_size_percent(cascade.average_size, price).unwrap_or(0.0);
    let last3_pct = cascade::recent_signed_total_size_percent(state, 3, price).unwrap_or(0.0);
    let last5_pct = cascade::recent_signed_total_size_percent(state, 5, price).unwrap_or(0.0);
    format!(
        "{:?}:{}:move={:.2}%/{:.0}%:avg={:.2}%:last3dist={:.2}%:last5dist={:.2}%",
        cascade.phase,
        cascade.dir,
        movement_pct,
        cascade.movement_position * 100.0,
        cascade_avg_pct,
        last3_pct,
        last5_pct,
    )
}

fn format_tp_reprice(
    history: &crate::observer::TpRepriceHistory,
    tick_size: f64,
) -> Option<String> {
    history.summary(tick_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detection::cascade::CascadeSignal;
    use crate::model::{
        Candle, EntryGate, EntryReason, ExitMode, IndicatorSnapshot, MarketState, PositionState,
        RegimeSnapshot, TradeDecision,
    };
    use crate::state::{EngineState, PositionRuntime};

    fn snapshot() -> IndicatorSnapshot {
        IndicatorSnapshot {
            tf1m: crate::model::TimeframeSnapshot {
                candle: Candle {
                    open: 100.0,
                    high: 101.0,
                    low: 99.0,
                    close: 100.0,
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn regime() -> RegimeSnapshot {
        RegimeSnapshot {
            primary: MarketState::Long,
            fallback: MarketState::Long,
            confirmed: MarketState::Long,
            fallback_used: false,
            context_1h: MarketState::Sideways,
            blocked_by_context: false,
        }
    }

    fn decision() -> TradeDecision {
        TradeDecision {
            market_state: MarketState::Long,
            entry: EntryGate::Open {
                notes: vec!["5m-primary".to_string()],
                reason: EntryOpenReason::Entry(EntryReason::SugInfoScalp { price: 100.0 }),
            },
        }
    }

    fn cascade(dir: i8) -> CascadeSignal {
        CascadeSignal {
            dir,
            prev_dir: dir,
            strength: 1.0,
            stretched: false,
            phase: crate::detection::cascade::MovementPhase::Building,
            completed_size: 1.0,
            movement_position: 0.5,
            average_size: 1.0,
            samples: 10,
            break_price: 0.0,
        }
    }

    #[test]
    fn single_mode_debug_omits_budget_fields() {
        let mut state = EngineState::default();
        state.position = PositionRuntime {
            side: PositionState::Long,
            entry_price: 100.0,
            peak_price: 101.0,
            trough_price: 99.0,
            take_profit: 101.0,
            stop_loss: 99.0,
            stop_loss_reason: "structural:1m:below:zz_sell".to_string(),
            exit_tp_reliable: true,
            exit_mode: ExitMode::Structural,
            exit_type: crate::model::ExitType::LevelExit,
            entry_reason: Some(EntryReason::SugInfoScalp { price: 100.0 }),
        };
        let debug = render(
            &state,
            &snapshot(),
            &regime(),
            &cascade(1),
            &decision(),
            None,
            0.01,
        );

        assert!(!debug.contains("mode=single"));
        assert!(!debug.contains("cap_init="));
        assert!(!debug.contains("trading_amount="));
    }

    #[test]
    fn entry_wait_debug_renders_solutions() {
        let mut state = EngineState::default();
        let mut decision = decision();
        decision.entry = EntryGate::Wait(crate::model::EntryWait {
            reason: "mrc-pullback-wait".to_string(),
            solutions: vec![crate::model::EntryWaitSolution::ReclaimLevel {
                source: "mrc-window".to_string(),
                level: 99.0,
                band_low: 98.0,
                band_high: 100.0,
            }],
            notes: vec!["forced-pullback".to_string()],
        });

        let debug = render(
            &state,
            &snapshot(),
            &regime(),
            &cascade(1),
            &decision,
            None,
            1.0,
        );

        assert!(!debug.contains("gate="));
        assert!(debug.contains(
            "exec=entry-wait reason=mrc-pullback-wait expect=reclaim:mrc-window:99[98..100]"
        ));
    }

    #[test]
    fn open_debug_renders_entry_reason() {
        let mut state = EngineState::default();
        let debug = render(
            &state,
            &snapshot(),
            &regime(),
            &cascade(1),
            &decision(),
            None,
            0.01,
        );

        assert!(debug.contains("exec=open:sug-info-scalp dir=long price=100"));
    }

    #[test]
    fn open_debug_renders_different_entry_reason() {
        let mut state = EngineState::default();
        let mut decision = decision();
        decision.entry = EntryGate::Open {
            notes: vec!["5m-primary".to_string()],
            reason: EntryOpenReason::Entry(EntryReason::SugInfoMovement { price: 101.25 }),
        };
        let debug = render(
            &state,
            &snapshot(),
            &regime(),
            &cascade(1),
            &decision,
            None,
            0.01,
        );

        assert!(debug.contains("exec=open:sug-info-movement dir=long price=101.25"));
    }

    #[test]
    fn exec_is_omitted_when_position_exists() {
        let mut state = EngineState::default();
        state.position = PositionRuntime {
            side: PositionState::Long,
            entry_price: 100.0,
            peak_price: 101.0,
            trough_price: 99.0,
            take_profit: 101.0,
            stop_loss: 99.0,
            stop_loss_reason: "structural:1m:below:zz_sell".to_string(),
            exit_tp_reliable: true,
            exit_mode: ExitMode::Structural,
            exit_type: crate::model::ExitType::LevelExit,
            entry_reason: None,
        };
        let debug = render(
            &state,
            &snapshot(),
            &regime(),
            &cascade(1),
            &decision(),
            None,
            0.01,
        );

        assert!(!debug.contains("exec="));
        assert!(debug.contains("sl=99 reason=structural"));
    }

    #[test]
    fn constraint_debug_renders_budget_too_small() {
        let state = EngineState::default();
        let debug = render(
            &state,
            &snapshot(),
            &regime(),
            &cascade(1),
            &decision(),
            Some("budget-too-small"),
            0.01,
        );

        assert!(debug.contains("constraint=budget-too-small"));
    }
}
