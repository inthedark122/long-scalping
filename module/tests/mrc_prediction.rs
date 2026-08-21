use long_scalping::detection::mrc_prediction::{self, MrcLineRole, MrcPredictionContext};
use long_scalping::state::MrcForecastStats;

fn lines() -> [f64; 9] {
    [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0]
}

#[test]
fn line_plan_reports_movement_stop_time_and_route_role() {
    let mut stats = [MrcForecastStats::default(); 9];
    stats[6] = MrcForecastStats {
        count: 2,
        duration_sum_ms: 10_000,
        ..Default::default()
    };
    let plans = mrc_prediction::build_line_plans(
        lines(),
        5,
        104.0,
        1,
        Some(103.0),
        Some(6),
        stats,
        MrcPredictionContext {
            cascade_enabled: true,
            ntps: 42,
            ntps_fast_time_ms: Some(12_000),
            cascade_direction: 1,
            cascade_speed_pct_per_min: Some(1.0),
            cascade_strength: 1.2,
            cascade_stretched: false,
            cascade_phase: long_scalping::detection::cascade::MovementPhase::Building,
            cascade_potential_move_pct: Some(3.0),
            cascade_adverse_move_pct: Some(0.25),
            ..Default::default()
        },
    );

    assert_eq!(plans[6].move_pct, 100.0 / 104.0 * 2.0);
    assert_eq!(plans[6].distance_pct, 100.0 / 104.0 * 2.0);
    assert_eq!(plans[6].stop_loss_pct, 100.0 / 104.0);
    assert!(plans[6].stop_loss_set);
    assert_eq!(plans[6].historical_time_ms, 5_000.0);
    assert!(plans[6].cascade_time_ms > 0.0);
    assert_eq!(plans[6].estimated_time_ms, plans[6].cascade_time_ms);
    assert_eq!(plans[6].route_role, MrcLineRole::PlannedTarget);
    assert_eq!(plans[6].ntps, 42);
    assert_eq!(plans[5].route_role, MrcLineRole::Current);
    assert!(matches!(plans[7].route_role, MrcLineRole::BeyondTarget));
}

#[test]
fn every_line_explains_direction_and_coverage() {
    let plans = mrc_prediction::build_line_plans(
        lines(),
        3,
        96.0,
        -1,
        Some(99.0),
        Some(1),
        [MrcForecastStats::default(); 9],
        MrcPredictionContext::default(),
    );

    assert!(plans[2].move_pct > 0.0);
    assert_eq!(plans[1].route_role, MrcLineRole::PlannedTarget);
    assert!(matches!(
        plans[2].route_role,
        MrcLineRole::CoveredByTarget { target_line: 1 }
    ));
    assert!(plans[4].move_pct < 0.0);
    assert_eq!(plans[4].route_role, MrcLineRole::OppositeDirection);
    assert_eq!(plans[3].route_role, MrcLineRole::Current);
}
