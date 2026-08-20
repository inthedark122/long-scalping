use long_scalping::detection::mrc_pattern_classification::{
    classify, classify_all, definition, MrcPatternKind, StaircaseLifecycle, StaircaseTransition,
    MRC_PATTERN_DEFINITIONS,
};
use long_scalping::state::{MrcHistoryEvent, MrcMovementSnapshot};

fn history(lines: &[(u8, f64)]) -> Vec<MrcHistoryEvent> {
    lines
        .iter()
        .enumerate()
        .map(|(index, (line, price))| MrcHistoryEvent {
            line: *line,
            direction: 1,
            now_ms: index as u64 * 60_000,
            market_price: *price,
            mrc_low: 90.0,
            mrc_high: 110.0,
            no_pullback_count: 0,
            reverted_line: None,
            completed_movement: None,
        })
        .collect()
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
        duration_ms: 30_000,
    }
}

#[test]
fn pattern_identifiers_are_stable_and_round_trip() {
    assert_eq!(MRC_PATTERN_DEFINITIONS.len(), 7);

    for pattern in MRC_PATTERN_DEFINITIONS {
        assert_eq!(
            MrcPatternKind::from_str(pattern.kind.as_str()),
            Some(pattern.kind)
        );
        assert!(!pattern.summary.is_empty());
        assert!(!pattern.detection.is_empty());
        assert!(!pattern.prediction.is_empty());
    }
}

#[test]
fn each_pattern_has_a_definition() {
    assert_eq!(
        definition(MrcPatternKind::StairsUp).title,
        "MRC upward staircase"
    );
    assert_eq!(
        definition(MrcPatternKind::FastBreakReturn).kind,
        MrcPatternKind::FastBreakReturn
    );
    assert_eq!(
        definition(MrcPatternKind::OutwardContinuation).kind,
        MrcPatternKind::OutwardContinuation
    );
}

#[test]
fn classification_requires_an_expanding_upward_zigzag() {
    let signal = classify(&history(&[
        (2, 100.0),
        (3, 101.0),
        (2, 99.0),
        (3, 101.0),
        (2, 99.0),
        (3, 101.0),
        (4, 102.0),
    ]))
    .expect("upward zigzag staircase");
    assert_eq!(signal.kind, MrcPatternKind::StairsUp);
    assert_eq!(signal.direction, 1);
    assert_eq!(signal.target_line, 5);
    assert_eq!(signal.steps, 3);
    assert_eq!(signal.sequence, vec![2, 3, 2, 3, 2, 3, 4]);
    assert!((signal.price_move_pct - 2.0).abs() < 1e-12);
    assert!((signal.favorable_move_pct - 2.0).abs() < 1e-12);
    assert!((signal.adverse_move_pct - 1.0).abs() < 1e-12);

    let signal = classify(&history(&[
        (6, 102.0),
        (5, 101.0),
        (6, 103.0),
        (5, 101.0),
        (6, 103.0),
        (5, 101.0),
        (4, 100.0),
    ]))
    .expect("downward zigzag staircase");
    assert_eq!(signal.kind, MrcPatternKind::StairsDown);
    assert_eq!(signal.target_line, 3);
    assert_eq!(signal.steps, 3);
}

#[test]
fn classification_stays_active_during_pullback_and_ends_on_failed_new_extreme() {
    let active = classify(&history(&[
        (2, 100.0),
        (3, 101.0),
        (2, 99.0),
        (3, 101.0),
        (2, 99.0),
        (3, 101.0),
        (4, 102.0),
        (3, 101.0),
    ]))
    .expect("pullback remains part of the upward staircase");
    assert_eq!(active.kind, MrcPatternKind::StairsUp);
    assert_eq!(
        active.staircase_lifecycle,
        Some(StaircaseLifecycle::Pullback)
    );
    assert_eq!(
        active.staircase_transition,
        Some(StaircaseTransition::PullbackWithinStructure)
    );
    assert_eq!(active.source_line, 3);
    assert_eq!(active.target_line, 4);
    assert_eq!(active.sequence, vec![2, 3, 2, 3, 2, 3, 4, 3]);
    assert!((active.price_move_pct - 1.0).abs() < 1e-12);

    let exhausted = classify(&history(&[
        (2, 100.0),
        (3, 101.0),
        (2, 99.0),
        (3, 101.0),
        (2, 99.0),
        (3, 101.0),
        (4, 102.0),
        (3, 101.0),
        (4, 102.0),
    ]));
    let exhausted = exhausted.expect("a stalled upward staircase remains directional");
    assert_eq!(exhausted.kind, MrcPatternKind::StairsUp);
    assert_eq!(
        exhausted.staircase_lifecycle,
        Some(StaircaseLifecycle::Stalled)
    );
    assert_eq!(
        exhausted.staircase_transition,
        Some(StaircaseTransition::AttemptingExtreme)
    );
}

#[test]
fn staircase_invalidates_only_after_the_protected_swing_breaks() {
    let signal = classify(&history(&[
        (2, 100.0),
        (1, 99.0),
        (0, 98.0),
        (1, 99.0),
        (2, 100.0),
        (3, 101.0),
        (4, 102.0),
        (5, 103.0),
        (6, 104.0),
        (5, 103.0),
        (6, 104.0),
        (7, 105.0),
        (6, 104.0),
        (5, 103.0),
        (4, 102.0),
        (3, 101.0),
        (2, 100.0),
    ]))
    .expect("the broken upward staircase remains visible as an invalidated transition");

    assert_eq!(signal.kind, MrcPatternKind::StairsUp);
    assert_eq!(signal.direction, 1);
    assert_eq!(
        signal.staircase_lifecycle,
        Some(StaircaseLifecycle::Invalidated)
    );
    assert_eq!(
        signal.staircase_transition,
        Some(StaircaseTransition::ProtectedSwingBroken)
    );
    assert_eq!(signal.staircase_protected_line, Some(5));
    assert_eq!(signal.staircase_extreme_line, Some(7));
}

#[test]
fn last_confirmed_lower_expansion_keeps_the_stair_short() {
    let signal = classify(&history(&[
        (3, 100.0),
        (4, 101.0),
        (3, 99.0),
        (2, 98.0),
        (3, 99.0),
        (2, 98.0),
        (3, 99.0),
        (4, 101.0),
    ]))
    .expect("the lower staircase should remain active during the pullback");

    assert_eq!(signal.kind, MrcPatternKind::StairsDown);
    assert_eq!(signal.direction, -1);
    assert_eq!(signal.source_line, 4);
    assert_eq!(signal.target_line, 3);
    assert_eq!(signal.sequence, vec![4, 3, 2, 3, 2, 3, 4]);
    assert_eq!(signal.steps, 2);
}

#[test]
fn last_confirmed_upper_expansion_keeps_the_stair_long() {
    let signal = classify(&history(&[
        (5, 100.0),
        (4, 99.0),
        (5, 101.0),
        (6, 102.0),
        (5, 101.0),
        (6, 102.0),
        (5, 101.0),
        (4, 99.0),
    ]))
    .expect("the upper staircase should remain active during the pullback");

    assert_eq!(signal.kind, MrcPatternKind::StairsUp);
    assert_eq!(signal.direction, 1);
    assert_eq!(signal.source_line, 4);
    assert_eq!(signal.target_line, 5);
    assert_eq!(signal.steps, 2);
}

#[test]
fn repeated_equal_zigzags_are_not_a_staircase() {
    let signal = classify(&history(&[
        (2, 100.0),
        (3, 101.0),
        (2, 99.0),
        (3, 101.0),
        (2, 99.0),
        (3, 101.0),
    ]));
    assert_ne!(
        signal.map(|value| value.kind),
        Some(MrcPatternKind::StairsUp)
    );
}

#[test]
fn classification_prioritizes_break_return_and_pullback() {
    let mut fast_break = history(&[(5, 100.0), (6, 102.0), (5, 101.0)]);
    fast_break[1].now_ms = 5 * 60_000;
    fast_break[2].now_ms = 10 * 60_000;
    let signal = classify(&fast_break).expect("fast break-return");
    assert_eq!(signal.kind, MrcPatternKind::FastBreakReturn);
    assert_eq!(signal.direction, -1);
    assert_eq!(signal.target_line, 4);

    let signal =
        classify(&history(&[(5, 100.0), (6, 102.0), (4, 101.0)])).expect("pullback to inner line");
    assert_eq!(signal.kind, MrcPatternKind::PullbackToInner);
    assert_eq!(signal.direction, -1);
    assert_eq!(signal.target_line, 3);
}

#[test]
fn fast_break_return_requires_five_to_fifteen_minutes() {
    let mut too_fast = history(&[(5, 100.0), (6, 102.0), (5, 101.0)]);
    too_fast[1].now_ms = 60_000;
    too_fast[2].now_ms = 4 * 60_000;
    assert!(classify(&too_fast).is_none());

    let mut boundary = history(&[(5, 100.0), (6, 102.0), (5, 101.0)]);
    boundary[1].now_ms = 5 * 60_000;
    boundary[2].now_ms = 15 * 60_000;
    assert_eq!(
        classify(&boundary).map(|signal| signal.kind),
        Some(MrcPatternKind::FastBreakReturn)
    );

    let mut too_slow = history(&[(5, 100.0), (6, 102.0), (5, 101.0)]);
    too_slow[1].now_ms = 10 * 60_000;
    too_slow[2].now_ms = 26 * 60_000;
    assert!(classify(&too_slow).is_none());
}

#[test]
fn classification_distinguishes_sideway_and_trending_cycles() {
    let signal = classify(&history(&[(3, 100.0), (5, 105.0), (3, 99.0), (5, 101.0)]))
        .expect("sideway cycle");
    assert_eq!(signal.kind, MrcPatternKind::RangeCycleSideway);
    assert_eq!(signal.direction, -1);
    assert_eq!(signal.target_line, 3);

    let signal = classify(&history(&[(3, 100.0), (5, 105.0), (3, 110.0), (5, 120.0)]))
        .expect("trending cycle");
    assert_eq!(signal.kind, MrcPatternKind::TrendingCycle);
    assert_eq!(signal.direction, 1);
    assert_eq!(signal.target_line, 6);
}

#[test]
fn classification_records_outward_continuation_from_movement_result() {
    let mut events = history(&[(5, 100.0), (6, 102.0)]);
    events[1].completed_movement = Some(successful_movement());
    let signal = classify(&events).expect("successful outward continuation");
    assert_eq!(signal.kind, MrcPatternKind::OutwardContinuation);
    assert_eq!(signal.direction, 1);
    assert_eq!(signal.target_line, 7);
}

#[test]
fn classification_keeps_overlapping_pattern_matches_for_parallel_learning() {
    let mut events = history(&[(3, 100.0), (4, 101.0), (3, 99.0), (4, 101.0), (5, 102.0)]);
    events[4].completed_movement = Some(successful_movement());

    let kinds = classify_all(&events)
        .into_iter()
        .map(|signal| signal.kind)
        .collect::<Vec<_>>();
    assert!(kinds.contains(&MrcPatternKind::StairsUp));
    assert!(kinds.contains(&MrcPatternKind::OutwardContinuation));
    assert_eq!(
        classify(&events).map(|signal| signal.kind),
        Some(MrcPatternKind::StairsUp)
    );
}
