use long_scalping::detection::mrc_pattern_classification::{
    classify, classify_all, definition, MrcPatternKind, MRC_PATTERN_DEFINITIONS,
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
fn classification_uses_local_shapes_instead_of_full_paths() {
    let signal =
        classify(&history(&[(3, 100.0), (4, 101.0), (5, 102.0)])).expect("upward local staircase");
    assert_eq!(signal.kind, MrcPatternKind::StairsUp);
    assert_eq!(signal.direction, 1);
    assert_eq!(signal.target_line, 6);
    assert_eq!(signal.steps, 2);

    let signal = classify(&history(&[(5, 102.0), (4, 101.0), (3, 100.0)]))
        .expect("downward local staircase");
    assert_eq!(signal.kind, MrcPatternKind::StairsDown);
    assert_eq!(signal.direction, -1);
    assert_eq!(signal.target_line, 2);
}

#[test]
fn classification_prioritizes_break_return_and_pullback() {
    let signal =
        classify(&history(&[(5, 100.0), (6, 102.0), (5, 101.0)])).expect("fast break-return");
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
    let mut events = history(&[(3, 100.0), (4, 101.0), (5, 102.0)]);
    events[2].completed_movement = Some(successful_movement());

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
