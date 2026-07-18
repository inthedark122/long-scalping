use crate::detection::{cascade::CascadeSignal, sideways};
use crate::format::{fmt_metric, fmt_price_tick};
use crate::model::{IndicatorSnapshot, RegimeSnapshot, TradeDecision};
use crate::state::EngineState;

pub fn render(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    regime: &RegimeSnapshot,
    cascade_tf: &CascadeSignal,
    cascade_sug: &CascadeSignal,
    decision: &TradeDecision,
    constraint: Option<&str>,
    tick_size: f64,
) -> String {
    let vt = &state.vlines[4];
    let cascade_tf_text = format_cascade(cascade_tf, snapshot.tf1m.candle.close, tick_size);
    let cascade_sug_text = format_cascade(cascade_sug, snapshot.tf1m.candle.close, tick_size);
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
    let gain_1m = fmt_metric(snapshot.tf1m.volume_gain, 4);
    let gain_prog = fmt_metric(snapshot.tf1m.gain_progress_pct(), 2);
    let vml = if vt.long_side.count == 0 && vt.short_side.count == 0 {
        "na".to_string()
    } else {
        format!("{}/{}", vt.long_side.count, vt.short_side.count)
    };
    let tp_reprice = format_tp_reprice(&state.tp_reprice_history, tick_size);
    let mut parts = vec![
        format!("t={}", state.ticks),
        format!("p={}", fmt_price_tick(snapshot.tf1m.candle.close, tick_size)),
        format!("mode={}", state.cfg.trade_mode.as_str()),
        format!("5m={:?}", regime.primary),
        format!("15m={:?}", regime.fallback),
        format!("15m_fb={}", regime.fallback_used),
        format!("1h={:?}", regime.context_1h),
        format!(
            "dir_lock={}",
            format_direction_lock(&state.direction_lock, tick_size)
        ),
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
        format!("dir_hist=[{}]", state.direction_history.summary()),
        format!("casTf={}", cascade_tf_text),
        format!("casSug={}", cascade_sug_text),
        format!("vml={}", vml),
        format!("gain_1m={}", gain_1m),
        format!("gain_zone={:?}", snapshot.tf1m.gain_zone()),
        format!("gain_prog={}", gain_prog),
        format!("gain_building={}", snapshot.tf1m.gain_is_building()),
        format!("gain_late={}", snapshot.tf1m.gain_is_late()),
        format!("exec={}", format_execution_text(decision)),
    ];
    if let Some(constraint) = constraint {
        parts.push(format!("constraint={constraint}"));
    }
    if let Some(gate) = format_entry_gate(&decision.entry) {
        parts.push(format!("gate={gate}"));
    }
    if let Some(tp_reprice) = tp_reprice {
        parts.push(format!("tp_reprice={tp_reprice}"));
    }
    let base = parts.join(" ");

    match state.last_error.as_ref() {
        Some(error) => format!("{base} error={error}"),
        None => base,
    }
}

fn format_entry_gate(entry: &crate::model::EntryGate) -> Option<String> {
    match entry {
        crate::model::EntryGate::Wait(wait) => {
            let mut text = format!("wait:{}", wait.reason);
            if !wait.solutions.is_empty() {
                let solutions = wait
                    .solutions
                    .iter()
                    .map(format_entry_wait_solution)
                    .collect::<Vec<_>>()
                    .join(";");
                text.push_str(&format!(" sol=[{}]", solutions));
            }
            if !wait.notes.is_empty() {
                text.push_str(&format!(" notes={}", wait.notes.join("|")));
            }
            Some(text)
        }
        crate::model::EntryGate::Open { .. } => None,
    }
}

fn format_execution_text(decision: &TradeDecision) -> String {
    if let crate::model::EntryGate::Wait(wait) = &decision.entry {
        return format!(
            "entry-wait reason={} expect={}",
            wait.reason,
            wait.solutions
                .first()
                .map(format_entry_wait_solution)
                .unwrap_or_else(|| "wait".to_string())
        );
    }
    "na".to_string()
}

fn format_entry_wait_solution(solution: &crate::model::EntryWaitSolution) -> String {
    match solution {
        crate::model::EntryWaitSolution::WaitForSamples { required, current } => {
            format!("samples:{current}/{required}")
        }
        crate::model::EntryWaitSolution::WaitForDirectionBreak {
            source,
            direction,
            break_price,
        } => format!("dir-break:{}:{:?}@{break_price}", source, direction),
        crate::model::EntryWaitSolution::WaitForConfirmation {
            source,
            direction,
            armed_tick,
            price,
        } => format!(
            "confirm:{}:{:?}:armed={}:price={}",
            source, direction, armed_tick, price
        ),
        crate::model::EntryWaitSolution::ReclaimLevel {
            source,
            level,
            band_low,
            band_high,
        } => format!("reclaim:{}:{}[{}..{}]", source, level, band_low, band_high),
        crate::model::EntryWaitSolution::ReclaimDpsignal {
            source,
            anchor,
            distance_pct,
            band_low,
            band_high,
        } => format!(
            "dpsignal:{}:{} dist={:.2}% [{}..{}]",
            source, anchor, distance_pct, band_low, band_high
        ),
        crate::model::EntryWaitSolution::FastCandle {
            phase,
            fast_candle_pct,
            candle_pct,
            gain_zone,
            gain_progress_pct,
        } => format!(
            "fast-candle:{:?} fc={:.2}% candle={:.2}% gain_zone={:?} gain_prog={:.2}",
            phase, fast_candle_pct, candle_pct, gain_zone, gain_progress_pct
        ),
    }
}

fn format_direction_lock(lock: &crate::state::DirectionLockState, tick_size: f64) -> String {
    if lock.direction == crate::model::MarketState::Sideways {
        return "na".to_string();
    }
    if lock.break_price <= 0.0 {
        return format!("{:?}:na:{}", lock.direction, lock.reason);
    }
    format!(
        "{:?}:{}:{}",
        lock.direction,
        fmt_price_tick(lock.break_price, tick_size),
        lock.reason
    )
}

fn format_cascade(cascade: &CascadeSignal, price: f64, tick_size: f64) -> String {
    let cascade_avg_pct = if price > 0.0 {
        cascade.average_size / price * 100.0
    } else {
        0.0
    };
    format!(
        "{:?}:{}:avg={:.2}%:s={:.2}:b={}",
        cascade.phase,
        cascade.dir,
        cascade_avg_pct,
        cascade.strength,
        fmt_price_tick(cascade.break_price, tick_size),
    )
}

fn format_tp_reprice(history: &crate::observer::TpRepriceHistory, tick_size: f64) -> Option<String> {
    history.summary(tick_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detection::cascade::CascadeSignal;
    use crate::model::{
        Candle, EntryGate, EntryReason, ExitMode, IndicatorSnapshot, MarketState, PositionState,
        RegimeSnapshot, TradeDecision, TradeMode,
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
            },
            entry_reason: Some(EntryReason::SugInfoScalp {
                price: 100.0,
                started_at_ms: 1,
            }),
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
            average_size: 1.0,
            samples: 10,
            break_price: 0.0,
        }
    }

    #[test]
    fn single_mode_debug_omits_budget_fields() {
        let mut state = EngineState::default();
        state.cfg.trade_mode = TradeMode::Single;
        state.position = PositionRuntime {
            side: PositionState::Long,
            entry_price: 100.0,
            peak_price: 101.0,
            trough_price: 99.0,
            take_profit: 101.0,
            stop_loss: 99.0,
            exit_tp_reliable: true,
            exit_mode: ExitMode::Structural,
            tp_mark: "tp-long".to_string(),
            entry_reason: Some(EntryReason::SugInfoScalp {
                price: 100.0,
                started_at_ms: 1,
            }),
            martingale: None,
        };
        let debug = render(
            &state,
            &snapshot(),
            &regime(),
            &cascade(1),
            &cascade(1),
            &decision(),
            None,
            0.01,
        );

        assert!(debug.contains("mode=single"));
        assert!(debug.contains("mode_ctx=single exit_src=structural"));
        assert!(!debug.contains("cap_init="));
        assert!(!debug.contains("trading_amount="));
        assert!(!debug.contains("pullback_amount="));
    }

    #[test]
    fn entry_wait_debug_renders_solutions() {
        let mut state = EngineState::default();
        state.cfg.trade_mode = TradeMode::Single;
        let mut decision = decision();
        decision.entry = EntryGate::Wait(crate::model::EntryWait {
            reason: "sug-info-pullback-wait".to_string(),
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
            &cascade(1),
            &decision,
            None,
            1.0,
        );

        assert!(debug.contains(
            "gate=wait:sug-info-pullback-wait sol=[reclaim:mrc-window:99[98..100]] notes=forced-pullback"
        ));
    }

    #[test]
    fn martingale_mode_debug_shows_only_martingale_context() {
        let mut state = EngineState::default();
        state.cfg.trade_mode = TradeMode::Martingale;
        state.position = PositionRuntime {
            side: PositionState::Long,
            entry_price: 100.0,
            peak_price: 101.0,
            trough_price: 99.0,
            take_profit: 101.0,
            stop_loss: 99.0,
            exit_tp_reliable: true,
            exit_mode: ExitMode::Structural,
            tp_mark: "tp-long".to_string(),
            entry_reason: Some(EntryReason::SugInfoScalp {
                price: 100.0,
                started_at_ms: 1,
            }),
            martingale: Some(crate::state::MartingaleRuntime {
                profile: crate::martingale::LadderProfile {
                    levels: 9,
                    range_pct: 2.4,
                    step_pct: 0.8,
                    take_profit_pct: 0.64,
                },
                base_price: 0.0,
                base_amount: 0.0,
                last_filled_mark: None,
            }),
        };
        let debug = render(
            &state,
            &snapshot(),
            &regime(),
            &cascade(1),
            &cascade(1),
            &decision(),
            None,
            0.01,
        );

        assert!(debug.contains("mode=martingale"));
        assert!(debug.contains("mode_ctx=martingale exit_src=martingale"));
        assert!(debug.contains("mg_levels=9"));
        assert!(!debug.contains("cap_init="));
        assert!(!debug.contains("trading_amount="));
        assert!(!debug.contains("pullback_amount="));
    }

    #[test]
    fn constraint_debug_renders_budget_too_small() {
        let state = EngineState::default();
        let debug = render(
            &state,
            &snapshot(),
            &regime(),
            &cascade(1),
            &cascade(1),
            &decision(),
            Some("budget-too-small"),
            0.01,
        );

        assert!(debug.contains("constraint=budget-too-small"));
    }
}
