use long_scalping::detection::mrc_touch_learning::{
    MrcTouchLearningState, TouchMatchScope, TouchTimeBucket,
};

fn lines() -> [f64; 9] {
    [90.0, 92.5, 95.0, 97.5, 100.0, 102.5, 105.0, 107.5, 110.0]
}

#[test]
fn exact_mean_touch_learns_the_next_touched_line_and_time_bucket() {
    let bands = lines();
    let mut learning = MrcTouchLearningState::default();
    for (time_ms, price) in [
        (1_000, 100.0),
        (61_000, 102.5),
        (121_000, 100.0),
        (181_000, 102.5),
        (241_000, 100.0),
        (301_000, 102.5),
        (361_000, 100.0),
    ] {
        learning.update(bands, price, time_ms, bands, 1);
    }

    let signal = learning.signal;
    assert_eq!(signal.current_line, Some(4));
    assert!(signal.exact_touch);
    assert_eq!(signal.learned_direction, 1);
    assert_eq!(signal.learned_destination, Some(5));
    assert_eq!(signal.learned_samples, 3);
    assert_eq!(signal.learned_time_bucket, TouchTimeBucket::Normal);
    assert_eq!(signal.match_scope, TouchMatchScope::Exact);
    assert!(signal.reliable);
}

#[test]
fn different_five_minute_context_falls_back_to_slope_history() {
    let bands = lines();
    let mut learning = MrcTouchLearningState::default();
    for (time_ms, price) in [
        (1_000, 102.5),
        (10_000, 100.0),
        (20_000, 102.5),
        (30_000, 100.0),
        (40_000, 102.5),
        (50_000, 100.0),
        (60_000, 102.5),
    ] {
        learning.update(bands, price, time_ms, bands, 1);
    }

    let shifted_slow_lines = [95.0, 97.5, 100.0, 102.5, 105.0, 107.5, 110.0, 112.5, 115.0];
    learning.update(bands, 102.5, 70_000, shifted_slow_lines, 1);
    let signal = learning.signal;
    assert_eq!(signal.learned_direction, -1);
    assert_eq!(signal.match_scope, TouchMatchScope::Slope);
    assert!(signal.reliable);
}

#[test]
fn price_between_lines_is_neutral_until_an_exact_touch_is_learned() {
    let bands = lines();
    let mut learning = MrcTouchLearningState::default();
    learning.update(bands, 101.0, 1_000, bands, 1);
    assert!(!learning.signal.exact_touch);
    assert_eq!(learning.signal.learned_samples, 0);
    assert!(!learning.signal.reliable);
}

#[test]
fn crossing_event_is_kept_as_a_touch_when_the_sample_price_is_not_exact() {
    let bands = lines();
    let mut learning = MrcTouchLearningState::default();
    learning.update_with_events(bands, 101.0, 1_000, bands, 1, &[4]);
    assert_eq!(learning.signal.current_line, Some(4));
    assert!(learning.signal.exact_touch);
    assert_eq!(learning.signal.learned_samples, 0);

    learning.update_with_events(bands, 103.0, 61_000, bands, 1, &[5]);
    learning.update_with_events(bands, 101.0, 121_000, bands, 1, &[4]);
    learning.update_with_events(bands, 103.0, 181_000, bands, 1, &[5]);
    learning.update_with_events(bands, 101.0, 241_000, bands, 1, &[4]);
    learning.update_with_events(bands, 103.0, 301_000, bands, 1, &[5]);
    learning.update_with_events(bands, 101.0, 361_000, bands, 1, &[4]);

    assert_eq!(learning.signal.learned_direction, 1);
    assert!(learning.signal.reliable);
}
