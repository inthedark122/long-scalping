use long_scalping::detection::mrc_cross_analysis::{self, BarrierState};
use long_scalping::detection::mrc_crossing::{self, MrcPrediction, MrcWaitReason};
use long_scalping::detection::mrc_pattern_classification::MrcPatternKind;
use long_scalping::detection::vlines::MrcCrossEvent;
use long_scalping::model::EntryReason;
use long_scalping::state::{
    MrcCrossingModel, MrcCrossingState, MrcForecastStats, MrcHistoryEvent, MrcMovementSnapshot,
};

fn lines() -> [f64; 9] {
    [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0]
}

#[test]
fn movement_outcome_matches_route_statistics_rule() {
    assert_eq!(
        mrc_cross_analysis::movement_outcome(2.0, -1.0),
        Some(("profitable", 2.0))
    );
    assert_eq!(
        mrc_cross_analysis::movement_outcome(1.0, -2.0),
        Some(("negative", -2.0))
    );
    assert_eq!(mrc_cross_analysis::movement_outcome(f64::NAN, 0.0), None);
}

fn event(line_idx: u8, now_ms: u64) -> MrcCrossEvent {
    MrcCrossEvent {
        line_idx,
        line: lines()[line_idx as usize],
        price: lines()[line_idx as usize],
        mrc_low: lines()[0],
        mrc_high: lines()[8],
        direction: 1,
        tick: now_ms / 1_000,
        now_ms,
        no_pullback_count: 0,
        reverted_line: None,
        completed_movement: None,
    }
}

fn successful_movement() -> MrcMovementSnapshot {
    MrcMovementSnapshot {
        start_line: 5,
        start_price: 104.0,
        market_start_price: 104.0,
        target_price: 106.0,
        target_low: 106.0,
        target_high: 106.0,
        mrc_low: 90.0,
        mrc_high: 110.0,
        high: 106.0,
        low: 103.0,
        extreme: 106.0,
        direction: 1,
        duration_ms: 2_000,
    }
}

#[test]
fn pattern_performance_ledger_tracks_realized_net_results() {
    let mut model = MrcCrossingModel::default();

    mrc_crossing::record_pattern_result(&mut model, MrcPatternKind::StairsUp, 1.25);
    mrc_crossing::record_pattern_result(&mut model, MrcPatternKind::StairsUp, -0.25);

    let performance = model.pattern_performance[MrcPatternKind::StairsUp.index()];
    assert_eq!(performance.trades, 2);
    assert_eq!(performance.wins, 1);
    assert_eq!(performance.pnl_sum_pct, 1.0);
    assert_eq!(performance.last_pnl_pct, -0.25);
    assert!(performance.ema_pnl_pct.is_finite());
}

#[test]
fn prediction_history_survives_local_pattern_window() {
    let mut model = MrcCrossingModel::default();
    for index in 0..20u64 {
        let mut crossing = event(5, (index + 1) * 10_000);
        crossing.direction = if index % 2 == 0 { 1 } else { -1 };
        mrc_crossing::observe(&mut model, &[crossing], lines());
    }

    assert_eq!(model.prediction_history.len(), 20);
    assert!(model.history.len() < model.prediction_history.len());
}

fn successful_short_movement() -> MrcMovementSnapshot {
    MrcMovementSnapshot {
        start_line: 5,
        start_price: 104.0,
        market_start_price: 104.0,
        target_price: 102.0,
        target_low: 102.0,
        target_high: 102.0,
        mrc_low: 90.0,
        mrc_high: 110.0,
        high: 105.0,
        low: 102.0,
        extreme: 102.0,
        direction: -1,
        duration_ms: 2_000,
    }
}

fn learned_model(samples: usize) -> MrcCrossingModel {
    let mut model = MrcCrossingModel::default();
    for sample in 0..samples {
        let base = sample as u64 * 10_000;
        for (offset, line) in [3, 4, 5, 6].into_iter().enumerate() {
            model.history.push(MrcHistoryEvent {
                line,
                direction: 1,
                now_ms: base + offset as u64 * 1_000,
                market_price: lines()[line as usize],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: (line == 6).then_some(successful_movement()),
            });
        }
    }
    let base = samples as u64 * 10_000;
    model
        .history
        .extend(
            [3, 4, 3, 4]
                .into_iter()
                .enumerate()
                .map(|(offset, line)| MrcHistoryEvent {
                    line,
                    direction: if offset % 2 == 0 { 1 } else { -1 },
                    now_ms: base + offset as u64 * 1_000,
                    market_price: lines()[line as usize],
                    mrc_low: lines()[0],
                    mrc_high: lines()[8],
                    no_pullback_count: 0,
                    reverted_line: None,
                    completed_movement: None,
                }),
        );
    model.last_line = Some(4);
    model.last_direction = 1;
    model
}

fn forecast_model() -> MrcCrossingModel {
    let mut model = learned_model(3);
    model.history.push(MrcHistoryEvent {
        line: 5,
        direction: 1,
        now_ms: 40_000,
        market_price: lines()[5],
        mrc_low: lines()[0],
        mrc_high: lines()[8],
        no_pullback_count: 0,
        reverted_line: None,
        completed_movement: None,
    });
    model.last_line = Some(5);
    model.last_direction = 1;
    model
}

#[test]
fn prediction_uses_observed_adjacent_transition_without_a_sample_gate() {
    let mut model = learned_model(0);
    let mut first = event(5, 30_000);
    first.price = 103.0;
    let observation = mrc_crossing::observe(&mut model, &[first], lines());

    assert_eq!(observation.samples, 0);
    assert!(observation.entry.is_some());
    assert!(observation.selected_pattern.is_some());
    assert!(model.current_entry.is_some());

    let mut model = learned_model(3);
    let mut second = event(5, 40_000);
    second.price = 103.0;
    let observation = mrc_crossing::observe(&mut model, &[second], lines());
    let Some(EntryReason::MrcCrossing {
        source_line,
        target_line,
        pattern,
        pattern_samples,
        target_samples,
        movement_samples,
        successful_moves,
        failed_moves,
        movement_success_rate,
        estimated_time_ms,
        ..
    }) = observation.entry
    else {
        panic!("three historical cases should produce a forecast entry");
    };

    assert_eq!((source_line, target_line), (5, 6));
    assert_eq!(pattern.last(), Some(&5));
    assert!(pattern.len() >= 2);
    assert_eq!(pattern_samples, 3);
    assert_eq!(target_samples, 3);
    assert_eq!(movement_samples, 3);
    assert_eq!(successful_moves, 3);
    assert_eq!(failed_moves, 0);
    assert_eq!(movement_success_rate, 1.0);
    assert_eq!(estimated_time_ms, 2_000.0);
}

#[test]
fn stair_entry_stays_on_the_crossed_line_and_stop_stays_behind_entry() {
    let mut model = learned_model(3);
    let mut crossing = event(5, 40_000);
    crossing.price = 104.5;
    let analysis_lines = [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 114.0, 116.0, 118.0];

    let observation = mrc_crossing::observe(&mut model, &[crossing], analysis_lines);
    let Some(entry) = observation.entry else {
        panic!("the learned long transition should produce an entry");
    };
    let EntryReason::MrcCrossing {
        price, stop_price, ..
    } = entry;

    assert_eq!(price, 104.0);
    assert_eq!(stop_price, 100.0);
    assert_eq!(observation.events[0].stop_loss_line, Some(4));
    assert_ne!(observation.events[0].stop_loss_line, Some(5));
}

#[test]
fn pattern_prediction_requires_supporting_target_evidence() {
    let mut model = MrcCrossingModel::default();
    for (index, target) in [6, 6, 7, 6].into_iter().enumerate() {
        let base = index as u64 * 10_000;
        model.history.extend([
            MrcHistoryEvent {
                line: index as u8,
                direction: 1,
                now_ms: base,
                market_price: lines()[index],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: None,
            },
            MrcHistoryEvent {
                line: 5,
                direction: 1,
                now_ms: base + 1_000,
                market_price: lines()[5],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: None,
            },
            MrcHistoryEvent {
                line: target,
                direction: 1,
                now_ms: base + 2_000,
                market_price: lines()[target as usize],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: (target == 6).then_some(successful_movement()),
            },
        ]);
    }

    let observation = mrc_crossing::observe(&mut model, &[event(5, 50_000)], lines());
    assert!(observation.entry.is_none());
    assert_eq!(
        observation.wait_reason,
        Some(MrcWaitReason::NoReliablePrediction)
    );
}

#[test]
fn crossing_observation_keeps_market_price_and_mrc_range() {
    let mut model = learned_model(3);
    let mut crossing = event(5, 40_000);
    crossing.price = 103.0;
    crossing.mrc_low = 89.5;
    crossing.mrc_high = 111.0;

    let observation = mrc_crossing::observe(&mut model, &[crossing], lines());
    let event = &observation.events[0];
    assert_eq!(event.market_price, 103.0);
    assert_eq!(event.mrc_min, 89.5);
    assert_eq!(event.mrc_max, 111.0);

    let Some(EntryReason::MrcCrossing {
        market_price,
        mrc_min,
        mrc_max,
        expected_price_low,
        expected_price_high,
        ..
    }) = observation.entry
    else {
        panic!("the observed adjacent transition should produce an entry");
    };
    assert_eq!(market_price, 103.0);
    assert_eq!(mrc_min, 89.5);
    assert_eq!(mrc_max, 111.0);
    assert!(expected_price_low < expected_price_high);
}

fn stair_prediction(kind: MrcPatternKind, direction: i8, target_line: u8) -> MrcPrediction {
    MrcPrediction {
        pattern_kind: kind,
        pattern_direction: direction,
        pattern_steps: 3,
        pattern_elapsed_ms: 7 * 60_000,
        stair_sequence: if direction > 0 {
            vec![5, 4, 5, 6, 5]
        } else {
            vec![5, 4, 5, 6, 5, 4, 3]
        },
        stair_price_move_pct: direction as f64,
        stair_favorable_move_pct: 2.0,
        stair_adverse_move_pct: 1.0,
        pattern: vec![target_line],
        elapsed_ms: vec![7 * 60_000],
        target_line,
        samples: 1,
        stats: MrcForecastStats {
            count: 1,
            movement_samples: 1,
            duration_sum_ms: 7 * 60_000,
            favorable_pct_sum: 2.0,
            adverse_pct_sum: 1.0,
            outcome_pct_sum: 1.0,
            success_count: 1,
            ..Default::default()
        },
        line_stats: {
            let mut stats = [MrcForecastStats::default(); 9];
            stats[target_line as usize] = MrcForecastStats {
                count: 1,
                movement_samples: 1,
                duration_sum_ms: 7 * 60_000,
                favorable_pct_sum: 2.0,
                adverse_pct_sum: 1.0,
                outcome_pct_sum: 1.0,
                success_count: 1,
                ..Default::default()
            };
            stats
        },
        next_line_crosses: 1,
        support_resistance_moves: 0,
        price_only_moves: 0,
        context_samples: 1,
        failed_break_line: None,
        failed_break_direction: 0,
    }
}

#[test]
fn staircase_entry_stays_at_the_crossed_line() {
    let mut short_event = event(3, 40_000);
    short_event.direction = -1;
    let short = mrc_cross_analysis::build(
        &short_event,
        Some(&stair_prediction(MrcPatternKind::StairsDown, -1, 2)),
        lines(),
        &MrcCrossingState::default(),
        (0.0, 0.0, 0.0, 0.0),
    );
    assert_eq!(short.market_price, 96.0);
    assert_eq!(short.effective_entry_price, Some(96.0));
    assert_eq!(short.target_price, Some(94.0));
    assert_eq!(short.stop_price, Some(100.0));
    assert!(short.tradeable());

    let long_event = event(5, 50_000);
    let long = mrc_cross_analysis::build(
        &long_event,
        Some(&stair_prediction(MrcPatternKind::StairsUp, 1, 6)),
        lines(),
        &MrcCrossingState::default(),
        (0.0, 0.0, 0.0, 0.0),
    );
    assert_eq!(long.market_price, 104.0);
    assert_eq!(long.effective_entry_price, Some(104.0));
    assert_eq!(long.target_price, Some(106.0));
    assert_eq!(long.stop_price, Some(100.0));
    assert!(long.tradeable());
}

#[test]
fn reverted_opposite_line_is_a_barrier_until_it_is_crossed() {
    let mut model = learned_model(3);
    let crossing = long_scalping::state::MrcCrossingState {
        reverted_short_line: Some(6),
        ..Default::default()
    };
    let observation = mrc_crossing::observe_with_context(
        &mut model,
        &[event(5, 40_000)],
        lines(),
        &crossing,
        (0.0, 0.0, 0.0, 0.0),
    );

    assert_eq!(observation.events[0].barrier_line, Some(6));
    assert_eq!(
        observation.events[0].barrier_state,
        BarrierState::Approaching
    );
    assert!(observation.entry.is_none());
    assert_eq!(observation.wait_reason, Some(MrcWaitReason::Crossing));
}

#[test]
fn movement_data_is_stored_with_the_crossing_that_realized_it() {
    let mut model = MrcCrossingModel::default();
    let crossing = MrcCrossEvent {
        completed_movement: Some(successful_movement()),
        ..event(4, 14_000)
    };

    let observation = mrc_crossing::observe(&mut model, &[crossing], lines());

    assert_eq!(model.history.len(), 1);
    assert_eq!(
        model.history[0].completed_movement,
        crossing.completed_movement
    );
    assert_eq!(
        observation.wait_reason,
        Some(MrcWaitReason::NoReliablePrediction)
    );
}

#[test]
fn prediction_keeps_support_resistance_counts() {
    let mut model = MrcCrossingModel::default();
    for (index, target) in [6, 6, 4].into_iter().enumerate() {
        let base = index as u64 * 10_000;
        model.history.extend([
            MrcHistoryEvent {
                line: 5,
                direction: 1,
                now_ms: base,
                market_price: lines()[5],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: None,
            },
            MrcHistoryEvent {
                line: target,
                direction: 1,
                now_ms: base + 1_000,
                market_price: lines()[target as usize],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: Some(successful_movement()),
            },
        ]);
    }
    // Keep the latest observation out of the A -> B -> A classifier so this
    // test continues to exercise the generic transition statistics.
    model.history.push(MrcHistoryEvent {
        line: 4,
        direction: -1,
        now_ms: 30_000,
        market_price: lines()[4],
        mrc_low: lines()[0],
        mrc_high: lines()[8],
        no_pullback_count: 0,
        reverted_line: None,
        completed_movement: None,
    });

    let mut crossing = event(5, 40_000);
    crossing.price = 103.0;
    crossing.completed_movement = Some(successful_movement());
    let observation = mrc_crossing::observe(&mut model, &[crossing], lines());
    let Some(EntryReason::MrcCrossing {
        target_line,
        next_line_crosses,
        support_resistance_moves,
        ..
    }) = observation.entry
    else {
        panic!("the continuation route should remain tradable");
    };

    assert_eq!(target_line, 6);
    assert_eq!(next_line_crosses, 2);
    assert_eq!(support_resistance_moves, 1);
}

#[test]
fn prediction_uses_pullback_and_revert_context_when_available() {
    let mut model = MrcCrossingModel::default();
    for index in 0..2u64 {
        let base = index * 10_000;
        model.history.extend([
            MrcHistoryEvent {
                line: 5,
                direction: 1,
                now_ms: base,
                market_price: lines()[5],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: None,
            },
            MrcHistoryEvent {
                line: 6,
                direction: 1,
                now_ms: base + 1_000,
                market_price: lines()[6],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: Some(successful_movement()),
            },
        ]);
    }
    for index in 0..3u64 {
        let base = 30_000 + index * 10_000;
        model.history.extend([
            MrcHistoryEvent {
                line: 5,
                direction: 1,
                now_ms: base,
                market_price: lines()[5],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 2,
                reverted_line: Some(4),
                completed_movement: None,
            },
            MrcHistoryEvent {
                line: 4,
                direction: 1,
                now_ms: base + 1_000,
                market_price: lines()[4],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 2,
                reverted_line: Some(4),
                completed_movement: Some(successful_movement()),
            },
        ]);
    }
    model.history.push(MrcHistoryEvent {
        line: 4,
        direction: -1,
        now_ms: 60_000,
        market_price: lines()[4],
        mrc_low: lines()[0],
        mrc_high: lines()[8],
        no_pullback_count: 0,
        reverted_line: None,
        completed_movement: None,
    });

    let mut crossing = event(5, 70_000);
    crossing.price = 103.0;
    crossing.completed_movement = Some(successful_movement());
    let observation = mrc_crossing::observe(&mut model, &[crossing], lines());
    let Some(EntryReason::MrcCrossing {
        target_line,
        context_samples,
        no_pullback_count,
        reverted_line,
        ..
    }) = observation.entry
    else {
        panic!("matching pullback/revert context should produce a prediction");
    };

    assert_eq!(target_line, 6);
    assert_eq!(context_samples, 2);
    assert_eq!(no_pullback_count, 0);
    assert_eq!(reverted_line, None);
}

#[test]
fn pattern_prediction_uses_exact_break_return_rejection() {
    let mut model = MrcCrossingModel::default();
    // The same failed upward break is observed four times and is rejected
    // downward every time.
    for (index, next_line) in [4, 4, 4, 4].into_iter().enumerate() {
        let base = index as u64 * 30 * 60_000;
        model.history.extend([
            MrcHistoryEvent {
                line: 5,
                direction: 1,
                now_ms: base,
                market_price: lines()[5],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: None,
            },
            MrcHistoryEvent {
                line: 6,
                direction: 1,
                now_ms: base + 5 * 60_000,
                market_price: lines()[6],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: None,
            },
            MrcHistoryEvent {
                line: 5,
                direction: 1,
                now_ms: base + 10 * 60_000,
                market_price: lines()[5],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: None,
            },
            MrcHistoryEvent {
                line: next_line,
                direction: 1,
                now_ms: base + 11 * 60_000,
                market_price: lines()[next_line as usize],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: Some(if next_line > 5 {
                    successful_movement()
                } else {
                    successful_short_movement()
                }),
            },
        ]);
    }
    // Start the current failed break after the historical outcomes.
    model.history.extend([
        MrcHistoryEvent {
            line: 5,
            direction: 1,
            now_ms: 4 * 30 * 60_000,
            market_price: lines()[5],
            mrc_low: lines()[0],
            mrc_high: lines()[8],
            no_pullback_count: 0,
            reverted_line: None,
            completed_movement: None,
        },
        MrcHistoryEvent {
            line: 6,
            direction: 1,
            now_ms: 4 * 30 * 60_000 + 5 * 60_000,
            market_price: lines()[6],
            mrc_low: lines()[0],
            mrc_high: lines()[8],
            no_pullback_count: 0,
            reverted_line: None,
            completed_movement: None,
        },
    ]);

    let mut crossing = event(5, 4 * 30 * 60_000 + 10 * 60_000);
    crossing.price = 104.5;
    let observation = mrc_crossing::observe(&mut model, &[crossing], lines());
    assert!(observation.entry.is_none());
    assert!(observation.selected_candidate_index.is_none());
    let event = &observation.events[0];
    assert_eq!(event.target_line, Some(4));
    assert_eq!(
        event.pattern_kind.map(|kind| kind.as_str()),
        Some("FAST_BREAK_RETURN")
    );
    assert_eq!(event.take_profit_line, Some(4));
    assert_eq!(event.stop_loss_line, Some(6));
    assert_eq!(event.barrier_line, Some(6));
    assert_eq!(event.barrier_state, BarrierState::Rejected);
    assert_eq!(event.target_price, Some(lines()[4]));
    assert_eq!(event.stop_price, Some(lines()[6]));
    assert_eq!(event.next_line_crosses, 0);
    assert_eq!(event.support_resistance_moves, 4);
}

#[test]
fn pattern_prediction_follows_exact_break_return_continuation() {
    let mut model = MrcCrossingModel::default();
    for (index, next_line) in [6, 6, 6, 6].into_iter().enumerate() {
        let base = index as u64 * 30 * 60_000;
        model.history.extend([
            MrcHistoryEvent {
                line: 5,
                direction: 1,
                now_ms: base,
                market_price: lines()[5],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: None,
            },
            MrcHistoryEvent {
                line: 6,
                direction: 1,
                now_ms: base + 5 * 60_000,
                market_price: lines()[6],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: None,
            },
            MrcHistoryEvent {
                line: 5,
                direction: 1,
                now_ms: base + 10 * 60_000,
                market_price: lines()[5],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: None,
            },
            MrcHistoryEvent {
                line: next_line,
                direction: 1,
                now_ms: base + 11 * 60_000,
                market_price: lines()[next_line as usize],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: Some(if next_line > 5 {
                    successful_movement()
                } else {
                    successful_short_movement()
                }),
            },
        ]);
        if index < 3 {
            model.history.push(MrcHistoryEvent {
                line: 3,
                direction: 1,
                now_ms: base + 12 * 60_000,
                market_price: lines()[3],
                mrc_low: lines()[0],
                mrc_high: lines()[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: None,
            });
        }
    }

    let mut crossing = event(5, 3 * 30 * 60_000 + 20 * 60_000);
    crossing.price = 103.0;
    let observation = mrc_crossing::observe(&mut model, &[crossing], lines());
    let event = &observation.events[0];
    assert!(observation.entry.is_none());
    assert_eq!(event.trade_direction, 1);
    assert_eq!(event.target_line, Some(6));
    assert_eq!(event.take_profit_line, Some(6));
    assert_eq!(event.stop_loss_line, Some(4));
    assert_eq!(event.barrier_line, Some(6));
    assert_eq!(event.barrier_state, BarrierState::Approaching);
    assert_eq!(event.next_line_crosses, 4);
    assert_eq!(event.support_resistance_moves, 0);
}

#[test]
fn fast_break_return_without_exact_outcome_does_not_trade() {
    let mut model = MrcCrossingModel::default();
    model.history.extend([
        MrcHistoryEvent {
            line: 5,
            direction: 1,
            now_ms: 5 * 60_000,
            market_price: lines()[5],
            mrc_low: lines()[0],
            mrc_high: lines()[8],
            no_pullback_count: 0,
            reverted_line: None,
            completed_movement: None,
        },
        MrcHistoryEvent {
            line: 4,
            direction: -1,
            now_ms: 10 * 60_000,
            market_price: lines()[4],
            mrc_low: lines()[0],
            mrc_high: lines()[8],
            no_pullback_count: 0,
            reverted_line: None,
            completed_movement: None,
        },
    ]);

    // The latest A -> B -> A is visible, but no historical A -> B -> A -> X
    // outcome exists for this exact break.
    let observation = mrc_crossing::observe(&mut model, &[event(5, 11 * 60_000)], lines());
    assert!(observation.entry.is_none());
    assert_eq!(
        observation.wait_reason,
        Some(MrcWaitReason::NoReliablePrediction)
    );
}

#[test]
fn mrc_levels_reject_reward_smaller_than_stop_risk() {
    assert!(!mrc_crossing::valid_price_levels(
        -1, 3.61, 3.602868, 3.689655, None,
    ));
    assert!(mrc_crossing::valid_price_levels(
        -1, 3.61, 3.52, 3.689655, None,
    ));
}

#[test]
fn staircase_continuation_is_allowed_when_adjacent_reward_is_smaller_than_stop_risk() {
    let mut model = learned_model(3);
    let mut crossing = event(5, 40_000);
    crossing.price = 104.5;

    let observation = mrc_crossing::observe(&mut model, &[crossing], lines());
    let Some(EntryReason::MrcCrossing {
        pattern_kind,
        target_price,
        stop_price,
        ..
    }) = observation.entry
    else {
        panic!("a detected staircase should continue to the next MRC line");
    };

    assert_eq!(pattern_kind, "STAIRS_UP");
    assert_eq!((target_price, stop_price), (106.0, 100.0));
}

#[test]
fn current_entry_is_only_for_the_current_observation() {
    let mut model = learned_model(3);
    let mut crossing = event(5, 40_000);
    crossing.price = 103.0;
    let first = mrc_crossing::observe(&mut model, &[crossing], lines());
    assert!(first.entry.is_some());
    assert!(model.current_entry.is_some());

    let next = mrc_crossing::observe(&mut model, &[], lines());
    assert!(next.entry.is_none());
    assert!(model.current_entry.is_none());
}

#[test]
fn forecast_enters_when_predicted_crossing_is_reached() {
    let mut model = forecast_model();
    let forecast_lines = [90.0, 98.0, 99.0, 103.0, 104.0, 105.0, 107.0, 112.0, 114.0];
    let observation = mrc_crossing::observe_with_forecast(
        &mut model,
        &[],
        forecast_lines,
        &Default::default(),
        (0.0, 0.0, 0.0, 0.0),
        107.0,
        50_000,
    );

    let Some(EntryReason::MrcCrossing {
        price,
        target_price,
        stop_price,
        source_line,
        target_line,
        ..
    }) = observation.entry
    else {
        panic!("the learned route should be tradable before the next crossing");
    };

    assert_eq!(price, 105.0);
    assert_eq!((source_line, target_line), (5, 6));
    assert_eq!(target_price, 107.0);
    assert_eq!(stop_price, 104.0);
    assert!(observation.forecast.is_some());
    assert!(observation.events.is_empty());
}

#[test]
fn forecast_staircase_continues_even_when_learned_move_is_below_next_line() {
    let mut model = forecast_model();
    let forecast_lines = [90.0, 98.0, 99.0, 103.0, 104.0, 105.0, 107.0, 112.0, 114.0];
    let observation = mrc_crossing::observe_with_forecast(
        &mut model,
        &[],
        forecast_lines,
        &Default::default(),
        (0.0, 0.0, 0.0, 0.0),
        104.9,
        50_000,
    );

    assert!(observation.forecast.is_some());
    assert!(observation.entry.is_some());
    assert!(observation.selected_pattern.is_some());
    assert!(model.current_entry.is_some());
}

#[test]
fn mrc_exit_levels_follow_current_lines_and_fill_price() {
    let mut model = forecast_model();
    let original_lines = [90.0, 98.0, 99.0, 103.0, 104.0, 105.0, 107.0, 112.0, 114.0];
    let observation = mrc_crossing::observe_with_forecast(
        &mut model,
        &[],
        original_lines,
        &Default::default(),
        (0.0, 0.0, 0.0, 0.0),
        107.0,
        50_000,
    );
    let Some(reason) = observation.entry else {
        panic!("the forecast should produce an MRC reason");
    };
    let live_lines = [90.0, 98.0, 99.0, 103.5, 104.5, 105.5, 107.5, 112.0, 114.0];
    assert_eq!(
        mrc_crossing::exit_levels(&reason, live_lines, 105.0),
        Some((107.5, 104.5))
    );
}
