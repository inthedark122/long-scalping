use long_scalping::detection::cascade::{
    movement_history, recent_average_size, recent_average_size_percent, recent_movement_distance,
    recent_movement_distance_percent, recent_signed_total_size, recent_signed_total_size_percent,
    recent_total_size, recent_total_size_percent, update_from_candle, update_from_candle_at,
    CascadeMovement, CascadeState, MovementPhase,
};
use long_scalping::model::Candle;

fn candle(open: f64, high: f64, low: f64, close: f64) -> Candle {
    Candle {
        open,
        high,
        low,
        close,
    }
}

#[test]
fn movement_starts_then_builds() {
    let mut state = CascadeState::default();
    let first = update_from_candle(&mut state, &candle(100.0, 101.0, 99.5, 100.8));
    assert_eq!(first.phase, MovementPhase::Starting);

    let second = update_from_candle(&mut state, &candle(100.8, 103.0, 100.4, 102.5));
    assert!(matches!(
        second.phase,
        MovementPhase::Building | MovementPhase::Mature
    ));
    assert!(second.dir > 0);
}

#[test]
fn timestamped_cascade_reports_movement_speed() {
    let mut state = CascadeState::default();
    let _ = update_from_candle_at(&mut state, &candle(100.0, 101.0, 99.5, 100.8), 1_000);
    let signal = update_from_candle_at(&mut state, &candle(100.8, 103.0, 100.4, 102.5), 61_000);

    assert_eq!(signal.elapsed_ms, 60_000);
    assert_eq!(signal.speed_pct_per_min, Some(3.5 / 102.5 * 100.0));
}

#[test]
fn counter_move_and_reversal_are_flagged() {
    let mut state = CascadeState::default();
    let _ = update_from_candle(&mut state, &candle(100.0, 101.0, 99.5, 100.8));
    let _ = update_from_candle(&mut state, &candle(100.8, 103.0, 100.4, 102.5));
    let reversal = update_from_candle(&mut state, &candle(102.5, 102.8, 99.2, 99.8));
    assert_eq!(reversal.phase, MovementPhase::Reversal);
    assert!(reversal.completed_size > 0.0);

    let history = movement_history(&state);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].high, 103.0);
    assert_eq!(history[0].low, 99.5);
}

#[test]
fn new_block_starts_from_current_candle_after_reversal() {
    let mut state = CascadeState::default();
    let _ = update_from_candle(&mut state, &candle(100.0, 101.0, 99.0, 100.5));
    let _ = update_from_candle(&mut state, &candle(100.5, 106.0, 100.0, 105.5));
    let _ = update_from_candle(&mut state, &candle(105.5, 105.8, 104.8, 104.9));

    assert!(state.cas_high <= 105.8 + f64::EPSILON);
    assert!(state.cas_low >= 104.8 - f64::EPSILON);
    assert!((state.cas_high - state.cas_low).abs() <= 1.1);
}

#[test]
fn movement_position_uses_realtime_close_inside_active_range() {
    let mut state = CascadeState::default();
    let signal = update_from_candle(&mut state, &candle(100.0, 104.0, 100.0, 100.8));

    assert!((signal.completed_size - 4.0).abs() < 1e-9);
    assert!((signal.movement_position - 0.20).abs() < 1e-9);
}

#[test]
fn recent_movement_metrics_use_the_latest_history() {
    let mut state = CascadeState::default();
    state.movement_history[0] = CascadeMovement {
        high: 110.0,
        low: 100.0,
        dir: 1,
    };
    state.movement_history[1] = CascadeMovement {
        high: 108.0,
        low: 103.0,
        dir: -1,
    };
    state.movement_history[2] = CascadeMovement {
        high: 112.0,
        low: 105.0,
        dir: 1,
    };
    state.movement_history_len = 3;

    assert_eq!(recent_movement_distance(&state, 2), Some(9.0));
    assert_eq!(
        recent_movement_distance_percent(&state, 2, 100.0),
        Some(9.0)
    );
    assert_eq!(recent_average_size(&state, 2), Some(6.0));
    assert_eq!(recent_average_size_percent(&state, 2, 100.0), Some(6.0));
    assert_eq!(recent_total_size(&state, 2), Some(12.0));
    assert_eq!(recent_total_size_percent(&state, 2, 100.0), Some(12.0));
    assert_eq!(recent_signed_total_size(&state, 2), Some(2.0));
    assert_eq!(
        recent_signed_total_size_percent(&state, 2, 100.0),
        Some(2.0)
    );
}

#[test]
fn invalid_candles_do_not_change_state() {
    let mut state = CascadeState::default();
    let before = state;
    let signal = update_from_candle(&mut state, &candle(0.0, 0.0, 0.0, 0.0));

    assert_eq!(signal, Default::default());
    assert_eq!(state, before);
}
