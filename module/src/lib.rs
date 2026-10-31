mod debug;
pub mod detection;
pub mod engine;
mod entry;
mod entry_policy;
mod exit;
mod format;
pub mod market_data;
pub mod model;
pub mod observer;
mod order_manager;
mod orders;
pub mod pattern_matching;
mod risk;
mod risk_control;
pub mod state;

pub use engine::run;
pub use format::{fmt_duration_f64, fmt_duration_ms};
