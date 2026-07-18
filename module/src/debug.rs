use crate::detection::{cascade::CascadeSignal, sideways};
use crate::format::fmt_metric;
use crate::model::{IndicatorSnapshot, RegimeSnapshot, TradeDecision};
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
    let gain_1m = fmt_metric(snapshot.tf1m.volume_gain, 4);
    let gain_prog = fmt_metric(snapshot.tf1m.gain_progress_pct(), 2);
    let vml = if vt.long_side.count == 0 && vt.short_side.count == 0 {
        "na".to_string()
    } else {
        format!("{}/{}", vt.long_side.count, vt.short_side.count)
    };
    let mut parts = vec![
        format!("t={}", state.ticks),
        format!("p={:.4}", snapshot.tf1m.candle.close),
        format!("mode={}", state.cfg.trade_mode.as_str()),
        format!("5m={:?}", regime.primary),
        format!("15m={:?}", regime.fallback),
        format!("15m_fb={}", regime.fallback_used),
        format!("1h={:?}", regime.context_1h),
        format!("dir_lock={}", state.direction_lock.label()),
        format!("pending={}", pending_entry),
        format!(
            "sideway={}",
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
        format!("exec={}", execution_note),
    ];
    if let Some(gate) = format_entry_gate(&decision.entry) {
        parts.push(format!("gate={gate}"));
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
                text.push_str(&format!(" sol={}", wait.solutions.len()));
            }
            if !wait.notes.is_empty() {
                text.push_str(&format!(" notes={}", wait.notes.join("|")));
            }
            Some(text)
        }
        crate::model::EntryGate::Open { .. } => None,
    }
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
