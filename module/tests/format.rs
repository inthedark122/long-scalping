use long_scalping::{fmt_duration_f64, fmt_duration_ms};

#[test]
fn durations_are_human_readable() {
    assert_eq!(fmt_duration_ms(1_013_135), "16m 53s");
    assert_eq!(fmt_duration_f64(1_013_135.4), "16m 53s");
    assert_eq!(fmt_duration_ms(0), "na");
}
