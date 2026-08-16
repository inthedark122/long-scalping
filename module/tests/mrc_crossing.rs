use long_scalping::detection::mrc_crossing::{self, MrcWaitReason};
use long_scalping::detection::vlines::MrcCrossEvent;
use long_scalping::model::EntryReason;
use long_scalping::state::{MrcCrossingModel, MrcForecastStats, MrcMovementSnapshot};

fn lines() -> [f64; 9] {
    [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0]
}

fn event(line_idx: u8, now_ms: u64) -> MrcCrossEvent {
    MrcCrossEvent {
        line_idx,
        line: lines()[line_idx as usize],
        direction: 1,
        tick: now_ms / 1_000,
        now_ms,
        completed_movement: None,
    }
}

fn learned_model(count: u32) -> MrcCrossingModel {
    let mut model = MrcCrossingModel::default();
    model.pattern_len = 2;
    model.pattern_lines[0] = 3;
    model.pattern_lines[1] = 4;
    model.patterns[3][4][5].next[6] = MrcForecastStats {
        count,
        duration_sum_ms: count as u64 * 2_000,
        favorable_pct_sum: count as f64 * 2.0,
        adverse_pct_sum: count as f64 * -0.5,
    };
    model
}

#[test]
fn required_samples_is_the_minimum_for_the_exact_three_line_pattern() {
    let mut model = learned_model(2);
    let observation = mrc_crossing::observe(&mut model, &[event(5, 10_000)], lines(), 3);

    assert_eq!(observation.samples, 2);
    assert_eq!(observation.wait_reason, Some(MrcWaitReason::Warmup));
    assert!(observation.entry.is_none());
    assert!(model.pending_entry.is_none());

    let mut model = learned_model(3);
    let observation = mrc_crossing::observe(&mut model, &[event(5, 10_000)], lines(), 3);
    let Some(EntryReason::MrcCrossing {
        source_line,
        target_line,
        probability,
        estimated_time_ms,
        favorable_move_pct,
        adverse_move_pct,
        ..
    }) = observation.entry
    else {
        panic!("three learned observations should produce a forecast entry");
    };

    assert_eq!((source_line, target_line), (5, 6));
    assert_eq!(probability, 1.0);
    assert_eq!(estimated_time_ms, 2_000.0);
    assert_eq!(favorable_move_pct, 2.0);
    assert_eq!(adverse_move_pct, -0.5);
}

#[test]
fn a_new_event_updates_the_three_line_pattern_with_movement_metrics() {
    let mut model = MrcCrossingModel::default();
    model.pattern_len = 3;
    model.pattern_lines = [1, 2, 3];
    let crossing = MrcCrossEvent {
        completed_movement: Some(MrcMovementSnapshot {
            start_line: 3,
            start_price: 94.0,
            target_price: 96.0,
            high: 96.0,
            low: 93.0,
            extreme: 96.0,
            direction: 1,
            duration_ms: 4_000,
        }),
        ..event(4, 14_000)
    };

    let observation = mrc_crossing::observe(&mut model, &[crossing], lines(), 3);
    let stats = model.patterns[1][2][3].next[4];

    assert_eq!(stats.count, 1);
    assert_eq!(stats.average_duration_ms(), Some(4_000.0));
    assert_eq!(
        stats.average_favorable_pct(),
        Some((96.0 - 94.0) / 94.0 * 100.0)
    );
    assert_eq!(
        stats.average_adverse_pct(),
        Some((93.0 - 94.0) / 94.0 * 100.0)
    );
    assert_eq!(model.pattern_lines, [2, 3, 4]);
    assert_eq!(observation.wait_reason, Some(MrcWaitReason::Warmup));
}

#[test]
fn pending_entry_is_only_for_the_current_observation() {
    let mut model = learned_model(3);
    let first = mrc_crossing::observe(&mut model, &[event(5, 10_000)], lines(), 3);
    assert!(first.entry.is_some());
    assert!(model.pending_entry.is_some());

    let next = mrc_crossing::observe(&mut model, &[], lines(), 3);
    assert!(next.entry.is_none());
    assert!(model.pending_entry.is_none());
}
