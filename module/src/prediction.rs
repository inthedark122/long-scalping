use crate::detection::cascade::{self, CascadeSignal};
use crate::detection::{levels, sideways};
use crate::exit;
use crate::model::{
    EntryGate, EntryOpenReason, EntryReason, EntryWait, EntryWaitSolution, ExitKind,
    FastCandlePhase, IndicatorSnapshot, MarketState, PositionState, SugInfoSnapshot,
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
    exit::compute(
        snapshot,
        sug_snapshot,
        cascade,
        price,
        side,
        entry_reason,
        false,
    )
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

fn mrc_window(snapshot: &IndicatorSnapshot, price: f64) -> Option<levels::MrcPriceWindow> {
    levels::mrc_price_window(&snapshot.tf1m.mrc, price)
}

fn mrc_window_band(window: &levels::MrcPriceWindow) -> f64 {
    let factor = match window.distance {
        levels::MrcLineDistance::One => 0.25,
        levels::MrcLineDistance::Two => 0.35,
    };
    (window.width * factor).max(f64::EPSILON)
}

#[derive(Debug, Clone)]
pub struct MovementEntryPlan {
    pub movement_direction: i8,
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

#[derive(Debug, Clone)]
pub struct SugEntryPlan {
    pub market_state: MarketState,
    pub entry: f64,
    pub target: f64,
    pub band: f64,
    pub notes: Vec<String>,
}

fn movement_fill_price(
    cascade: &CascadeSignal,
    history: &crate::state::CascadeState,
    market_state: MarketState,
) -> Option<f64> {
    let offset = (cascade.completed_size * 0.20).min(cascade.average_size * 0.20);
    if !offset.is_finite() || offset <= 0.0 {
        return None;
    }
    let previous = cascade::movement_history(history).into_iter().last()?;
    match market_state {
        MarketState::Long if previous.low > 0.0 => {
            let price = previous.low + offset;
            price.is_finite().then_some(price)
        }
        MarketState::Short if previous.high > 0.0 => {
            let price = previous.high - offset;
            (price.is_finite() && price > 0.0).then_some(price)
        }
        MarketState::Sideways | MarketState::Long | MarketState::Short => None,
    }
}

pub fn sideway_entry_plan(
    snapshot: &IndicatorSnapshot,
    signal: &sideways::SidewaySignal,
    cascade: &CascadeSignal,
) -> Option<SidewayEntryPlan> {
    if !signal.active || signal.block_high <= signal.block_low {
        return None;
    }

    let market_state = match signal.price_bias {
        -1 => MarketState::Long,
        1 if matches!(signal.source, sideways::SidewaySource::LineCross) => MarketState::Long,
        _ => return None,
    };

    let width = (signal.block_high - signal.block_low).abs();
    let buffer = (width * 0.10)
        .max(snapshot.tf1m.mrc.range() * 0.005)
        .max(snapshot.tf1m.candle.close * 0.0002)
        .max(f64::EPSILON);

    let current_price = snapshot.tf1m.candle.close.max(f64::EPSILON);
    let cascade_pullback = (cascade.average_size * 0.60)
        .max(snapshot.tf1m.mrc.range() * 0.005)
        .max(f64::EPSILON);
    let pullback_cap = match market_state {
        MarketState::Long => signal.block_low + cascade_pullback,
        MarketState::Short => (signal.block_high - cascade_pullback).max(f64::EPSILON),
        MarketState::Sideways => return None,
    };

    if matches!(signal.source, sideways::SidewaySource::LineCross) {
        match market_state {
            MarketState::Long if current_price > pullback_cap => return None,
            MarketState::Short if current_price < pullback_cap => return None,
            _ => {}
        }
    }

    let (price, take_profit, stop_loss, mode_note) = match (market_state, signal.price_bias) {
        (MarketState::Long, -1) => (
            signal.block_low.max(f64::EPSILON),
            signal.block_high.max(signal.block_low + buffer),
            (signal.block_low - buffer).max(f64::EPSILON),
            "reversal-entry".to_string(),
        ),
        (MarketState::Long, 1) => (
            current_price,
            signal.block_high.max(current_price + buffer),
            (signal.block_low - buffer).max(f64::EPSILON),
            if matches!(signal.source, sideways::SidewaySource::LineCross) {
                "sug-info-pullback".to_string()
            } else {
                "continuation-entry".to_string()
            },
        ),
        _ => return None,
    };

    Some(SidewayEntryPlan {
        market_state,
        price,
        notes: vec![
            "sideway-scalp".to_string(),
            format!("source={:?}", signal.source),
            format!("bias={}", signal.price_bias),
            mode_note,
            format!("block={:.4}-{:.4}", signal.block_low, signal.block_high),
            format!("pullback_cap={:.4}", pullback_cap),
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
    history: &crate::state::CascadeState,
    market_state: MarketState,
) -> Option<MovementEntryPlan> {
    if cascade.samples < 3 || cascade.average_size <= 0.0 || cascade.stretched {
        return None;
    }

    let movement_direction = cascade.dir;
    let entry_state = match market_state {
        MarketState::Long | MarketState::Short => market_state,
        // This is intentionally counter-directional: the movement has already
        // happened, so the entry tries to catch a pullback from its prior
        // high/low rather than chase continuation. It is a candidate, not a
        // guarantee that every completed movement will reverse.
        MarketState::Sideways => match movement_direction {
            1 => MarketState::Short,
            -1 => MarketState::Long,
            _ => return None,
        },
    };
    let price = movement_fill_price(cascade, history, entry_state)?;
    let current_price = signal_candle(snapshot, sug_snapshot).close;
    let band = movement_continuation_band(snapshot, sug_snapshot, cascade);
    let distance = (current_price - price).abs();
    if distance > band {
        return None;
    }

    let notes = vec![
        "movement-research".to_string(),
        format!("movement_dir={}", movement_direction),
        format!("market={:?}", entry_state),
        format!("strength={:.2}", cascade.strength),
        format!("avg={:.4}", cascade.average_size),
        format!("entry={:.4}", price),
        format!("pullback-band={:.4}", band),
    ];

    Some(MovementEntryPlan {
        movement_direction,
        market_state: entry_state,
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
        solutions: vec![EntryWaitSolution::ReclaimLevel {
            source: "sideway-reentry".to_string(),
            level: (signal.block_low + signal.block_high) / 2.0,
            band_low: signal.block_low,
            band_high: signal.block_high,
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
    cascade: &CascadeSignal,
    side: PositionState,
) -> Option<EntryGate> {
    if state.exit_memory.kind != Some(ExitKind::StopLoss) {
        return None;
    }

    let signal = sideways::analyze(state, &snapshot.tf1m);
    let current_price = snapshot.tf1m.candle.close.max(f64::EPSILON);
    let Some(window) = mrc_window(snapshot, current_price) else {
        return None;
    };
    let mrc_window_width = window.width;
    if signal.active && matches!(signal.source, sideways::SidewaySource::LineCross) {
        let cascade_pullback = (cascade.average_size * 0.60)
            .max(mrc_window_width * 0.20)
            .max(f64::EPSILON);
        let pullback_cap = if matches!(signal.side, PositionState::Long) {
            signal.block_low + cascade_pullback
        } else {
            (signal.block_high - cascade_pullback).max(f64::EPSILON)
        };

        let pullback_wait = match signal.side {
            PositionState::Long if current_price > pullback_cap => {
                Some(EntryGate::Wait(EntryWait {
                    reason: "sideway-linecross-pullback-wait".to_string(),
                    solutions: vec![EntryWaitSolution::ReclaimLevel {
                        source: "linecross".to_string(),
                        level: pullback_cap,
                        band_low: signal.block_low,
                        band_high: pullback_cap,
                    }],
                    notes: vec![
                        "sug-info default scalping path".to_string(),
                        format!("linecross-pullback-cap={:.4}", pullback_cap),
                        format!("price={:.4}", current_price),
                        format!("cascade_avg={:.4}", cascade.average_size),
                    ],
                }))
            }
            PositionState::Short if current_price < pullback_cap => {
                Some(EntryGate::Wait(EntryWait {
                    reason: "sideway-linecross-pullback-wait".to_string(),
                    solutions: vec![EntryWaitSolution::ReclaimLevel {
                        source: "linecross".to_string(),
                        level: pullback_cap,
                        band_low: pullback_cap,
                        band_high: signal.block_high,
                    }],
                    notes: vec![
                        "sug-info default scalping path".to_string(),
                        format!("linecross-pullback-cap={:.4}", pullback_cap),
                        format!("price={:.4}", current_price),
                        format!("cascade_avg={:.4}", cascade.average_size),
                    ],
                }))
            }
            _ => None,
        };
        if let Some(wait) = pullback_wait {
            return Some(wait);
        }
    }

    let plan = sideway_entry_plan(snapshot, &signal, cascade)?;
    if let Some(wait) = one_way_lock_gate(state, snapshot, side) {
        return Some(wait);
    }
    if signal.active && !sideways::reentry_ready(&signal, side) {
        return Some(EntryGate::Wait(EntryWait {
            reason: "sideway-opposite-wait".to_string(),
            solutions: vec![EntryWaitSolution::ReclaimLevel {
                source: "sideway-opposite".to_string(),
                level: (signal.block_low + signal.block_high) / 2.0,
                band_low: signal.block_low,
                band_high: signal.block_high,
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

    Some(EntryGate::Open {
        notes: plan.notes,
        reason: EntryOpenReason::Entry(EntryReason::SidewayScalp {
            price: plan.price,
            block_low: signal.block_low,
            block_high: signal.block_high,
            source: format!("{:?}", signal.source),
        }),
    })
}

pub fn sug_entry_plan(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    state: &EngineState,
    cascade: &CascadeSignal,
    side: PositionState,
    force_pullback: bool,
) -> Option<SugEntryPlan> {
    let sug = sug_snapshot.sug_candle?;
    if matches!(side, PositionState::Flat) || cascade.samples < 3 || cascade.stretched {
        return None;
    }

    if !force_pullback
        && direction_lock_gate(
            state,
            snapshot,
            match side {
                PositionState::Long => MarketState::Long,
                PositionState::Short => MarketState::Short,
                PositionState::Flat => MarketState::Sideways,
            },
        )
        .is_some()
    {
        return None;
    }

    let price = sug.close;
    let Some(window) = mrc_window(snapshot, price) else {
        return None;
    };
    let (market_state, entry, target) = match side {
        PositionState::Long if matches!(window.side, levels::MrcPriceZoneSide::Lower) => {
            (MarketState::Long, window.lower, window.upper)
        }
        PositionState::Short if matches!(window.side, levels::MrcPriceZoneSide::Upper) => {
            (MarketState::Short, window.upper, window.lower)
        }
        PositionState::Flat => return None,
        _ => return None,
    };
    let band = mrc_window_band(&window);
    let near_entry = (price - entry).abs() <= band;

    let mut notes = vec![
        "sug-info-scalp".to_string(),
        format!("entry={:.4}", entry),
        format!("tp={:.4}", target),
        format!("band={:.4}", band),
        format!(
            "mrc_window={:.4}-{:.4} dist={:?}",
            window.lower, window.upper, window.distance
        ),
        format!("cascade_samples={}", cascade.samples),
    ];
    if force_pullback {
        notes.push("forced-pullback".to_string());
    }
    if near_entry {
        notes.push("1m-confirmed".to_string());
        Some(SugEntryPlan {
            market_state,
            entry,
            target,
            band,
            notes,
        })
    } else {
        None
    }
}

fn sug_entry_gate(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    state: &EngineState,
    cascade: &CascadeSignal,
    side: PositionState,
) -> Option<EntryGate> {
    let sug = sug_snapshot.sug_candle?;
    if state.cfg.pullback_ratio() <= 0.0 {
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
    }

    if cascade.samples < 3 {
        return Some(EntryGate::Wait(EntryWait {
            reason: "sug-info-1m-confirm-wait".to_string(),
            solutions: vec![EntryWaitSolution::WaitForConfirmation {
                source: "1m-cascade".to_string(),
                direction: match side {
                    PositionState::Long => MarketState::Long,
                    PositionState::Short => MarketState::Short,
                    PositionState::Flat => MarketState::Sideways,
                },
                armed_tick: state.ticks,
                price: sug.close,
            }],
            notes: vec![
                "sug-info default scalping path".to_string(),
                format!("cascade_samples={}", cascade.samples),
            ],
        }));
    }

    let price = sug.close;
    if cascade.stretched {
        return Some(EntryGate::Wait(EntryWait {
            reason: "cascade-stretched-wait".to_string(),
            solutions: vec![EntryWaitSolution::WaitForConfirmation {
                source: "cascade-risk".to_string(),
                direction: match side {
                    PositionState::Long => MarketState::Long,
                    PositionState::Short => MarketState::Short,
                    PositionState::Flat => MarketState::Sideways,
                },
                armed_tick: state.ticks,
                price,
            }],
            notes: vec![
                format!("cascade_strength={:.2}", cascade.strength),
                format!("cascade_samples={}", cascade.samples),
            ],
        }));
    }
    let Some(window) = mrc_window(snapshot, price) else {
        return Some(EntryGate::Wait(EntryWait {
            reason: "mrc-window-unavailable".to_string(),
            solutions: vec![],
            notes: vec!["sug-info default scalping path".to_string()],
        }));
    };
    let (entry, target) = match side {
        PositionState::Long => (window.lower, window.upper),
        PositionState::Short => (window.upper, window.lower),
        PositionState::Flat => return None,
    };
    let two_line_zone = matches!(window.distance, levels::MrcLineDistance::Two);
    let band = mrc_window_band(&window);
    let near_entry = two_line_zone || (price - entry).abs() <= band;

    if near_entry {
        Some(EntryGate::Open {
            notes: vec![
                "sug-info-scalp".to_string(),
                if two_line_zone {
                    "mrc-two-line-eligible".to_string()
                } else {
                    "mrc-one-line-eligible".to_string()
                },
                format!("entry={:.4}", entry),
                format!("tp={:.4}", target),
                format!("band={:.4}", band),
                format!(
                    "mrc_window={:.4}-{:.4} dist={:?}",
                    window.lower, window.upper, window.distance
                ),
                "1m-confirmed".to_string(),
            ],
            reason: EntryOpenReason::Entry(EntryReason::SugInfoScalp { price: entry }),
        })
    } else if state.cfg.pullback_ratio() > 0.0 {
        Some(EntryGate::Wait(EntryWait {
            reason: "mrc-pullback-wait".to_string(),
            solutions: vec![EntryWaitSolution::ReclaimLevel {
                source: "mrc-window".to_string(),
                level: entry,
                band_low: (entry - band).max(0.0),
                band_high: entry + band,
            }],
            notes: vec![
                "sug-info-scalp".to_string(),
                "forced-pullback".to_string(),
                if two_line_zone {
                    "mrc-two-line-eligible".to_string()
                } else {
                    "mrc-one-line-eligible".to_string()
                },
                format!("force_pct={:.2}", state.cfg.pullback_amount),
                format!("entry={:.4}", price),
                format!("pullback_entry={:.4}", entry),
                format!("tp={:.4}", target),
                format!("band={:.4}", band),
                format!(
                    "mrc_window={:.4}-{:.4} dist={:?}",
                    window.lower, window.upper, window.distance
                ),
                "1m-confirmed".to_string(),
            ],
        }))
    } else {
        Some(EntryGate::Wait(EntryWait {
            reason: "mrc-pullback-wait".to_string(),
            solutions: vec![EntryWaitSolution::ReclaimLevel {
                source: "mrc-window".to_string(),
                level: entry,
                band_low: (entry - band).max(0.0),
                band_high: entry + band,
            }],
            notes: vec![
                "sug-info default scalping path".to_string(),
                format!("entry={:.4}", entry),
                format!("tp={:.4}", target),
                format!("price={:.4}", price),
                format!(
                    "mrc_window={:.4}-{:.4} dist={:?}",
                    window.lower, window.upper, window.distance
                ),
                format!("cascade_samples={}", cascade.samples),
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
    if state.cfg.pullback_ratio() > 0.0 && sug_snapshot.sug_candle.is_some() && cascade.samples >= 3
    {
        if let Some(plan) = sug_entry_plan(snapshot, sug_snapshot, state, cascade, side, true) {
            return EntryGate::Open {
                notes: plan.notes,
                reason: EntryOpenReason::Entry(EntryReason::SugInfoScalp { price: plan.entry }),
            };
        }
    }

    if let Some(wait) = one_way_lock_gate(state, snapshot, side) {
        return wait;
    }

    if state.position.side == PositionState::Flat && cascade.samples < 3 {
        return EntryGate::Wait(EntryWait {
            reason: "movement-warmup".to_string(),
            solutions: vec![EntryWaitSolution::WaitForSamples {
                required: 3,
                current: cascade.samples,
            }],
            notes: vec![format!("cascade-samples={}", cascade.samples)],
        });
    }

    if cascade.stretched {
        return EntryGate::Wait(EntryWait {
            reason: "cascade-stretched-wait".to_string(),
            solutions: vec![EntryWaitSolution::WaitForConfirmation {
                source: "cascade-risk".to_string(),
                direction: match side {
                    PositionState::Long => MarketState::Long,
                    PositionState::Short => MarketState::Short,
                    PositionState::Flat => MarketState::Sideways,
                },
                armed_tick: state.ticks,
                price: signal_candle(snapshot, sug_snapshot).close,
            }],
            notes: vec![format!("cascade_strength={:.2}", cascade.strength)],
        });
    }

    if let Some(gate) = sideway_entry_gate(snapshot, state, cascade, side) {
        return gate;
    }

    if let Some(reason) = fast_candle_entry_block(snapshot, sug_snapshot) {
        let candle = signal_candle(snapshot, sug_snapshot);
        let fc = sug_snapshot.fast_candle;
        let fc_phase = fc
            .map(|fc| fc.phase(candle))
            .unwrap_or(FastCandlePhase::Absent);
        return EntryGate::Wait(EntryWait {
            reason: reason.clone(),
            solutions: vec![EntryWaitSolution::FastCandle {
                phase: fc_phase,
                fast_candle_pct: fc.map(|fc| fc.range_pct()).unwrap_or(0.0),
                candle_pct: candle_range_pct(candle),
                gain_zone: snapshot.tf1m.gain_zone(),
                gain_progress_pct: snapshot.tf1m.gain_progress_pct(),
            }],
            notes: vec![reason],
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

    if let Some(gate) = sug_entry_gate(snapshot, sug_snapshot, state, cascade, side) {
        return gate;
    }

    let market_state = match side {
        PositionState::Long => MarketState::Long,
        PositionState::Short => MarketState::Short,
        PositionState::Flat => MarketState::Sideways,
    };
    if let Some(plan) = movement_entry_plan(
        snapshot,
        sug_snapshot,
        cascade,
        &state.cascade_tf,
        market_state,
    ) {
        if let Some(wait) = direction_lock_gate(state, snapshot, plan.market_state) {
            state.movement_entry_pending = None;
            return wait;
        }
        if let Some(wait) = movement_entry_confirmation_gate(state, &plan) {
            return wait;
        }
        return EntryGate::Open {
            notes: plan.notes,
            reason: EntryOpenReason::Entry(EntryReason::SugInfoMovement { price: plan.price }),
        };
    }

    state.movement_entry_pending = None;

    let current_price = signal_candle(snapshot, sug_snapshot).close;
    let plan = if let Some(plan) = candidate_exit_plan(
        snapshot,
        sug_snapshot,
        &state.cascade_tf,
        current_price,
        side,
        state.position.entry_reason.as_ref(),
    ) {
        plan
    } else {
        return EntryGate::Open {
            notes: vec!["no-exit-plan".to_string()],
            reason: EntryOpenReason::Entry(EntryReason::Structural {
                price: current_price,
            }),
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
                plan.take_profit.price(),
                plan.stop_loss.price()
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
                format!("tp={:.4}", plan.take_profit.price()),
                format!("sl={:.4}", plan.stop_loss.price()),
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
                format!("tp={:.4}", plan.take_profit.price()),
                format!("sl={:.4}", plan.stop_loss.price()),
                state.exit_memory.summary(),
            ],
        });
    }

    EntryGate::Open {
        notes: vec![format!(
            "tp={:.4} sl={:.4} tp_reliable={}",
            plan.take_profit.price(),
            plan.stop_loss.price(),
            plan.tp_reliable
        )],
        reason: EntryOpenReason::Entry(EntryReason::Structural {
            price: current_price,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Candle, MrcBands, SugInfoSnapshot, TimeframeSnapshot};
    use crate::state::EngineState;

    fn snapshot() -> IndicatorSnapshot {
        IndicatorSnapshot {
            tf1m: TimeframeSnapshot {
                candle: Candle {
                    open: 100.0,
                    high: 100.2,
                    low: 99.8,
                    close: 100.03,
                },
                mrc: MrcBands {
                    down_big: 90.0,
                    down_outer: 92.0,
                    down_small: 94.0,
                    down_inner: 96.0,
                    mean_line: 100.0,
                    up_inner: 104.0,
                    up_small: 106.0,
                    up_outer: 108.0,
                    up_big: 110.0,
                    current_cross: 0.0,
                    mean_line_slope: 0.0,
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn snapshot_with_close(close: f64) -> IndicatorSnapshot {
        IndicatorSnapshot {
            tf1m: TimeframeSnapshot {
                candle: Candle {
                    open: close,
                    high: close + 0.2,
                    low: close - 0.2,
                    close,
                },
                mrc: MrcBands {
                    down_big: close - 10.0,
                    down_outer: close - 8.0,
                    down_small: close - 6.0,
                    down_inner: close - 4.0,
                    mean_line: close,
                    up_inner: close + 4.0,
                    up_small: close + 6.0,
                    up_outer: close + 8.0,
                    up_big: close + 10.0,
                    current_cross: 0.0,
                    mean_line_slope: 0.0,
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn sug_snapshot() -> SugInfoSnapshot {
        SugInfoSnapshot {
            sug_candle: Some(Candle {
                open: 96.2,
                high: 96.4,
                low: 96.0,
                close: 96.2,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn sug_entry_plan_uses_cascade_samples_as_confirmation() {
        let mut state = EngineState::default();
        let mut cascade = CascadeSignal {
            dir: 1,
            samples: 3,
            ..Default::default()
        };
        let plan = sug_entry_plan(
            &snapshot(),
            &sug_snapshot(),
            &state,
            &cascade,
            PositionState::Long,
            false,
        );
        assert!(plan.is_some());

        cascade.dir = -1;
        assert!(sug_entry_plan(
            &snapshot(),
            &sug_snapshot(),
            &state,
            &cascade,
            PositionState::Long,
            false
        )
        .is_some());

        state.direction_lock.direction = MarketState::Short;
        assert!(sug_entry_plan(
            &snapshot(),
            &sug_snapshot(),
            &state,
            &CascadeSignal {
                dir: 1,
                samples: 3,
                ..Default::default()
            },
            PositionState::Long,
            false
        )
        .is_none());
    }

    #[test]
    fn movement_entry_uses_movement_direction_when_regime_is_sideways() {
        let mut history = crate::state::CascadeState::default();
        history.movement_history[0] = crate::state::CascadeMovement {
            high: 100.2,
            low: 99.2,
        };
        history.movement_history_len = 1;

        let cascade = CascadeSignal {
            dir: 1,
            completed_size: 1.0,
            average_size: 1.0,
            samples: 3,
            strength: 1.0,
            ..Default::default()
        };
        let plan = movement_entry_plan(
            &snapshot(),
            &SugInfoSnapshot::default(),
            &cascade,
            &history,
            MarketState::Sideways,
        )
        .expect("movement should provide a directional plan");

        assert_eq!(plan.market_state, MarketState::Short);
        assert_eq!(plan.movement_direction, 1);
    }

    #[test]
    fn forced_pullback_ignores_opposite_cascade() {
        let mut state = EngineState::default();
        state.direction_lock.direction = MarketState::Short;
        let cascade = CascadeSignal {
            dir: -1,
            samples: 30,
            ..Default::default()
        };
        let plan = sug_entry_plan(
            &snapshot(),
            &sug_snapshot(),
            &state,
            &cascade,
            PositionState::Long,
            true,
        );
        assert!(plan.is_some());
        let plan = plan.expect("forced pullback plan");
        assert!(plan.notes.iter().any(|note| note == "forced-pullback"));
    }

    #[test]
    fn two_line_mrc_zone_opens_immediately() {
        let mut state = EngineState::default();
        let cascade = CascadeSignal {
            dir: 1,
            samples: 30,
            ..Default::default()
        };
        let gate = entry_gate(
            &snapshot_with_close(80.0),
            &sug_snapshot(),
            &cascade,
            &mut state,
            PositionState::Long,
        );

        match gate {
            EntryGate::Open { notes, .. } => {
                assert!(notes.iter().any(|note| note == "mrc-two-line-eligible"));
            }
            EntryGate::Wait(wait) => panic!("unexpected wait: {}", wait.reason),
        }
    }

    #[test]
    fn linecross_waits_for_pullback_then_opens() {
        let signal = sideways::SidewaySignal {
            active: true,
            source: sideways::SidewaySource::LineCross,
            side: PositionState::Long,
            block_low: 99.0,
            block_high: 101.0,
            price_bias: 1,
            reason: "line-active".to_string(),
            ..Default::default()
        };
        let cascade = CascadeSignal {
            average_size: 1.0,
            samples: 30,
            dir: 1,
            ..Default::default()
        };
        assert!(sideway_entry_plan(&snapshot_with_close(100.03), &signal, &cascade).is_none());

        let plan = sideway_entry_plan(&snapshot_with_close(99.50), &signal, &cascade)
            .expect("pullback-ready plan");
        assert!(plan.notes.iter().any(|note| note == "sug-info-pullback"));
    }
}
