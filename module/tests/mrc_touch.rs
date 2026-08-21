use long_scalping::detection::mrc_touch::{MrcTouchRegime, MrcTouchState};

fn lines(low: f64, high: f64) -> [f64; 9] {
    let step = (high - low) / 8.0;
    std::array::from_fn(|index| low + step * index as f64)
}

#[test]
fn repeated_near_line_reversals_raise_debounce_and_sideway_score() {
    let mut state = MrcTouchState::default();
    let bands = lines(90.0, 110.0);
    state.update(bands, 100.0, 1_000);
    for index in 1..80u64 {
        let price = if index % 2 == 0 { 99.90 } else { 100.10 };
        state.update(bands, price, 1_000 + index * 1_000);
    }

    let signal = state.signal;
    assert!(signal.line(4).expect("mean line state").debounce_ema > 0.20);
    assert!(signal.sideways_score > 0.30);
    assert!(matches!(signal.regime, MrcTouchRegime::Sideways));
}

#[test]
fn directional_price_updates_raise_speed_angle_and_interest() {
    let mut state = MrcTouchState::default();
    let bands = lines(90.0, 110.0);
    state.update(bands, 100.0, 1_000);
    state.update(bands, 100.25, 61_000);
    state.update(bands, 100.75, 121_000);

    let signal = state.signal;
    assert!(signal.price_speed_pct_per_min > 0.20);
    assert!(signal.price_angle_deg > 0.0);
    assert!(signal.angle_slope_deg_per_min > 0.0);
    assert!(signal.direction_score > 0.0);
    assert!(signal.interest > 0.0);
}

#[test]
fn contracting_big_band_adds_upward_stairs_pressure() {
    let mut state = MrcTouchState::default();
    state.update(lines(90.0, 110.0), 100.0, 1_000);
    state.update(lines(91.0, 109.0), 100.20, 61_000);
    state.update(lines(92.0, 108.0), 100.60, 121_000);

    let signal = state.signal;
    assert!(signal.band_width_slope_pct_per_min < 0.0);
    assert!(signal.stairs_pressure > 0.0);
    assert!(signal.stairs_score(1) > signal.stairs_score(-1));
}

#[test]
fn contracting_big_band_supports_downward_stairs_too() {
    let mut state = MrcTouchState::default();
    state.update(lines(90.0, 110.0), 100.0, 1_000);
    state.update(lines(91.0, 109.0), 99.80, 61_000);
    state.update(lines(92.0, 108.0), 99.40, 121_000);

    let signal = state.signal;
    assert!(signal.band_width_slope_pct_per_min < 0.0);
    assert!(signal.stairs_pressure < 0.0);
    assert!(signal.stairs_score(-1) > signal.stairs_score(1));
}
