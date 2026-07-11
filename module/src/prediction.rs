use crate::detection::cascade::CascadeSignal;
use crate::detection::{levels, sideways};
use crate::exit;
use crate::model::{
    EntryGate, EntryReason, EntryWait, EntryWaitSolution, ExitKind, FastCandlePhase,
    IndicatorSnapshot, MarketState, PositionState, SugInfoSnapshot,
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
    sug_snapshot: &SugInfoSnapshot,
    cascade: &crate::state::CascadeState,
    price: f64,
    side: PositionState,
    entry_reason: Option<&EntryReason>,
) -> Option<crate::exit::ExitPlan> {
    exit::plan(snapshot, sug_snapshot, cascade, price, side, entry_reason)
}

fn signal_candle<'a>(
    snapshot: &'a IndicatorSnapshot,
    sug_snapshot: &'a SugInfoSnapshot,
) -> &'a crate::model::Candle {
    sug_snapshot
        .sug_candle
        .as_ref()
        .unwrap_or(&snapshot.tf1m.candle)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementEntryKind {
    ReverseReady,
    SafeContinuation,
}

#[derive(Debug, Clone)]
pub struct MovementEntryPlan {
    pub market_state: MarketState,
    pub price: f64,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SidewayEntryPlan {
    pub market_state: MarketState,
    pub price: f64,
    pub notes: Vec<String>,
}

fn movement_fill_price(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    market_state: MarketState,
) -> f64 {
    let candle = signal_candle(snapshot, sug_snapshot);
    match market_state {
        MarketState::Long => candle.low.max(f64::EPSILON),
        MarketState::Short => candle.high.max(f64::EPSILON),
        MarketState::Sideways => candle.close.max(f64::EPSILON),
    }
}

pub fn sideway_entry_plan(
    snapshot: &IndicatorSnapshot,
    signal: &sideways::SidewaySignal,
) -> Option<SidewayEntryPlan> {
    if !signal.active || signal.block_high <= signal.block_low {
        return None;
    }

    let market_state = match signal.price_bias {
        -1 => MarketState::Long,
        1 => MarketState::Short,
        _ => return None,
    };

    let width = (signal.block_high - signal.block_low).abs();
    let buffer = (width * 0.10)
        .max(snapshot.tf1m.mrc.range() * 0.005)
        .max(snapshot.tf1m.candle.close * 0.0002)
        .max(f64::EPSILON);

    let (price, take_profit, stop_loss) = match market_state {
        MarketState::Long => (
            signal.block_low.max(f64::EPSILON),
            signal.block_high.max(signal.block_low + buffer),
            (signal.block_low - buffer).max(f64::EPSILON),
        ),
        MarketState::Short => (
            signal.block_high.max(f64::EPSILON),
            signal.block_low.max(f64::EPSILON),
            signal.block_high + buffer,
        ),
        MarketState::Sideways => return None,
    };

    Some(SidewayEntryPlan {
        market_state,
        price,
        notes: vec![
            "sideway-scalp".to_string(),
            format!("source={:?}", signal.source),
            format!("bias={}", signal.price_bias),
            format!("block={:.4}-{:.4}", signal.block_low, signal.block_high),
            format!("entry={:.4}", price),
            format!("tp={:.4}", take_profit),
            format!("sl={:.4}", stop_loss),
        ],
    })
}

fn movement_continuation_band(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    cascade: &CascadeSignal,
) -> f64 {
    let candle = signal_candle(snapshot, sug_snapshot);
    let candle_range = (candle.high - candle.low).abs();
    let avg_band = cascade.average_size * 0.20;
    let candle_band = candle_range * 0.25;
    let price_band = candle.close.abs() * 0.0005;
    avg_band.max(candle_band).max(price_band)
}

pub fn movement_entry_plan(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    cascade: &CascadeSignal,
) -> Option<MovementEntryPlan> {
    if cascade.samples < 3 || cascade.average_size <= 0.0 || cascade.dir == 0 {
        return None;
    }

    let stable = cascade.samples >= 3;
    let kind = if stable && cascade.strength >= 1.3 {
        Some(MovementEntryKind::ReverseReady)
    } else if stable && cascade.strength <= 0.8 {
        Some(MovementEntryKind::SafeContinuation)
    } else {
        None
    }?;

    let market_state = match kind {
        MovementEntryKind::ReverseReady => match cascade.prev_dir {
            1 => MarketState::Short,
            -1 => MarketState::Long,
            _ => return None,
        },
        MovementEntryKind::SafeContinuation => match cascade.dir {
            1 => MarketState::Long,
            -1 => MarketState::Short,
            _ => return None,
        },
    };

    let price = movement_fill_price(snapshot, sug_snapshot, market_state);
    if matches!(kind, MovementEntryKind::SafeContinuation) {
        let current_price = signal_candle(snapshot, sug_snapshot).close;
        let band = movement_continuation_band(snapshot, sug_snapshot, cascade);
        let distance = (current_price - price).abs();
        if distance > band {
            return None;
        }
    }

    let mut notes = vec![
        "movement-research".to_string(),
        format!(
            "kind={}",
            match kind {
                MovementEntryKind::ReverseReady => "reverse-ready",
                MovementEntryKind::SafeContinuation => "safe-continuation",
            }
        ),
        format!("dir={}", cascade.dir),
        format!("prev_dir={}", cascade.prev_dir),
        format!("strength={:.2}", cascade.strength),
        format!("avg={:.4}", cascade.average_size),
        format!("entry={:.4}", price),
    ];
    if matches!(kind, MovementEntryKind::ReverseReady) {
        notes.push("reverse-entry".to_string());
        notes.push("fill=aggressive".to_string());
    } else {
        notes.push("continuation-entry".to_string());
        notes.push(format!(
            "pullback-band={:.4}",
            movement_continuation_band(snapshot, sug_snapshot, cascade)
        ));
    }

    Some(MovementEntryPlan {
        market_state,
        price,
        notes,
    })
}

pub fn fast_candle_entry_block(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
) -> Option<String> {
    let candle = signal_candle(snapshot, sug_snapshot);
    let candle_pct = candle_range_pct(candle);
    let small_candle = candle_pct <= 0.40;
    let fc_phase = sug_snapshot
        .fast_candle
        .map(|fc| fc.phase(candle))
        .unwrap_or(FastCandlePhase::Absent);

    match fc_phase {
        FastCandlePhase::Building | FastCandlePhase::Tiring => Some(format!(
            "fc-phase={:?} fc={:.2}% candle={:.2}% gain_zone={:?} gain_prog={:.2}",
            fc_phase,
            sug_snapshot.fast_candle.map(|fc| fc.range_pct()).unwrap_or(0.0),
            candle_pct,
            snapshot.tf1m.gain_zone(),
            snapshot.tf1m.gain_progress_pct()
        )),
        FastCandlePhase::Cooling if !small_candle => Some(format!(
            "fc-phase=Cooling candle-not-small fc={:.2}% candle={:.2}% gain_zone={:?} gain_prog={:.2}",
            sug_snapshot.fast_candle.map(|fc| fc.range_pct()).unwrap_or(0.0),
            candle_pct,
            snapshot.tf1m.gain_zone(),
            snapshot.tf1m.gain_progress_pct()
        )),
        _ => None,
    }
}

fn dp_signal_band(snapshot: &IndicatorSnapshot, price: f64, anchor: f64) -> f64 {
    let span = (snapshot.tf1m.dp_zee - snapshot.tf1m.dp_zee1).abs();
    let anchor_gap = (price - anchor).abs();
    span.max(anchor_gap) * 0.25
}

fn direction_lock_gate(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    desired: MarketState,
) -> Option<EntryGate> {
    let lock = &state.direction_lock;
    if lock.direction == MarketState::Sideways || lock.direction == desired {
        return None;
    }

    Some(EntryGate::Wait(EntryWait {
        reason: "direction-frozen".to_string(),
        solutions: vec![EntryWaitSolution::WaitForDirectionBreak {
            source: "zigzagtrend+dpsignal".to_string(),
            direction: lock.direction,
            break_price: lock.break_price,
        }],
        notes: vec![
            format!("lock={}", lock.label()),
            format!("plan={:?}", desired),
            format!("price={:.4}", snapshot.tf1m.candle.close),
        ],
    }))
}

fn one_way_lock_gate(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    side: PositionState,
) -> Option<EntryGate> {
    let lock = &state.direction_lock;
    if lock.direction == MarketState::Sideways {
        return None;
    }

    let desired = match side {
        PositionState::Long => MarketState::Long,
        PositionState::Short => MarketState::Short,
        PositionState::Flat => MarketState::Sideways,
    };

    if desired != MarketState::Sideways && desired != lock.direction {
        return Some(EntryGate::Wait(EntryWait {
            reason: "one-way-lock".to_string(),
            solutions: vec![EntryWaitSolution::WaitForDirectionBreak {
                source: "direction-lock".to_string(),
                direction: lock.direction,
                break_price: lock.break_price,
            }],
            notes: vec![
                format!("lock={}", lock.label()),
                format!("plan={:?}", desired),
                format!("price={:.4}", snapshot.tf1m.candle.close),
                "same-side-only".to_string(),
            ],
        }));
    }

    None
}

fn movement_entry_confirmation_gate(
    state: &mut EngineState,
    plan: &MovementEntryPlan,
) -> Option<EntryGate> {
    match state.movement_entry_pending.as_ref() {
        Some(pending) if pending.direction == plan.market_state => {
            if state.ticks > pending.armed_tick {
                state.movement_entry_pending = None;
                None
            } else {
                Some(EntryGate::Wait(EntryWait {
                    reason: "movement-confirming".to_string(),
                    solutions: vec![EntryWaitSolution::WaitForConfirmation {
                        source: "movement".to_string(),
                        direction: plan.market_state,
                        armed_tick: pending.armed_tick,
                        price: pending.price,
                    }],
                    notes: vec![
                        format!("pending={:?}", pending.direction),
                        format!("armed_tick={}", pending.armed_tick),
                        format!("price={:.4}", pending.price),
                        "wait-next-signal".to_string(),
                    ],
                }))
            }
        }
        _ => {
            state.movement_entry_pending = Some(crate::state::PendingEntryState {
                direction: plan.market_state,
                armed_tick: state.ticks,
                price: plan.price,
                reason: "movement-realized".to_string(),
            });
            Some(EntryGate::Wait(EntryWait {
                reason: "movement-realized".to_string(),
                solutions: vec![EntryWaitSolution::WaitForConfirmation {
                    source: "movement".to_string(),
                    direction: plan.market_state,
                    armed_tick: state.ticks,
                    price: plan.price,
                }],
                notes: vec![
                    format!("pending={:?}", plan.market_state),
                    format!("armed_tick={}", state.ticks),
                    format!("price={:.4}", plan.price),
                    "wait-next-enter".to_string(),
                ],
            }))
        }
    }
}

fn sideway_reentry_gate(
    snapshot: &IndicatorSnapshot,
    state: &EngineState,
    side: PositionState,
) -> Option<EntryGate> {
    if state.exit_memory.kind != Some(ExitKind::TakeProfit) || state.exit_memory.side != side {
        return None;
    }

    let signal = sideways::analyze(state, &snapshot.tf1m);
    if sideways::reentry_ready(&signal, side) {
        return None;
    }

    Some(EntryGate::Wait(EntryWait {
        reason: "sideway-reentry-wait".to_string(),
        solutions: vec![EntryWaitSolution::WaitForCooling {
            note: format!(
                "sideway={} reentry-side={:?} need-zone={}",
                signal.reason,
                side,
                if matches!(side, PositionState::Long) {
                    "-1..-2"
                } else {
                    "1..2"
                }
            ),
        }],
        notes: vec![
            format!("sideway={}", signal.reason),
            format!("last_tp_side={:?}", state.exit_memory.side),
            format!("price={:.4}", snapshot.tf1m.candle.close),
        ],
    }))
}

fn sideway_entry_gate(
    snapshot: &IndicatorSnapshot,
    state: &EngineState,
    side: PositionState,
) -> Option<EntryGate> {
    let signal = sideways::analyze(state, &snapshot.tf1m);
    let plan = sideway_entry_plan(snapshot, &signal)?;
    if let Some(wait) = one_way_lock_gate(state, snapshot, side) {
        return Some(wait);
    }
    if signal.active && !sideways::reentry_ready(&signal, side) {
        return Some(EntryGate::Wait(EntryWait {
            reason: "sideway-opposite-wait".to_string(),
            solutions: vec![EntryWaitSolution::WaitForCooling {
                note: format!("sideway={} side={:?} need-ready", signal.reason, side),
            }],
            notes: vec![
                format!("sideway={}", signal.reason),
                format!("price={:.4}", snapshot.tf1m.candle.close),
                format!("lock={}", state.direction_lock.label()),
            ],
        }));
    }
    let desired_side = match plan.market_state {
        MarketState::Long => PositionState::Long,
        MarketState::Short => PositionState::Short,
        MarketState::Sideways => PositionState::Flat,
    };
    if desired_side != side || desired_side == PositionState::Flat {
        return None;
    }

    Some(EntryGate::Open { notes: plan.notes })
}

fn sug_entry_gate(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    state: &EngineState,
    side: PositionState,
) -> Option<EntryGate> {
    let sug = sug_snapshot.sug_candle?;
    let side_dir = match side {
        PositionState::Long => 1,
        PositionState::Short => -1,
        PositionState::Flat => return None,
    };
    if sug_snapshot.sug_direction != 0 && sug_snapshot.sug_direction != side_dir {
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
    if let Some(wait) = direction_lock_gate(
        state,
        snapshot,
        match side {
            PositionState::Long => MarketState::Long,
            PositionState::Short => MarketState::Short,
            PositionState::Flat => MarketState::Sideways,
        },
    ) {
        return Some(wait);
    }
    let range = (sug.high - sug.low).abs().max(f64::EPSILON);
    let band = (range * 0.20).max(snapshot.tf1m.mrc.range() * 0.01);
    let price = sug.close;
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
    sug_snapshot: &SugInfoSnapshot,
    cascade: &CascadeSignal,
    state: &mut EngineState,
    side: PositionState,
) -> EntryGate {
    if let Some(wait) = one_way_lock_gate(state, snapshot, side) {
        return wait;
    }

    if let Some(gate) = sideway_entry_gate(snapshot, state, side) {
        return gate;
    }

    if let Some(reason) = fast_candle_entry_block(snapshot, sug_snapshot) {
        return EntryGate::Wait(EntryWait {
            reason: "fast-candle-wait".to_string(),
            solutions: vec![EntryWaitSolution::WaitForCooling {
                note: reason.clone(),
            }],
            notes: vec![reason],
        });
    }

    if state.position.side == PositionState::Flat
        && cascade.samples < 3
        && sug_snapshot.sug_candle.is_some()
    {
        return EntryGate::Wait(EntryWait {
            reason: "movement-warmup".to_string(),
            solutions: vec![EntryWaitSolution::WaitForSamples {
                required: 3,
                current: cascade.samples,
            }],
            notes: vec![format!("cascade-samples={}", cascade.samples)],
        });
    }

    if let Some(wait) = sideway_reentry_gate(snapshot, state, side) {
        return wait;
    }

    if matches!(state.exit_memory.kind, Some(ExitKind::StopLoss))
        && state.exit_memory.side == side
        && state.exit_memory.price > 0.0
    {
        let price = snapshot.tf1m.candle.close;
        let level_book = levels::structural_levels(snapshot);
        let band = dp_signal_band(snapshot, price, state.exit_memory.price);
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

    if let Some(plan) = movement_entry_plan(snapshot, sug_snapshot, cascade) {
        if let Some(wait) = direction_lock_gate(state, snapshot, plan.market_state) {
            state.movement_entry_pending = None;
            return wait;
        }
        if let Some(wait) = movement_entry_confirmation_gate(state, &plan) {
            return wait;
        }
        return EntryGate::Open { notes: plan.notes };
    }

    state.movement_entry_pending = None;

    if let Some(gate) = sug_entry_gate(snapshot, sug_snapshot, state, side) {
        return gate;
    }

    let current_price = signal_candle(snapshot, sug_snapshot).close;
    let plan = if let Some(plan) = candidate_exit_plan(
        snapshot,
        sug_snapshot,
        &state.cascade_sug,
        current_price,
        side,
        state.position.entry_reason.as_ref(),
    ) {
        plan
    } else {
        return EntryGate::Open {
            notes: vec!["no-exit-plan".to_string()],
        };
    };
    let candle = signal_candle(snapshot, sug_snapshot);
    let candle_pct = candle_range_pct(candle);
    let small_candle = candle_pct <= 0.40;
    let fc_phase = sug_snapshot
        .fast_candle
        .map(|fc| fc.phase(candle))
        .unwrap_or(FastCandlePhase::Absent);

    if !plan.tp_reliable && !(matches!(fc_phase, FastCandlePhase::Cooling) && small_candle) {
        let level_book = levels::structural_levels(snapshot);
        let anchor = if matches!(side, PositionState::Long) {
            snapshot.tf1m.dp_zee1.max(snapshot.tf1m.dp_zee)
        } else {
            snapshot.tf1m.dp_zee1.min(snapshot.tf1m.dp_zee)
        };
        let band = dp_signal_band(snapshot, current_price, anchor);
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
                        ((current_price - anchor).abs() / current_price) * 100.0
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
                        level_book.nearest_above(current_price).unwrap_or(anchor)
                    } else {
                        level_book.nearest_below(current_price).unwrap_or(anchor)
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
                distance_pct: if current_price > 0.0 {
                    ((current_price - snapshot.tf1m.dp_zee1).abs() / current_price) * 100.0
                } else {
                    0.0
                },
                band_low: (snapshot.tf1m.dp_zee1
                    - dp_signal_band(snapshot, current_price, snapshot.tf1m.dp_zee1))
                .max(0.0),
                band_high: snapshot.tf1m.dp_zee1
                    + dp_signal_band(snapshot, current_price, snapshot.tf1m.dp_zee1),
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
