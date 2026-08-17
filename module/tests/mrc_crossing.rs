use long_scalping::detection::mrc_cross_analysis::BarrierState;
use long_scalping::detection::mrc_crossing::{self, MrcWaitReason};
use long_scalping::detection::vlines::MrcCrossEvent;
use long_scalping::model::EntryReason;
use long_scalping::state::{MrcCrossingModel, MrcHistoryEvent, MrcMovementSnapshot};

fn lines() -> [f64; 9] {
    [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0]
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
    model.history.extend([
        MrcHistoryEvent {
            line: 3,
            direction: 1,
            now_ms: base,
            market_price: lines()[3],
            mrc_low: lines()[0],
            mrc_high: lines()[8],
            no_pullback_count: 0,
            reverted_line: None,
            completed_movement: None,
        },
        MrcHistoryEvent {
            line: 4,
            direction: 1,
            now_ms: base + 1_000,
            market_price: lines()[4],
            mrc_low: lines()[0],
            mrc_high: lines()[8],
            no_pullback_count: 0,
            reverted_line: None,
            completed_movement: None,
        },
    ]);
    model.last_line = Some(4);
    model.last_direction = 1;
    model
}

#[test]
fn prediction_uses_observed_adjacent_transition_without_a_sample_gate() {
    let mut model = learned_model(0);
    let observation = mrc_crossing::observe(&mut model, &[event(5, 30_000)], lines());

    assert_eq!(observation.samples, 0);
    assert_eq!(
        observation.wait_reason,
        Some(MrcWaitReason::NoReliablePrediction)
    );
    assert!(observation.entry.is_none());
    assert!(model.current_entry.is_none());

    let mut model = learned_model(3);
    let observation = mrc_crossing::observe(&mut model, &[event(5, 40_000)], lines());
    let Some(EntryReason::MrcCrossing {
        source_line,
        target_line,
        pattern,
        pattern_samples,
        target_samples,
        movement_samples,
        successful_moves,
        failed_moves,
        probability,
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
    assert_eq!(probability, 1.0);
    assert_eq!(movement_success_rate, 1.0);
    assert_eq!(estimated_time_ms, 2_000.0);
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
    crossing.price = 103.5;
    crossing.mrc_low = 89.5;
    crossing.mrc_high = 111.0;

    let observation = mrc_crossing::observe(&mut model, &[crossing], lines());
    let event = &observation.events[0];
    assert_eq!(event.market_price, 103.5);
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
    assert_eq!(market_price, 103.5);
    assert_eq!(mrc_min, 89.5);
    assert_eq!(mrc_max, 111.0);
    assert!(expected_price_low < expected_price_high);
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
        Default::default(),
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
fn prediction_keeps_support_resistance_in_probability() {
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

    let observation = mrc_crossing::observe(&mut model, &[event(5, 40_000)], lines());
    let Some(EntryReason::MrcCrossing {
        target_line,
        probability,
        next_line_crosses,
        support_resistance_moves,
        ..
    }) = observation.entry
    else {
        panic!("the continuation route should remain tradable");
    };

    assert_eq!(target_line, 6);
    assert!((probability - (2.0 / 3.0)).abs() < 1e-12);
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

    let observation = mrc_crossing::observe(&mut model, &[event(5, 70_000)], lines());
    let Some(EntryReason::MrcCrossing {
        target_line,
        context_samples,
        no_pullback_count,
        reverted_line,
        probability,
        ..
    }) = observation.entry
    else {
        panic!("matching pullback/revert context should produce a prediction");
    };

    assert_eq!(target_line, 6);
    assert_eq!(context_samples, 2);
    assert_eq!(no_pullback_count, 0);
    assert_eq!(reverted_line, None);
    assert_eq!(probability, 1.0);
}

#[test]
fn pattern_prediction_follows_fast_break_return_direction() {
    let mut model = MrcCrossingModel::default();
    for (index, target) in [4, 6].into_iter().enumerate() {
        let base = index as u64 * 10_000;
        model.history.extend([
            MrcHistoryEvent {
                line: 5,
                direction: -1,
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
                completed_movement: Some(successful_short_movement()),
            },
        ]);
    }

    let mut crossing = event(5, 40_000);
    crossing.price = 104.5;
    let observation = mrc_crossing::observe(&mut model, &[crossing], lines());
    let Some(entry) = observation.entry else {
        panic!("a learned fast break-return should be tradable");
    };
    assert_eq!(mrc_crossing::reason_direction(&entry), -1);
    let EntryReason::MrcCrossing {
        price,
        target_price,
        stop_price,
        ..
    } = &entry
    else {
        panic!("the core MRC entry should include directional exits");
    };
    assert_eq!(*price, 104.5);
    assert!(target_price < price);
    assert!(stop_price > price);
    let event = &observation.events[0];
    assert_eq!(event.target_line, Some(4));
    assert_eq!(
        event.pattern_kind.map(|kind| kind.as_str()),
        Some("FAST_BREAK_RETURN")
    );
    assert_eq!(event.take_profit_line, Some(4));
    assert_eq!(event.stop_loss_line, Some(6));
    assert_eq!(event.target_price, Some(lines()[4]));
    assert_eq!(event.stop_price, Some(lines()[6]));
    assert!((event.probability.unwrap() - 0.5).abs() < 1e-12);
    assert_eq!(event.next_line_crosses, 1);
    assert_eq!(event.support_resistance_moves, 1);
}

#[test]
fn current_entry_is_only_for_the_current_observation() {
    let mut model = learned_model(3);
    let first = mrc_crossing::observe(&mut model, &[event(5, 40_000)], lines());
    assert!(first.entry.is_some());
    assert!(model.current_entry.is_some());

    let next = mrc_crossing::observe(&mut model, &[], lines());
    assert!(next.entry.is_none());
    assert!(model.current_entry.is_none());
}
