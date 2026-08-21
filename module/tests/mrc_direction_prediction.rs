use long_scalping::detection::mrc_direction_prediction::{
    predict, MrcDirectionContext, MrcPredictionState,
};
use long_scalping::detection::mrc_touch::MrcTouchState;
use long_scalping::detection::mrc_touch_learning::MrcTouchLearningState;

fn lines(low: f64, high: f64) -> [f64; 9] {
    let step = (high - low) / 8.0;
    std::array::from_fn(|index| low + step * index as f64)
}

fn learned_up_touch(bands: [f64; 9]) -> MrcTouchLearningState {
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
    learning
}

#[test]
fn exact_touch_history_predicts_the_next_line_without_a_weighted_formula() {
    let mut touch = MrcTouchState::default();
    let bands = lines(90.0, 110.0);
    touch.update(bands, 100.0, 1_000);
    touch.update(bands, 100.1, 61_000);
    let learning = learned_up_touch(bands);

    let prediction = predict(
        touch.signal,
        MrcDirectionContext {
            structural_direction: -1,
            candidate_direction: 1,
            touch_learning: learning.signal,
            ..Default::default()
        },
    );

    assert_eq!(prediction.learned_direction, 1);
    assert!(prediction.candidate_allowed);
    assert_eq!(prediction.up_probability, 1.0);
    assert!(matches!(
        prediction.state,
        MrcPredictionState::ContinuationUp
    ));
}

#[test]
fn opposite_candidate_has_lower_expected_value_in_recovery() {
    let mut touch = MrcTouchState::default();
    let bands = lines(90.0, 110.0);
    touch.update(bands, 100.0, 1_000);
    let learning = learned_up_touch(bands);

    let long = predict(
        touch.signal,
        MrcDirectionContext {
            structural_direction: -1,
            candidate_direction: 1,
            touch_learning: learning.signal,
            ..Default::default()
        },
    );
    let short = predict(
        touch.signal,
        MrcDirectionContext {
            structural_direction: -1,
            candidate_direction: -1,
            touch_learning: learning.signal,
            ..Default::default()
        },
    );

    assert!(long.candidate_allowed);
    assert!(!short.candidate_allowed);
    assert_eq!(short.direction_score, 1.0);
}

#[test]
fn repeated_mean_bounces_raise_sideways_probability() {
    let mut touch = MrcTouchState::default();
    let bands = lines(90.0, 110.0);
    touch.update(bands, 100.0, 1_000);
    for index in 1..80u64 {
        let price = if index % 2 == 0 { 99.90 } else { 100.10 };
        touch.update(bands, price, 1_000 + index * 1_000);
    }

    let prediction = predict(
        touch.signal,
        MrcDirectionContext {
            mean_distance_pct: touch.signal.line(4).unwrap().distance_ema_pct,
            ..Default::default()
        },
    );

    assert_eq!(prediction.state, MrcPredictionState::Unknown);
    assert_eq!(prediction.learned_samples, 0);
}
