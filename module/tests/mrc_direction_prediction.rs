use long_scalping::detection::mrc_direction_prediction::{
    predict, MrcDirectionContext, MrcPredictionState,
};
use long_scalping::detection::mrc_touch::MrcTouchState;

fn lines(low: f64, high: f64) -> [f64; 9] {
    let step = (high - low) / 8.0;
    std::array::from_fn(|index| low + step * index as f64)
}

#[test]
fn below_mean_recovery_predicts_up_without_forcing_a_trade() {
    let mut touch = MrcTouchState::default();
    let bands = lines(90.0, 110.0);
    touch.update(bands, 99.0, 1_000);
    touch.update(bands, 99.20, 61_000);
    touch.update(bands, 99.60, 121_000);

    let prediction = predict(
        touch.signal,
        MrcDirectionContext {
            structural_direction: -1,
            candidate_direction: 1,
            mean_distance_pct: touch.signal.line(4).unwrap().distance_ema_pct,
            pattern_samples: 3,
            target_samples: 1,
            target_distance_pct: 1.2,
            stop_distance_pct: 1.0,
            ..Default::default()
        },
    );

    assert!(prediction.up_probability > prediction.down_probability);
    assert!(prediction.candidate_edge_pct > 0.0);
    assert!(matches!(
        prediction.state,
        MrcPredictionState::RecoveryUp | MrcPredictionState::ContinuationUp
    ));
}

#[test]
fn opposite_candidate_has_lower_expected_value_in_recovery() {
    let mut touch = MrcTouchState::default();
    let bands = lines(90.0, 110.0);
    touch.update(bands, 99.0, 1_000);
    touch.update(bands, 99.20, 61_000);
    touch.update(bands, 99.60, 121_000);
    let mean_distance_pct = touch.signal.line(4).unwrap().distance_ema_pct;

    let long = predict(
        touch.signal,
        MrcDirectionContext {
            structural_direction: -1,
            candidate_direction: 1,
            mean_distance_pct,
            target_distance_pct: 1.2,
            stop_distance_pct: 1.0,
            ..Default::default()
        },
    );
    let short = predict(
        touch.signal,
        MrcDirectionContext {
            structural_direction: -1,
            candidate_direction: -1,
            mean_distance_pct,
            target_distance_pct: 1.2,
            stop_distance_pct: 1.0,
            ..Default::default()
        },
    );

    assert!(long.candidate_edge_pct > short.candidate_edge_pct);
    assert!(short.direction_score > -1.0);
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

    assert!(prediction.sideways_probability > prediction.up_probability);
    assert!(prediction.sideways_probability > prediction.down_probability);
}
