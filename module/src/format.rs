use lte_strategy_bridge::abi::ModuleInput;

pub fn tick_size(input: &ModuleInput) -> f64 {
    input.symbol_info.price_tick_size.max(f64::EPSILON)
}

pub fn fmt_price(value: f64) -> String {
    if value <= 0.0 {
        "na".to_string()
    } else {
        format!("{:.4}", value)
    }
}

pub fn fmt_price_tick(value: f64, tick_size: f64) -> String {
    if value <= 0.0 {
        return "na".to_string();
    }
    if tick_size <= 0.0 || !tick_size.is_finite() {
        return fmt_price(value);
    }
    let rounded = (value / tick_size).round() * tick_size;
    trim_price(rounded)
}

pub fn fmt_duration_ms(value: u64) -> String {
    if value == 0 {
        "na".to_string()
    } else if value >= 3_600_000 {
        let total_minutes = value / 60_000;
        format!("{}h {:02}m", total_minutes / 60, total_minutes % 60)
    } else if value >= 60_000 {
        let total_seconds = value / 1_000;
        format!("{}m {:02}s", total_seconds / 60, total_seconds % 60)
    } else if value >= 1_000 {
        format!("{:.1}s", value as f64 / 1_000.0)
    } else {
        format!("{}ms", value)
    }
}

pub fn fmt_duration_f64(value: f64) -> String {
    if !value.is_finite() || value <= 0.0 {
        "na".to_string()
    } else {
        fmt_duration_ms(value.round() as u64)
    }
}

pub fn fmt_metric(value: f64, precision: usize) -> String {
    if value == 0.0 {
        "na".to_string()
    } else {
        format!("{value:.precision$}")
    }
}

fn trim_price(value: f64) -> String {
    let mut text = format!("{:.12}", value);
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text.is_empty() {
        "0".to_string()
    } else {
        text
    }
}
