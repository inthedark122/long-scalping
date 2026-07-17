#![allow(dead_code)]

use std::error::Error;
use std::process::Command;

use serde_json::Value;

#[path = "../builder/mod.rs"]
mod builder;
#[path = "../detection/cascade.rs"]
mod cascade_impl;
#[path = "../exit.rs"]
mod exit_impl;
#[path = "../detection/levels.rs"]
mod levels_impl;
#[path = "../martingale.rs"]
mod martingale;
#[path = "../model.rs"]
mod model;
#[path = "../detection/mrc.rs"]
mod mrc_impl;
#[path = "../orders.rs"]
mod orders;
#[path = "../prediction.rs"]
mod prediction_impl;
#[path = "../detection/regime.rs"]
mod regime_impl;
#[path = "../detection/sideways.rs"]
mod sideways_impl;
#[path = "../state.rs"]
mod state;
#[path = "../stop_loss.rs"]
mod stop_loss_impl;

mod detection {
    pub mod cascade {
        pub use crate::cascade_impl::*;
    }
    pub mod levels {
        pub use crate::levels_impl::*;
    }
    pub mod mrc {
        pub use crate::mrc_impl::*;
    }
    pub mod regime {
        pub use crate::regime_impl::*;
    }
    pub mod sideways {
        pub use crate::sideways_impl::*;
    }
}

mod exit {
    pub use crate::exit_impl::*;
}

mod prediction {
    pub use crate::prediction_impl::*;
}

mod stop_loss {
    pub use crate::stop_loss_impl::*;
}

use detection::cascade::{self, CascadeSignal};
use detection::{mrc, regime, sideways};
use model::{
    Candle, EntryGate, IndicatorSnapshot, MarketState, PositionState, SugInfoSnapshot,
    TimeframeSnapshot,
};
use state::{CascadeState, EngineState};

const BASE_URL: &str = "https://okx.tg.trading.sinvoid.me/api/indicators";
const BASE_HOST: &str = "okx.tg.trading.sinvoid.me";
const BASE_FALLBACK_IP: &str = "65.109.35.177";
const DEFAULT_SYMBOLS: &[&str] = &["ALLO", "CAP", "BILL"];

#[derive(Debug, Clone)]
struct LiveFrame {
    time_ms: i64,
    snapshot: TimeframeSnapshot,
}

fn encode_symbol_key(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            _ => out.push_str(&format!("%{:02X}", ch as u32)),
        }
    }
    out
}

fn normalize_symbol_key(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.contains('#') {
        return trimmed.to_string();
    }
    let core = trimmed
        .trim_end_matches("-USDT-SWAP")
        .trim_end_matches("-SWAP");
    format!("OKX#{}-USDT-SWAP#SWAP", core)
}

fn resolve_host_ip() -> String {
    let output = Command::new("getent").args(["hosts", BASE_HOST]).output();
    output
        .ok()
        .and_then(|out| {
            if !out.status.success() {
                return None;
            }
            String::from_utf8(out.stdout).ok()
        })
        .and_then(|text| text.split_whitespace().next().map(|ip| ip.to_string()))
        .unwrap_or_else(|| BASE_FALLBACK_IP.to_string())
}

fn f64_or(obj: &Value, primary: &str, fallback: &str) -> f64 {
    let parse = |v: &Value| {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
    };
    obj.get(primary)
        .and_then(parse)
        .or_else(|| obj.get(fallback).and_then(parse))
        .unwrap_or(0.0)
}

fn get_i64(map: &Value, key: &str) -> i64 {
    map.get(key)
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .unwrap_or(0)
}

fn dir_to_int(value: Option<&Value>) -> i8 {
    match value
        .and_then(Value::as_str)
        .map(|s| s.to_ascii_uppercase())
    {
        Some(s) if s == "LONG" => 1,
        Some(s) if s == "SHORT" => -1,
        _ => 0,
    }
}

fn fetch_rows(
    symbol_key: &str,
    timeframe: &str,
    limit: usize,
) -> Result<Vec<Value>, Box<dyn Error>> {
    let url = format!(
        "{}?symbol_key={}&timeframe={}&limit={}",
        BASE_URL,
        encode_symbol_key(symbol_key),
        timeframe,
        limit
    );
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL"]);
    cmd.args([
        "--resolve",
        &format!("{}:443:{}", BASE_HOST, resolve_host_ip()),
    ]);
    let output = cmd.arg(&url).output()?;
    if !output.status.success() {
        return Err(format!("curl fetch failed for {symbol_key} timeframe={timeframe}").into());
    }
    let payload: Value = serde_json::from_slice(&output.stdout)?;
    let mut rows = Vec::new();
    if let Some(data) = payload.get("data").and_then(Value::as_array) {
        rows.extend(data.iter().cloned());
    }
    rows.sort_by_key(|row| get_i64(row, "time"));
    Ok(rows)
}

fn parse_timeframe(rows: &[Value], live_price: Option<f64>) -> Option<LiveFrame> {
    let current = rows.last()?;
    let prev = rows.get(rows.len().saturating_sub(2));
    let ind = current.get("indicators").unwrap_or(&Value::Null);
    let candle = ind.get("candle").unwrap_or(&Value::Null);
    let mrc = ind.get("mrc_200").unwrap_or(&Value::Null);
    let dp = ind.get("dpsignal").unwrap_or(&Value::Null);
    let zz = ind.get("zigzagtrend").unwrap_or(&Value::Null);
    let _signal = ind.get("signal").unwrap_or(&Value::Null);
    let volume_50 = ind.get("volume_50").unwrap_or(&Value::Null);

    let mean_line = f64_or(mrc, "MeanLine", "mean_line");
    let prev_mean_line = prev
        .and_then(|row| row.get("indicators"))
        .and_then(|i| i.get("mrc_200"))
        .map(|prev_mrc| f64_or(prev_mrc, "MeanLine", "mean_line"))
        .unwrap_or(mean_line);

    let close = live_price.unwrap_or_else(|| f64_or(candle, "Close", "close"));
    let snapshot = TimeframeSnapshot {
        candle: Candle {
            open: f64_or(candle, "Open", "open"),
            high: f64_or(candle, "High", "high"),
            low: f64_or(candle, "Low", "low"),
            close,
        },
        mrc: model::MrcBands {
            down_big: f64_or(mrc, "DownBig", "down_big"),
            down_outer: f64_or(mrc, "DownOuter", "down_outer"),
            down_small: f64_or(mrc, "DownSmall", "down_small"),
            down_inner: f64_or(mrc, "DownInner", "down_inner"),
            mean_line,
            up_inner: f64_or(mrc, "UpInner", "up_inner"),
            up_small: f64_or(mrc, "UpSmall", "up_small"),
            up_outer: f64_or(mrc, "UpOuter", "up_outer"),
            up_big: f64_or(mrc, "UpBig", "up_big"),
            current_cross: f64_or(mrc, "CurrentCross", "current_cross"),
            mean_line_slope: mean_line - prev_mean_line,
        },
        mrc_signal: mrc::analyze(
            &model::MrcBands {
                down_big: f64_or(mrc, "DownBig", "down_big"),
                down_outer: f64_or(mrc, "DownOuter", "down_outer"),
                down_small: f64_or(mrc, "DownSmall", "down_small"),
                down_inner: f64_or(mrc, "DownInner", "down_inner"),
                mean_line,
                up_inner: f64_or(mrc, "UpInner", "up_inner"),
                up_small: f64_or(mrc, "UpSmall", "up_small"),
                up_outer: f64_or(mrc, "UpOuter", "up_outer"),
                up_big: f64_or(mrc, "UpBig", "up_big"),
                current_cross: f64_or(mrc, "CurrentCross", "current_cross"),
                mean_line_slope: mean_line - prev_mean_line,
            },
            close,
        ),
        dp_dir: dir_to_int(dp.get("Direction")),
        zz_dir: dir_to_int(zz.get("Direction")),
        dp_zee: f64_or(dp, "Zee", "zee"),
        dp_zee1: f64_or(dp, "Zee1", "zee1"),
        zz_buy_price: f64_or(zz, "BuyPrice", "buy_price"),
        zz_sell_price: f64_or(zz, "SellPrice", "sell_price"),
        zz_break_price: f64_or(zz, "BreakPrice", "break_price"),
        volume_gain: f64_or(volume_50, "Gain", "gain"),
        volume_gain_min: f64_or(volume_50, "Min", "min"),
        volume_gain_max: f64_or(volume_50, "Max", "max"),
    };

    Some(LiveFrame {
        time_ms: get_i64(current, "time"),
        snapshot,
    })
}

fn fetch_timeframe(
    symbol_key: &str,
    timeframe: &str,
    limit: usize,
) -> Result<LiveFrame, Box<dyn Error>> {
    let rows = fetch_rows(symbol_key, timeframe, limit)?;
    parse_timeframe(&rows, None)
        .ok_or_else(|| format!("no timeframe data for {symbol_key} tf={timeframe}").into())
}

fn fetch_1m_history(symbol_key: &str, limit: usize) -> Result<Vec<LiveFrame>, Box<dyn Error>> {
    let rows = fetch_rows(symbol_key, "60", limit)?;
    let mut out = Vec::new();
    for idx in 0..rows.len() {
        let slice = &rows[..=idx];
        if let Some(frame) = parse_timeframe(slice, None) {
            out.push(frame);
        }
    }
    Ok(out)
}

fn build_indicator_snapshot(
    tf1m: TimeframeSnapshot,
    tf5m: TimeframeSnapshot,
    tf15m: TimeframeSnapshot,
    tf1h: TimeframeSnapshot,
) -> IndicatorSnapshot {
    IndicatorSnapshot {
        tf1m,
        tf5m,
        tf15m,
        tf1h,
        ..Default::default()
    }
}

fn cascade_signal_from_history(history: &[LiveFrame]) -> (CascadeState, CascadeSignal) {
    let mut cascade_state = CascadeState::default();
    let mut last_signal = CascadeSignal::default();
    for frame in history {
        last_signal = cascade::update_from_candle(&mut cascade_state, &frame.snapshot.candle);
    }
    (cascade_state, last_signal)
}

fn gate_summary(gate: &EntryGate) -> String {
    match gate {
        EntryGate::Open { notes } => {
            if notes.is_empty() {
                "open".to_string()
            } else {
                format!("open:{}", notes.join("|"))
            }
        }
        EntryGate::Wait(wait) => {
            let mut text = format!("wait:{}", wait.reason);
            if !wait.solutions.is_empty() {
                text.push_str(&format!(" sol={}", wait.solutions.len()));
            }
            if !wait.notes.is_empty() {
                text.push_str(&format!(" notes={}", wait.notes.join("|")));
            }
            text
        }
    }
}

fn predict_side(
    snapshot: &IndicatorSnapshot,
    cascade_signal: &CascadeSignal,
    side: PositionState,
) -> String {
    let mut state = EngineState::default();
    let gate = prediction::entry_gate(
        snapshot,
        &SugInfoSnapshot::default(),
        cascade_signal,
        &mut state,
        side,
    );
    gate_summary(&gate)
}

fn side_gate(
    snapshot: &IndicatorSnapshot,
    cascade_signal: &CascadeSignal,
    side: PositionState,
) -> EntryGate {
    let mut state = EngineState::default();
    prediction::entry_gate(
        snapshot,
        &SugInfoSnapshot::default(),
        cascade_signal,
        &mut state,
        side,
    )
}

#[derive(Debug, Clone)]
struct TradeRecommendation {
    side: PositionState,
    enter_now: bool,
    reason: String,
    gate: EntryGate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeMode {
    Recommendation,
    Full,
}

#[derive(Debug, Clone)]
struct ProbeOptions {
    mode: ProbeMode,
    show_history: bool,
    history_windows: Vec<usize>,
}

fn confirmation_label(value: regime::OneMConfirmation) -> &'static str {
    match value {
        regime::OneMConfirmation::Aligned => "aligned",
        regime::OneMConfirmation::Neutral => "neutral",
        regime::OneMConfirmation::Tracked => "tracked",
        regime::OneMConfirmation::Countertrend => "countertrend",
    }
}

fn trade_recommendation(
    regime: &model::RegimeSnapshot,
    snapshot: &IndicatorSnapshot,
    cascade_signal: &CascadeSignal,
) -> TradeRecommendation {
    let long_gate = side_gate(snapshot, cascade_signal, PositionState::Long);
    let short_gate = side_gate(snapshot, cascade_signal, PositionState::Short);
    let long_open = matches!(long_gate, EntryGate::Open { .. });
    let short_open = matches!(short_gate, EntryGate::Open { .. });
    let sideway_signal = sideways::analyze(&EngineState::default(), &snapshot.tf1m);

    let (side, reason) = match (long_open, short_open) {
        (true, false) => (PositionState::Long, "long-open".to_string()),
        (false, true) => (PositionState::Short, "short-open".to_string()),
        (true, true) => match regime.confirmed {
            MarketState::Long => (PositionState::Long, "regime-confirmed-long".to_string()),
            MarketState::Short => (PositionState::Short, "regime-confirmed-short".to_string()),
            MarketState::Sideways => {
                if sideway_signal.active {
                    if sideway_signal.side == PositionState::Long {
                        (PositionState::Long, "sideway-long".to_string())
                    } else if sideway_signal.side == PositionState::Short {
                        (PositionState::Short, "sideway-short".to_string())
                    } else {
                        (PositionState::Flat, "sideway-no-side".to_string())
                    }
                } else if cascade_signal.dir < 0 {
                    (PositionState::Short, "cascade-down".to_string())
                } else if cascade_signal.dir > 0 {
                    (PositionState::Long, "cascade-up".to_string())
                } else {
                    (PositionState::Flat, "both-open-undecided".to_string())
                }
            }
        },
        (false, false) => match regime.confirmed {
            MarketState::Long => (
                PositionState::Long,
                "regime-long-but-no-long-gate".to_string(),
            ),
            MarketState::Short => (
                PositionState::Short,
                "regime-short-but-no-short-gate".to_string(),
            ),
            MarketState::Sideways => {
                if sideway_signal.active {
                    (
                        sideway_signal.side,
                        format!("sideway-{}-wait", sideway_signal.reason),
                    )
                } else if cascade_signal.dir < 0 {
                    (PositionState::Short, "cascade-down-wait".to_string())
                } else if cascade_signal.dir > 0 {
                    (PositionState::Long, "cascade-up-wait".to_string())
                } else {
                    (PositionState::Flat, "no-side-confirmation".to_string())
                }
            }
        },
    };

    let gate = match side {
        PositionState::Long => long_gate.clone(),
        PositionState::Short => short_gate.clone(),
        PositionState::Flat => EntryGate::Wait(model::EntryWait {
            reason: "no-side".to_string(),
            solutions: vec![],
            notes: vec!["no directional recommendation".to_string()],
        }),
    };

    let enter_now = matches!(gate, EntryGate::Open { .. });
    TradeRecommendation {
        side,
        enter_now,
        reason: format!(
            "{} regime={:?} sideway={}",
            reason, regime.confirmed, sideway_signal.reason
        ),
        gate,
    }
}

fn tf_line(name: &str, tf: &TimeframeSnapshot) -> String {
    format!(
        "{} close={:.6} dp={} zz={} zee1={:.6} above_zee1={} mrc_zone={} slope={} bull={} bear={} pzone={:.2} mean={:.6} down_outer={:.6} up_inner={:.6} up_outer={:.6}",
        name,
        tf.candle.close,
        tf.dp_dir,
        tf.zz_dir,
        tf.dp_zee1,
        tf.dp_zee1 > 0.0 && tf.candle.close >= tf.dp_zee1,
        tf.mrc_signal.zone,
        tf.mrc_signal.slope_sign,
        tf.mrc_signal.bullish_structure,
        tf.mrc_signal.bearish_structure,
        tf.mrc_signal.pzone,
        tf.mrc.mean_line,
        tf.mrc.down_outer,
        tf.mrc.up_inner,
        tf.mrc.up_outer,
    )
}

fn direction_summary(regime: &model::RegimeSnapshot, tf5: &TimeframeSnapshot) -> String {
    if regime.confirmed == MarketState::Long {
        return "long-confirmed".to_string();
    }
    if regime.primary == MarketState::Long && regime.blocked_by_context {
        return "5m-bullish-but-1h-blocked".to_string();
    }
    if tf5.mrc_signal.bullish_structure {
        return "5m-structure-bullish-but-regime-not-confirmed".to_string();
    }
    if tf5.candle.close >= tf5.mrc.down_outer && tf5.mrc.mean_line_slope > 0.0 {
        return "price-above-down-outer-but-structure-not-strong-enough".to_string();
    }
    "no-long-confirmation".to_string()
}

fn print_symbol(symbol_key: &str, opts: &ProbeOptions) -> Result<(), Box<dyn Error>> {
    let symbol_key = normalize_symbol_key(symbol_key);
    let tf1m = fetch_timeframe(&symbol_key, "60", 3)?.snapshot;
    let tf5m = fetch_timeframe(&symbol_key, "300", 3)?.snapshot;
    let tf15m = fetch_timeframe(&symbol_key, "900", 3)?.snapshot;
    let tf1h = fetch_timeframe(&symbol_key, "3600", 3)?.snapshot;
    let snapshot =
        build_indicator_snapshot(tf1m.clone(), tf5m.clone(), tf15m.clone(), tf1h.clone());
    let regime = regime::classify_regime(&snapshot.tf5m, &snapshot.tf15m, &snapshot.tf1h);
    let confirmation = regime::classify_confirmation(regime.confirmed, &snapshot.tf1m);

    let history_limit = opts.history_windows.iter().copied().max().unwrap_or(200);
    let history = fetch_1m_history(&symbol_key, history_limit)?;
    let (cascade_state, cascade_signal) = cascade_signal_from_history(&history);
    let recommendation = trade_recommendation(&regime, &snapshot, &cascade_signal);
    let recommendation_side = recommendation.side;
    let entry_price = snapshot.tf1m.candle.close;
    let exit_plan = exit::plan(
        &snapshot,
        &SugInfoSnapshot::default(),
        &cascade_state,
        entry_price,
        recommendation_side,
        None,
    );

    println!("=== {} ===", symbol_key);
    println!(
        "recommendation side={:?} enter_now={} reason={} 1m_confirmation={}",
        recommendation.side,
        recommendation.enter_now,
        recommendation.reason,
        confirmation_label(confirmation)
    );

    if opts.mode == ProbeMode::Full {
        println!("{}", tf_line("1m", &snapshot.tf1m));
        println!("{}", tf_line("5m", &snapshot.tf5m));
        println!("{}", tf_line("15m", &snapshot.tf15m));
        println!("{}", tf_line("1h", &snapshot.tf1h));
        println!(
            "regime primary={:?} fallback={:?} context_1h={:?} confirmed={:?} blocked_by_context={}",
            regime.primary,
            regime.fallback,
            regime.context_1h,
            regime.confirmed,
            regime.blocked_by_context
        );
        println!(
            "1m_confirmation state={} regime={:?}",
            confirmation_label(confirmation),
            regime.confirmed
        );
        println!("direction={}", direction_summary(&regime, &snapshot.tf5m));
        println!(
            "cascade dir={} prev_dir={} strength={:.2} samples={} avg_size={:.6} recommendation_side={:?} enter_now={} entry_gate={}",
            cascade_signal.dir,
            cascade_signal.prev_dir,
            cascade_signal.strength,
            cascade_signal.samples,
            cascade_signal.average_size,
            recommendation_side,
            recommendation.enter_now,
            gate_summary(&recommendation.gate),
        );
        match exit_plan {
            Some(plan) => {
                println!(
                    "expected_exit source={:?} tp={:.6} sl={:.6} structural_tp={:.6} structural_sl={:.6} reliable={}",
                    plan.source,
                    plan.take_profit,
                    plan.stop_loss,
                    plan.structural_take_profit,
                    plan.structural_stop_loss,
                    plan.tp_reliable
                );
            }
            None => {
                println!("expected_exit none");
            }
        }
    }

    if opts.show_history {
        for window in &opts.history_windows {
            let actual_window = (*window).min(history.len()).max(1);
            let start = history.len().saturating_sub(actual_window);
            let window_history = &history[start..];
            let (window_cascade_state, window_cascade_signal) =
                cascade_signal_from_history(window_history);
            let long_pred = predict_side(&snapshot, &window_cascade_signal, PositionState::Long);
            let short_pred = predict_side(&snapshot, &window_cascade_signal, PositionState::Short);
            let recommendation = trade_recommendation(&regime, &snapshot, &window_cascade_signal);
            let confirmation = regime::classify_confirmation(regime.confirmed, &snapshot.tf1m);
            println!(
                "history[{}] recommendation={:?} enter_now={} reason={} 1m_confirmation={} long={} short={} cascade dir={} samples={} avg={:.6}",
                window,
                recommendation.side,
                recommendation.enter_now,
                recommendation.reason,
                confirmation_label(confirmation),
                long_pred,
                short_pred,
                window_cascade_signal.dir,
                window_cascade_state.samples,
                window_cascade_state.avg_size,
            );
        }
    }
    Ok(())
}

fn parse_history_windows(value: &str) -> Vec<usize> {
    value
        .split(',')
        .filter_map(|part| part.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .collect()
}

fn parse_args() -> (Vec<String>, ProbeOptions) {
    let mut symbols = Vec::new();
    let mut history_windows = Vec::new();
    let mut mode = ProbeMode::Recommendation;
    let mut show_history = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--symbol-key" => {
                if let Some(value) = args.next() {
                    symbols.push(value);
                }
            }
            "--symbols" => {
                if let Some(value) = args.next() {
                    symbols.extend(
                        value
                            .split(',')
                            .filter(|s| !s.trim().is_empty())
                            .map(|s| s.trim().to_string()),
                    );
                }
            }
            "--history" => {
                if let Some(value) = args.next() {
                    history_windows.extend(parse_history_windows(&value));
                }
            }
            "--show-history" => {
                show_history = true;
            }
            "--hide-history" => {
                show_history = false;
            }
            "--mode" => {
                if let Some(value) = args.next() {
                    mode = match value.as_str() {
                        "full" => ProbeMode::Full,
                        _ => ProbeMode::Recommendation,
                    };
                }
            }
            _ => {
                if !arg.starts_with('-') {
                    symbols.push(arg);
                }
            }
        }
    }

    if symbols.is_empty() {
        symbols = DEFAULT_SYMBOLS.iter().map(|s| s.to_string()).collect();
    }
    if history_windows.is_empty() {
        history_windows.push(200);
    }
    history_windows.sort_unstable();
    history_windows.dedup();
    (
        symbols,
        ProbeOptions {
            mode,
            show_history,
            history_windows,
        },
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    let (symbols, opts) = parse_args();
    for symbol in symbols {
        print_symbol(&symbol, &opts)?;
    }
    Ok(())
}
