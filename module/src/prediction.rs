use crate::detection::levels;
use crate::exit;
use crate::model::{
    EntryGate, EntryWait, EntryWaitSolution, ExitKind, FastCandlePhase, IndicatorSnapshot,
    PositionState,
};
use crate::state::EngineState;

fn candle_range_pct(candle: &crate::model::Candle) -> f64 {
    if candle.close <= 0.0 {
        return 0.0;
    }
    ((candle.high - candle.low).abs() / candle.close) * 100.0
}

fn candidate_exit_plan(
    snapshot: &IndicatorSnapshot,
    price: f64,
    side: PositionState,
) -> Option<crate::exit::ExitPlan> {
    exit::plan(snapshot, price, side)
}

pub fn fast_candle_entry_block(snapshot: &IndicatorSnapshot) -> Option<String> {
    let candle_pct = candle_range_pct(&snapshot.tf1m.candle);
    let small_candle = candle_pct <= 0.40;
    let fc_phase = snapshot
        .fast_candle
        .map(|fc| fc.phase(&snapshot.tf1m.candle))
        .unwrap_or(FastCandlePhase::Absent);

    match fc_phase {
        FastCandlePhase::Building | FastCandlePhase::Tiring => Some(format!(
            "fc-phase={:?} fc={:.2}% candle={:.2}% gain_zone={:?} gain_prog={:.2}",
            fc_phase,
            snapshot.fast_candle.map(|fc| fc.range_pct()).unwrap_or(0.0),
            candle_pct,
            snapshot.tf1m.gain_zone(),
            snapshot.tf1m.gain_progress_pct()
        )),
        FastCandlePhase::Cooling if !small_candle => Some(format!(
            "fc-phase=Cooling candle-not-small fc={:.2}% candle={:.2}% gain_zone={:?} gain_prog={:.2}",
            snapshot.fast_candle.map(|fc| fc.range_pct()).unwrap_or(0.0),
            candle_pct,
            snapshot.tf1m.gain_zone(),
            snapshot.tf1m.gain_progress_pct()
        )),
        _ => None,
    }
}

fn dp_signal_band(snapshot: &IndicatorSnapshot, anchor: f64) -> f64 {
    let price = snapshot.tf1m.candle.close;
    let span = (snapshot.tf1m.dp_zee - snapshot.tf1m.dp_zee1).abs();
    let anchor_gap = (price - anchor).abs();
    span.max(anchor_gap) * 0.25
}

fn sug_entry_gate(snapshot: &IndicatorSnapshot, side: PositionState) -> Option<EntryGate> {
    let sug = snapshot.sug_candle?;
    let side_dir = match side {
        PositionState::Long => 1,
        PositionState::Short => -1,
        PositionState::Flat => return None,
    };
    if snapshot.sug_direction != 0 && snapshot.sug_direction != side_dir {
        return None;
    }

    let entry = match side {
        PositionState::Long => sug.open.min(sug.close),
        PositionState::Short => sug.open.max(sug.close),
        PositionState::Flat => return None,
    };
    let target = match side {
        PositionState::Long => sug.close.max(entry),
        PositionState::Short => sug.close.min(entry),
        PositionState::Flat => return None,
    };
    let range = (sug.high - sug.low).abs().max(f64::EPSILON);
    let band = (range * 0.20).max(snapshot.tf1m.mrc.range() * 0.01);
    let price = snapshot.tf1m.candle.close;
    let near_entry = (price - entry).abs() <= band;

    if near_entry {
        Some(EntryGate::Open {
            notes: vec![
                "sug-info-scalp".to_string(),
                format!("entry={:.4}", entry),
                format!("tp={:.4}", target),
                format!("band={:.4}", band),
            ],
        })
    } else {
        Some(EntryGate::Wait(EntryWait {
            reason: "sug-info-pullback-wait".to_string(),
            solutions: vec![EntryWaitSolution::ReclaimLevel {
                source: "sug-info".to_string(),
                level: entry,
                band_low: (entry - band).max(0.0),
                band_high: entry + band,
            }],
            notes: vec![
                "sug-info default scalping path".to_string(),
                format!("entry={:.4}", entry),
                format!("tp={:.4}", target),
                format!("price={:.4}", price),
            ],
        }))
    }
}

pub fn entry_gate(
    snapshot: &IndicatorSnapshot,
    state: &EngineState,
    side: PositionState,
) -> EntryGate {
    if let Some(reason) = fast_candle_entry_block(snapshot) {
        return EntryGate::Wait(EntryWait {
            reason: "fast-candle-wait".to_string(),
            solutions: vec![EntryWaitSolution::WaitForCooling {
                note: reason.clone(),
            }],
            notes: vec![reason],
        });
    }

    if matches!(state.exit_memory.kind, Some(ExitKind::StopLoss))
        && state.exit_memory.side == side
        && state.exit_memory.price > 0.0
    {
        let price = snapshot.tf1m.candle.close;
        let level_book = levels::structural_levels(snapshot);
        let band = dp_signal_band(snapshot, state.exit_memory.price);
        let band_low = (state.exit_memory.price - band).max(0.0);
        let band_high = state.exit_memory.price + band;
        let notes = vec![
            format!("failed_price={:.4}", state.exit_memory.price),
            format!("current={:.4}", price),
            format!(
                "dp_distance={:.4}%",
                if price > 0.0 {
                    ((price - snapshot.tf1m.dp_zee1).abs() / price) * 100.0
                } else {
                    0.0
                }
            ),
            state.exit_memory.summary(),
        ];
        match side {
            PositionState::Long if price < state.exit_memory.price => {
                return EntryGate::Wait(EntryWait {
                    reason: "post-sl-recovery wait-reclaim-above".to_string(),
                    solutions: vec![
                        EntryWaitSolution::ReclaimLevel {
                            source: "failed-level".to_string(),
                            level: state.exit_memory.price,
                            band_low,
                            band_high,
                        },
                        EntryWaitSolution::ReclaimDpsignal {
                            source: "dpsignal".to_string(),
                            anchor: snapshot.tf1m.dp_zee1.max(snapshot.tf1m.dp_zee),
                            distance_pct: if price > 0.0 {
                                ((price - snapshot.tf1m.dp_zee1).abs() / price) * 100.0
                            } else {
                                0.0
                            },
                            band_low,
                            band_high,
                        },
                        EntryWaitSolution::ReclaimLevel {
                            source: "structural".to_string(),
                            level: level_book
                                .nearest_above(price)
                                .unwrap_or(state.exit_memory.price),
                            band_low,
                            band_high,
                        },
                    ],
                    notes,
                });
            }
            PositionState::Short if price > state.exit_memory.price => {
                return EntryGate::Wait(EntryWait {
                    reason: "post-sl-recovery wait-reclaim-below".to_string(),
                    solutions: vec![
                        EntryWaitSolution::ReclaimLevel {
                            source: "failed-level".to_string(),
                            level: state.exit_memory.price,
                            band_low,
                            band_high,
                        },
                        EntryWaitSolution::ReclaimDpsignal {
                            source: "dpsignal".to_string(),
                            anchor: snapshot.tf1m.dp_zee1.min(snapshot.tf1m.dp_zee),
                            distance_pct: if price > 0.0 {
                                ((price - snapshot.tf1m.dp_zee1).abs() / price) * 100.0
                            } else {
                                0.0
                            },
                            band_low,
                            band_high,
                        },
                        EntryWaitSolution::ReclaimLevel {
                            source: "structural".to_string(),
                            level: level_book
                                .nearest_below(price)
                                .unwrap_or(state.exit_memory.price),
                            band_low,
                            band_high,
                        },
                    ],
                    notes,
                });
            }
            PositionState::Flat => {}
            _ => {}
        }
    }

    if let Some(gate) = sug_entry_gate(snapshot, side) {
        return gate;
    }

    let plan = if let Some(plan) = candidate_exit_plan(snapshot, snapshot.tf1m.candle.close, side) {
        plan
    } else {
        return EntryGate::Open {
            notes: vec!["no-exit-plan".to_string()],
        };
    };
    let candle_pct = candle_range_pct(&snapshot.tf1m.candle);
    let small_candle = candle_pct <= 0.40;
    let fc_phase = snapshot
        .fast_candle
        .map(|fc| fc.phase(&snapshot.tf1m.candle))
        .unwrap_or(FastCandlePhase::Absent);

    if !plan.tp_reliable && !(matches!(fc_phase, FastCandlePhase::Cooling) && small_candle) {
        let level_book = levels::structural_levels(snapshot);
        let anchor = if matches!(side, PositionState::Long) {
            snapshot.tf1m.dp_zee1.max(snapshot.tf1m.dp_zee)
        } else {
            snapshot.tf1m.dp_zee1.min(snapshot.tf1m.dp_zee)
        };
        let band = dp_signal_band(snapshot, anchor);
        return EntryGate::Wait(EntryWait {
            reason: format!(
                "tp-unreliable tp={:.4} sl={:.4}",
                plan.take_profit, plan.stop_loss
            ),
            solutions: vec![
                EntryWaitSolution::ReclaimDpsignal {
                    source: "dpsignal".to_string(),
                    anchor,
                    distance_pct: if snapshot.tf1m.candle.close > 0.0 {
                        ((snapshot.tf1m.candle.close - anchor).abs() / snapshot.tf1m.candle.close)
                            * 100.0
                    } else {
                        0.0
                    },
                    band_low: (anchor - band).max(0.0),
                    band_high: anchor + band,
                },
                EntryWaitSolution::WaitForCooling {
                    note: format!("fc={:?} candle={:.2}%", fc_phase, candle_pct),
                },
                EntryWaitSolution::ReclaimLevel {
                    source: "structural".to_string(),
                    level: if matches!(side, PositionState::Long) {
                        level_book
                            .nearest_above(snapshot.tf1m.candle.close)
                            .unwrap_or(anchor)
                    } else {
                        level_book
                            .nearest_below(snapshot.tf1m.candle.close)
                            .unwrap_or(anchor)
                    },
                    band_low: (anchor - band).max(0.0),
                    band_high: anchor + band,
                },
            ],
            notes: vec![
                format!("tp={:.4}", plan.take_profit),
                format!("sl={:.4}", plan.stop_loss),
                format!("tp_reliable={}", plan.tp_reliable),
                state.exit_memory.summary(),
            ],
        });
    }

    if matches!(state.exit_memory.kind, Some(ExitKind::TakeProfit))
        && state.exit_memory.side == side
        && !state.exit_memory.reliable
    {
        return EntryGate::Wait(EntryWait {
            reason: "post-tp-tight".to_string(),
            solutions: vec![EntryWaitSolution::ReclaimDpsignal {
                source: "dpsignal".to_string(),
                anchor: snapshot.tf1m.dp_zee1,
                distance_pct: if snapshot.tf1m.candle.close > 0.0 {
                    ((snapshot.tf1m.candle.close - snapshot.tf1m.dp_zee1).abs()
                        / snapshot.tf1m.candle.close)
                        * 100.0
                } else {
                    0.0
                },
                band_low: (snapshot.tf1m.dp_zee1 - dp_signal_band(snapshot, snapshot.tf1m.dp_zee1))
                    .max(0.0),
                band_high: snapshot.tf1m.dp_zee1 + dp_signal_band(snapshot, snapshot.tf1m.dp_zee1),
            }],
            notes: vec![
                format!("tp={:.4}", plan.take_profit),
                format!("sl={:.4}", plan.stop_loss),
                state.exit_memory.summary(),
            ],
        });
    }

    EntryGate::Open {
        notes: vec![format!(
            "tp={:.4} sl={:.4} tp_reliable={}",
            plan.take_profit, plan.stop_loss, plan.tp_reliable
        )],
    }
}
