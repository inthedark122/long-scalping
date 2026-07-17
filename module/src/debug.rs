use crate::detection::{cascade::CascadeSignal, sideways};
use crate::model::{IndicatorSnapshot, RegimeSnapshot, TradeDecision, TradeMode};
use crate::state::EngineState;

pub fn render(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    regime: &RegimeSnapshot,
    cascade_tf: &CascadeSignal,
    cascade_sug: &CascadeSignal,
    decision: &TradeDecision,
    execution_note: &str,
) -> String {
    let vt = &state.vlines[4];
    let cascade_tf_text = format_cascade(cascade_tf, snapshot.tf1m.candle.close);
    let cascade_sug_text = format_cascade(cascade_sug, snapshot.tf1m.candle.close);
    let sideway = sideways::analyze(state, &snapshot.tf1m);
    let mode_ctx = render_mode_context(state);
    let entry_reason = match &decision.entry_reason {
        Some(crate::model::EntryReason::SugInfoScalp { price, .. }) => {
            format!("sug-info-scalp price={:.4}", price)
        }
        Some(crate::model::EntryReason::SugInfoMovement { price, .. }) => {
            format!("sug-info-movement price={:.4}", price)
        }
        Some(crate::model::EntryReason::SidewayScalp {
            price,
            block_low,
            block_high,
            source,
            ..
        }) => {
            format!(
                "sideway-scalp price={:.4} block={:.4}-{:.4} source={}",
                price, block_low, block_high, source
            )
        }
        None => "none".to_string(),
    };
    let entry = match &decision.entry {
        crate::model::EntryGate::Open { notes } => {
            if notes.is_empty() {
                "open".to_string()
            } else {
                format!("open:{}", notes.join("|"))
            }
        }
        crate::model::EntryGate::Wait(wait) => {
            let mut text = format!("wait:{}", wait.reason);
            if !wait.solutions.is_empty() {
                text.push_str(&format!(" sol={}", wait.solutions.len()));
            }
            if !wait.notes.is_empty() {
                text.push_str(&format!(" notes={}", wait.notes.join("|")));
            }
            text
        }
    };
    let pending_entry = state
        .movement_entry_pending
        .as_ref()
        .map(|pending| {
            format!(
                "{:?}:armed={} price={:.4} reason={}",
                pending.direction, pending.armed_tick, pending.price, pending.reason
            )
        })
        .unwrap_or_else(|| "na".to_string());
    format!(
        "t={} p={:.4} mode={} 5m={:?} 15m={:?} 15m_fb={} 1h={:?} dir_lock={} pending={} sideway={} casTf={} casSug={} pos={:?} entry={} entry_reason={} mem={} mode_ctx={} tp_mark={} tp={:.4} sl={:.4} vml={}/{} gain_1m={:.4} gain_zone={:?} gain_prog={:.2} gain_building={} gain_late={} exec={}",
        state.ticks,
        snapshot.tf1m.candle.close,
        state.cfg.trade_mode.as_str(),
        regime.primary,
        regime.fallback,
        regime.fallback_used,
        regime.context_1h,
        state.direction_lock.label(),
        pending_entry,
        if sideway.active {
            format!(
                "on:{:?} bias={} block={:.4}-{:.4} {}",
                sideway.source,
                sideway.price_bias,
                sideway.block_low,
                sideway.block_high,
                sideway.reason
            )
        } else {
            format!("off:{:?} {}", sideway.source, sideway.reason)
        },
        cascade_tf_text,
        cascade_sug_text,
        state.position.side,
        entry,
        entry_reason,
        state.exit_memory.summary(),
        mode_ctx,
        if state.position.tp_mark.is_empty() {
            "na"
        } else {
            &state.position.tp_mark
        },
        state.position.take_profit,
        state.position.stop_loss,
        vt.long_side.count,
        vt.short_side.count,
        snapshot.tf1m.volume_gain,
        snapshot.tf1m.gain_zone(),
        snapshot.tf1m.gain_progress_pct(),
        snapshot.tf1m.gain_is_building(),
        snapshot.tf1m.gain_is_late(),
        execution_note,
    )
}

fn format_cascade(cascade: &CascadeSignal, price: f64) -> String {
    let cascade_avg_pct = if price > 0.0 {
        cascade.average_size / price * 100.0
    } else {
        0.0
    };
    format!(
        "{:?}:{}:avg={:.2}%:s={:.2}:b={:.4}",
        cascade.phase, cascade.dir, cascade_avg_pct, cascade.strength, cascade.break_price,
    )
}

fn render_mode_context(state: &EngineState) -> String {
    let strategy = state
        .strategy_builder
        .as_ref()
        .map(|builder| builder.describe())
        .unwrap_or_else(|| "strategy=na".to_string());
    match state.cfg.trade_mode {
        TradeMode::Single => format!(
            "single exit_src={} {}",
            state.position.exit_mode.as_str(),
            strategy
        ),
        TradeMode::Martingale => {
            if let Some(martingale) = state.position.martingale.as_ref() {
                format!(
                    "martingale exit_src=martingale {} mg_levels={} mg_range={:.4}% mg_step={:.2}% mg_tp={:.2}% mg_last={} mg_orders=2",
                    strategy,
                    martingale.profile.levels,
                    martingale.profile.range_pct,
                    martingale.profile.step_pct,
                    martingale.profile.take_profit_pct,
                    martingale.last_filled_mark.as_deref().unwrap_or("na"),
                )
            } else {
                format!("martingale exit_src=martingale {} mg_profile=na", strategy)
            }
        }
    }
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
            "exec=hold-long",
        );

        assert!(debug.contains("mode=single"));
        assert!(debug.contains("mode_ctx=single exit_src=structural"));
        assert!(!debug.contains("cap_init="));
        assert!(!debug.contains("trading_amount="));
        assert!(!debug.contains("pullback_amount="));
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
            "exec=hold-long",
        );

        assert!(debug.contains("mode=martingale"));
        assert!(debug.contains("mode_ctx=martingale exit_src=martingale"));
        assert!(debug.contains("mg_levels=9"));
        assert!(!debug.contains("cap_init="));
        assert!(!debug.contains("trading_amount="));
        assert!(!debug.contains("pullback_amount="));
    }
}
