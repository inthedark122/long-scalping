pub fn fmt_price(value: f64) -> String {
    if value <= 0.0 {
        "na".to_string()
    } else {
        format!("{:.4}", value)
    }
}

pub fn fmt_qty(value: f64) -> String {
    if value <= 0.0 {
        "na".to_string()
    } else {
        format!("{:.8}", value)
    }
}

pub fn fmt_duration_ms(value: u64) -> String {
    if value == 0 {
        "na".to_string()
    } else if value >= 60_000 {
        format!("{:.1}m", value as f64 / 60_000.0)
    } else if value >= 1_000 {
        format!("{:.1}s", value as f64 / 1_000.0)
    } else {
        format!("{}ms", value)
    }
}

pub fn fmt_duration_ms_f64(value: f64) -> String {
    if value <= 0.0 {
        "na".to_string()
    } else if value >= 60_000.0 {
        format!("{:.1}m", value / 60_000.0)
    } else if value >= 1_000.0 {
        format!("{:.1}s", value / 1_000.0)
    } else {
        format!("{:.0}ms", value)
    }
}

pub fn fmt_metric(value: f64, precision: usize) -> String {
    if value == 0.0 {
        "na".to_string()
    } else {
        format!("{value:.precision$}")
    }
}
