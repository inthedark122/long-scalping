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
    } else if value >= 60_000 {
        format!("{:.1}m", value as f64 / 60_000.0)
    } else if value >= 1_000 {
        format!("{:.1}s", value as f64 / 1_000.0)
    } else {
        format!("{}ms", value)
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
