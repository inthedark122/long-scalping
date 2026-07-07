#![allow(dead_code)]

use std::error::Error;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

#[path = "../model.rs"]
mod model;
#[path = "../state.rs"]
mod state;
#[path = "../detection/mrc.rs"]
mod mrc_impl;
#[path = "../detection/cascade.rs"]
mod cascade_impl;
mod detection {
    pub mod mrc {
        pub use crate::mrc_impl::*;
    }
    pub mod cascade {
        pub use crate::cascade_impl::*;
    }
}

use detection::cascade::{self, CascadeSignal, MovementPhase};
use model::Candle;
use state::CascadeState;

const BASE_URL: &str = "https://okx.tg.trading.sinvoid.me/api/indicators";
const DEFAULT_SYMBOL_KEY: &str = "OKX#ETH-USDT-SWAP#SWAP";
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn handle_sigint(_: i32) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
fn install_sigint_handler() {
    unsafe {
        extern "C" {
            fn signal(signum: i32, handler: extern "C" fn(i32)) -> extern "C" fn(i32);
        }
        const SIGINT: i32 = 2;
        let _ = signal(SIGINT, handle_sigint);
    }
}

#[cfg(not(unix))]
fn install_sigint_handler() {}
#[derive(Debug, Clone)]
struct CandleRow {
    time_ms: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    dp_dir: i8,
    zz_dir: i8,
    gain: f64,
    ntps: i64,
}

#[derive(Debug, Clone)]
struct VirtualTrade {
    side: i8,
    entry_pct: f64,
    tp_pct: f64,
    sl_pct: f64,
    reason: String,
    max_favorable_pct: f64,
    max_adverse_pct: f64,
}

#[derive(Debug, Clone)]
struct TradeResult {
    side: i8,
    pnl_pct: f64,
    mfe_pct: f64,
    mae_pct: f64,
    reason: String,
}

#[derive(Debug, Clone)]
struct TradeEvent {
    kind: &'static str,
    side: &'static str,
    time_ms: i64,
    reason: String,
    pnl_pct: f64,
}

fn pct_range(lo: f64, hi: f64, price: f64) -> f64 {
    if price <= 0.0 {
        0.0
    } else {
        ((hi - lo).abs() / price) * 100.0
    }
}

fn fmt_time(ms: i64) -> String {
    if ms <= 0 {
        return "?".to_string();
    }
    let secs = ms / 1000;
    let days = secs / 86_400;
    let rem = secs % 86_400;
    format!(
        "d{} {:02}:{:02}:{:02}",
        days,
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

fn dir_to_int(value: Option<&Value>) -> i8 {
    match value.and_then(Value::as_str).map(|s| s.to_ascii_uppercase()) {
        Some(s) if s == "LONG" => 1,
        Some(s) if s == "SHORT" => -1,
        _ => 0,
    }
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
        .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok())))
        .unwrap_or(0)
}

fn candle_pct(candle: &CandleRow) -> f64 {
    pct_range(candle.low, candle.high, candle.close)
}

fn block_pct(size: f64, price: f64) -> f64 {
    if price <= 0.0 {
        0.0
    } else {
        (size.abs() / price) * 100.0
    }
}

fn cascade_window(state: &CascadeState, price: f64) -> String {
    let window = cascade::sample_window(state);
    if window.is_empty() {
        return "[]".to_string();
    }

    let values = window
        .into_iter()
        .map(|size| format!("{:.4}%", block_pct(size, price)))
        .collect::<Vec<_>>();
    format!("[{}]", values.join(", "))
}

fn to_candle(row: &CandleRow) -> Candle {
    Candle {
        open: row.open,
        high: row.high,
        low: row.low,
        close: row.close,
    }
}

fn movement_signal(signal: &CascadeSignal) -> &'static str {
    if signal.samples < 3 || signal.average_size <= 0.0 || signal.dir == 0 {
        return if signal.phase == MovementPhase::Starting {
            "start"
        } else {
            "building"
        };
    }
    match signal.phase {
        MovementPhase::Starting => "start",
        MovementPhase::Reversal => "reverse-ready",
        MovementPhase::Mature if signal.strength >= 1.3 => "reverse-ready",
        MovementPhase::CounterMove if signal.strength <= 0.8 => "safe-continuation",
        MovementPhase::CounterMove => "counter-move",
        MovementPhase::Building => "building",
        MovementPhase::Mature => "mature",
        MovementPhase::Unknown => "building",
    }
}

fn best_fill(candle: &CandleRow, side: i8) -> f64 {
    if side > 0 {
        candle.low
    } else if side < 0 {
        candle.high
    } else {
        candle.close
    }
}

fn open_virtual_trade(
    state: &CascadeState,
    candle: &CandleRow,
    side: i8,
    reason: &str,
    tp_mult: f64,
    sl_mult: f64,
) -> Option<VirtualTrade> {
    let tp_base = cascade::cascade_take_profit_distance(state).unwrap_or(state.avg_size);
    if side == 0 || tp_base <= 0.0 {
        return None;
    }
    let entry_pct = best_fill(candle, side);
    let (tp_pct, sl_pct) = if side > 0 {
        (
            entry_pct + tp_base * tp_mult,
            entry_pct - tp_base * sl_mult,
        )
    } else {
        (
            entry_pct - tp_base * tp_mult,
            entry_pct + tp_base * sl_mult,
        )
    };
    Some(VirtualTrade {
        side,
        entry_pct,
        tp_pct,
        sl_pct,
        reason: reason.to_string(),
        max_favorable_pct: 0.0,
        max_adverse_pct: 0.0,
    })
}

fn finalize_trade(trade: &VirtualTrade, exit_pct: f64, reason: &str, _exit_time_ms: i64) -> TradeResult {
    let (pnl_pct, mfe_pct, mae_pct) = if trade.side > 0 {
        (
            ((exit_pct - trade.entry_pct) / trade.entry_pct) * 100.0,
            ((trade.max_favorable_pct) / trade.entry_pct) * 100.0,
            ((trade.max_adverse_pct.abs()) / trade.entry_pct) * 100.0,
        )
    } else {
        (
            ((trade.entry_pct - exit_pct) / trade.entry_pct) * 100.0,
            ((trade.max_favorable_pct) / trade.entry_pct) * 100.0,
            ((trade.max_adverse_pct.abs()) / trade.entry_pct) * 100.0,
        )
    };
    TradeResult {
        side: trade.side,
        pnl_pct,
        mfe_pct,
        mae_pct,
        reason: reason.to_string(),
    }
}

fn summarize_results(results: &[TradeResult]) -> String {
    if results.is_empty() {
        return "no trades".to_string();
    }
    let wins: Vec<&TradeResult> = results.iter().filter(|r| r.pnl_pct > 0.0).collect();
    let losses: Vec<&TradeResult> = results.iter().filter(|r| r.pnl_pct <= 0.0).collect();
    let avg_win = if wins.is_empty() {
        0.0
    } else {
        wins.iter().map(|r| r.pnl_pct).sum::<f64>() / wins.len() as f64
    };
    let avg_loss = if losses.is_empty() {
        0.0
    } else {
        losses.iter().map(|r| r.pnl_pct).sum::<f64>() / losses.len() as f64
    };
    let avg_mfe = results.iter().map(|r| r.mfe_pct).sum::<f64>() / results.len() as f64;
    let avg_mae = results.iter().map(|r| r.mae_pct).sum::<f64>() / results.len() as f64;
    format!(
        "trades={} win_rate={:.1}% avg_win={:.3}% avg_loss={:.3}% avg_mfe={:.3}% avg_mae={:.3}%",
        results.len(),
        wins.len() as f64 / results.len() as f64 * 100.0,
        avg_win,
        avg_loss,
        avg_mfe,
        avg_mae
    )
}

fn parse_candle(item: &Value) -> CandleRow {
    let ind = item.get("indicators").unwrap_or(&Value::Null);
    let candle = ind.get("candle").unwrap_or(&Value::Null);
    let dp = ind.get("dpsignal").unwrap_or(&Value::Null);
    let zz = ind.get("zigzagtrend").unwrap_or(&Value::Null);
    let signal = ind.get("signal").unwrap_or(&Value::Null);
    let volume_50 = ind.get("volume_50").unwrap_or(&Value::Null);

    CandleRow {
        time_ms: get_i64(item, "time"),
        open: f64_or(candle, "Open", "open"),
        high: f64_or(candle, "High", "high"),
        low: f64_or(candle, "Low", "low"),
        close: f64_or(candle, "Close", "close"),
        dp_dir: dir_to_int(dp.get("Direction")),
        zz_dir: dir_to_int(zz.get("Direction")),
        gain: f64_or(volume_50, "Gain", "gain"),
        ntps: get_i64(signal, "ntps"),
    }
}

fn fetch_history(symbol_key: &str, limit: usize) -> Result<Vec<CandleRow>, Box<dyn Error>> {
    let py = r#"
import json, sys, urllib.parse, urllib.request
base = sys.argv[1]
symbol_key = sys.argv[2]
limit = sys.argv[3]
url = f"{base}?symbol_key={urllib.parse.quote(symbol_key, safe='')}&timeframe=60&limit={limit}"
with urllib.request.urlopen(url, timeout=20) as resp:
    print(resp.read().decode())
"#;
    let output = Command::new("python3")
        .args([ "-c", py, BASE_URL, symbol_key, &limit.to_string() ])
        .output()?;
    if !output.status.success() {
        return Err(format!("python fetch failed for {symbol_key}").into());
    }
    let payload: Value = serde_json::from_slice(&output.stdout)?;
    let mut rows = Vec::new();
    if let Some(data) = payload.get("data").and_then(Value::as_array) {
        for item in data {
            rows.push(parse_candle(item));
        }
    }
    rows.sort_by_key(|row| row.time_ms);
    Ok(rows)
}

fn simulate(candles: &[CandleRow], tp_mult: f64, sl_mult: f64) -> (Vec<TradeResult>, Vec<String>, Vec<TradeEvent>) {
    let mut state = CascadeState::default();
    let mut traces = Vec::new();
    let mut trades = Vec::new();
    let mut events = Vec::new();
    let mut active: Option<VirtualTrade> = None;

    if candles.len() < 3 {
        return (trades, vec!["not enough candles".to_string()], events);
    }

    for (idx, cur) in candles.iter().enumerate() {
        let prev_samples = state.samples;
        let prev_window = cascade_window(&state, cur.close);
        let signal = cascade::update_from_candle(&mut state, &to_candle(cur));
        let phase = signal.phase;
        let signal_kind = movement_signal(&signal);
        let size_pct = block_pct(signal.completed_size, cur.close);
        let avg_pct = block_pct(signal.average_size, cur.close);
        let tp_p90_pct = block_pct(
            cascade::cascade_take_profit_distance(&state).unwrap_or(state.avg_size),
            cur.close,
        );
        traces.push(format!(
            "{} candle={:.4}% dir={} phase={:?} signal={} size={:.4}% avg={:.4}% tp_p90={:.4}% samples={} window={} gain={:.4} ntps={}",
            fmt_time(cur.time_ms),
            candle_pct(cur),
            if signal.dir > 0 { "LONG" } else if signal.dir < 0 { "SHORT" } else { "FLAT" },
            phase,
            signal_kind,
            size_pct,
            avg_pct,
            tp_p90_pct,
            signal.samples,
            cascade_window(&state, cur.close),
            cur.gain,
            cur.ntps
        ));
        if state.samples != prev_samples {
            traces.push(format!(
                "{} commit samples={} avg={:.4}% window={} prev_window={} block={:.4}% dir={} phase={:?}",
                fmt_time(cur.time_ms),
                state.samples,
                block_pct(state.avg_size, cur.close),
                cascade_window(&state, cur.close),
                prev_window,
                size_pct,
                if signal.dir > 0 { "LONG" } else if signal.dir < 0 { "SHORT" } else { "FLAT" },
                phase
            ));
        }

        if active.is_none() {
            if signal_kind == "reverse-ready" && signal.dir != 0 {
                active = open_virtual_trade(
                    &state,
                    cur,
                    -signal.dir,
                    "reverse-entry",
                    tp_mult,
                    sl_mult,
                );
            } else if signal_kind == "safe-continuation" {
                active = open_virtual_trade(
                    &state,
                    cur,
                    signal.dir,
                    "continuation-entry",
                    tp_mult,
                    sl_mult,
                );
            }
            if let Some(trade) = active.as_ref() {
                events.push(TradeEvent {
                    kind: "enter",
                    side: if trade.side > 0 { "LONG" } else { "SHORT" },
                    time_ms: cur.time_ms,
                    reason: trade.reason.clone(),
                    pnl_pct: 0.0,
                });
            }
        }

        if let Some(trade) = active.as_mut() {
            if trade.side > 0 {
                trade.max_favorable_pct = trade.max_favorable_pct.max(cur.high - trade.entry_pct);
                trade.max_adverse_pct = trade.max_adverse_pct.min(cur.low - trade.entry_pct);
                if cur.low <= trade.sl_pct {
                    let result = finalize_trade(trade, trade.sl_pct, "sl", cur.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "LONG",
                        time_ms: cur.time_ms,
                        reason: "sl".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    trades.push(result);
                    active = None;
                } else if cur.high >= trade.tp_pct {
                    let result = finalize_trade(trade, trade.tp_pct, "tp", cur.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "LONG",
                        time_ms: cur.time_ms,
                        reason: "tp".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    trades.push(result);
                    active = None;
                }
            } else {
                trade.max_favorable_pct = trade.max_favorable_pct.max(trade.entry_pct - cur.low);
                trade.max_adverse_pct = trade.max_adverse_pct.min(trade.entry_pct - cur.high);
                if cur.high >= trade.sl_pct {
                    let result = finalize_trade(trade, trade.sl_pct, "sl", cur.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "SHORT",
                        time_ms: cur.time_ms,
                        reason: "sl".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    trades.push(result);
                    active = None;
                } else if cur.low <= trade.tp_pct {
                    let result = finalize_trade(trade, trade.tp_pct, "tp", cur.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "SHORT",
                        time_ms: cur.time_ms,
                        reason: "tp".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    trades.push(result);
                    active = None;
                }
            }
        }
        if idx == 0 {
            traces.push(format!(
                "seeded avg={:.4}% dir={}",
                avg_pct,
                if signal.dir > 0 { "LONG" } else if signal.dir < 0 { "SHORT" } else { "FLAT" }
            ));
        }
    }

    if let Some(trade) = active {
        let last = candles.last().unwrap();
        trades.push(finalize_trade(&trade, last.close, "eod", last.time_ms));
    }

    (trades, traces, events)
}

fn print_replay(symbol_key: &str, candles: &[CandleRow], results: &[TradeResult], traces: &[String], events: &[TradeEvent]) {
    println!("Movement replay: {symbol_key}");
    println!("candles={}", candles.len());
    for line in traces {
        println!("{line}");
    }
    for event in events {
        if event.kind == "enter" {
            println!(
                "{} ENTER {} reason={}",
                fmt_time(event.time_ms),
                event.side,
                event.reason
            );
        } else {
            println!(
                "{} EXIT {} pnl={:.3}% reason={}",
                fmt_time(event.time_ms),
                event.side,
                event.pnl_pct,
                event.reason
            );
        }
    }
    println!("{}", summarize_results(results));
}

fn live_trace(symbol: &str, duration_s: u64, raw: bool) -> Result<(), Box<dyn Error>> {
    let mut child = Command::new("python3")
        .args(["../scripts/sug_ws.py", "--symbol", symbol, "--stream", "--raw"])
        .stdout(Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().ok_or("missing python stdout")?;
    let reader = BufReader::new(stdout);
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(duration_s);
    let mut state = CascadeState::default();
    let mut active: Option<VirtualTrade> = None;
    let mut last_row: Option<CandleRow> = None;
    let mut trades: Vec<TradeResult> = Vec::new();
    let mut events: Vec<TradeEvent> = Vec::new();

    while Instant::now() < deadline && !INTERRUPTED.load(Ordering::SeqCst) {
        let Ok(line) = rx.recv_timeout(Duration::from_secs(1)) else {
            continue;
        };
        let frame: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if frame.get("event").and_then(Value::as_str) != Some("Projection") {
            continue;
        }
        if raw {
            println!("{line}");
            continue;
        }

        let candle = frame.get("candle").unwrap_or(&Value::Null);
        let indicator = frame.get("indicator").unwrap_or(&Value::Null);
        let row = CandleRow {
            time_ms: get_i64(&frame, "time"),
            open: f64_or(candle, "enter", "open"),
            high: f64_or(candle, "max", "high"),
            low: f64_or(candle, "min", "low"),
            close: f64_or(candle, "exit", "close"),
            dp_dir: 0,
            zz_dir: 0,
            gain: f64_or(indicator, "ntps", "ntps"),
            ntps: get_i64(indicator, "ntps"),
        };
        let signal = cascade::update_from_candle(&mut state, &to_candle(&row));

            println!(
                "{} dir={} phase={:?} signal={} candle={:.4}% avg={:.4}% tp_p90={:.4}% size={:.4}% samples={}",
                fmt_time(row.time_ms),
                if signal.dir > 0 { "LONG" } else if signal.dir < 0 { "SHORT" } else { "FLAT" },
                signal.phase,
                movement_signal(&signal),
                candle_pct(&row),
                block_pct(signal.average_size, row.close),
                block_pct(
                    cascade::cascade_take_profit_distance(&state).unwrap_or(state.avg_size),
                    row.close,
                ),
                block_pct(signal.completed_size, row.close),
                cascade_window(&state, row.close)
            );

        if active.is_none() && movement_signal(&signal) == "reverse-ready" && signal.dir != 0 {
            active = open_virtual_trade(&state, &row, -signal.dir, "reverse-entry", 1.0, 0.8);
            if let Some(trade) = active.as_ref() {
                events.push(TradeEvent {
                    kind: "enter",
                    side: if trade.side > 0 { "LONG" } else { "SHORT" },
                    time_ms: row.time_ms,
                    reason: trade.reason.clone(),
                    pnl_pct: 0.0,
                });
                println!(
                    "{} ENTER {} reason={}",
                    fmt_time(row.time_ms),
                    if trade.side > 0 { "LONG" } else { "SHORT" },
                    trade.reason
                );
            }
        }

        if let Some(trade) = active.as_mut() {
            if trade.side > 0 {
                trade.max_favorable_pct = trade.max_favorable_pct.max(row.high - trade.entry_pct);
                trade.max_adverse_pct = trade.max_adverse_pct.min(row.low - trade.entry_pct);
                if row.low <= trade.sl_pct {
                    let result = finalize_trade(trade, trade.sl_pct, "sl", row.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "LONG",
                        time_ms: row.time_ms,
                        reason: "sl".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    println!(
                        "{} EXIT LONG pnl={:.3}% reason=sl",
                        fmt_time(row.time_ms),
                        result.pnl_pct
                    );
                    trades.push(result);
                    active = None;
                } else if row.high >= trade.tp_pct {
                    let result = finalize_trade(trade, trade.tp_pct, "tp", row.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "LONG",
                        time_ms: row.time_ms,
                        reason: "tp".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    println!(
                        "{} EXIT LONG pnl={:.3}% reason=tp",
                        fmt_time(row.time_ms),
                        result.pnl_pct
                    );
                    trades.push(result);
                    active = None;
                }
            } else {
                trade.max_favorable_pct = trade.max_favorable_pct.max(trade.entry_pct - row.low);
                trade.max_adverse_pct = trade.max_adverse_pct.min(trade.entry_pct - row.high);
                if row.high >= trade.sl_pct {
                    let result = finalize_trade(trade, trade.sl_pct, "sl", row.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "SHORT",
                        time_ms: row.time_ms,
                        reason: "sl".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    println!(
                        "{} EXIT SHORT pnl={:.3}% reason=sl",
                        fmt_time(row.time_ms),
                        result.pnl_pct
                    );
                    trades.push(result);
                    active = None;
                } else if row.low <= trade.tp_pct {
                    let result = finalize_trade(trade, trade.tp_pct, "tp", row.time_ms);
                    events.push(TradeEvent {
                        kind: "exit",
                        side: "SHORT",
                        time_ms: row.time_ms,
                        reason: "tp".to_string(),
                        pnl_pct: result.pnl_pct,
                    });
                    println!(
                        "{} EXIT SHORT pnl={:.3}% reason=tp",
                        fmt_time(row.time_ms),
                        result.pnl_pct
                    );
                    trades.push(result);
                    active = None;
                }
            }
        }

        last_row = Some(row);
    }

    if let Some(active_trade) = active {
        if let Some(last) = last_row {
            let result = finalize_trade(&active_trade, last.close, "interrupt", last.time_ms);
            events.push(TradeEvent {
                kind: "exit",
                side: if active_trade.side > 0 { "LONG" } else { "SHORT" },
                time_ms: last.time_ms,
                reason: "interrupt".to_string(),
                pnl_pct: result.pnl_pct,
            });
            println!(
                "{} EXIT {} pnl={:.3}% reason=interrupt",
                fmt_time(last.time_ms),
                if active_trade.side > 0 { "LONG" } else { "SHORT" },
                result.pnl_pct
            );
            trades.push(result);
        }
    }

    if INTERRUPTED.load(Ordering::SeqCst) {
        println!("\nInterrupted.");
    }
    if !trades.is_empty() {
        println!("{}", summarize_results(&trades));
    }
    if !events.is_empty() {
        println!("events:");
        for event in &events {
            if event.kind == "enter" {
                println!(
                    "{} ENTER {} reason={}",
                    fmt_time(event.time_ms),
                    event.side,
                    event.reason
                );
            } else {
                println!(
                    "{} EXIT {} pnl={:.3}% reason={}",
                    fmt_time(event.time_ms),
                    event.side,
                    event.pnl_pct,
                    event.reason
                );
            }
        }
    }

    let _ = child.kill();
    Ok(())
}

fn usage() {
    eprintln!(
        "movement_research --mode replay|live|both [--symbol-key KEY] [--symbols A,B] [--history N] [--ws-seconds N] [--tp-mult X] [--sl-mult X] [--csv FILE] [--raw]"
    );
}

fn main() -> Result<(), Box<dyn Error>> {
    install_sigint_handler();

    let mut symbol_keys: Vec<String> = Vec::new();
    let mut mode = "replay".to_string();
    let mut history: usize = 60;
    let mut ws_seconds: u64 = 300;
    let mut tp_mult = 1.0;
    let mut sl_mult = 0.8;
    let mut csv = String::new();
    let mut raw = false;
    let mut symbols = String::new();

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--symbol-key" => {
                if let Some(val) = args.get(i + 1) {
                    symbol_keys.push(val.clone());
                    i += 2;
                    continue;
                }
            }
            "--symbols" => {
                if let Some(val) = args.get(i + 1) {
                    symbols = val.clone();
                    i += 2;
                    continue;
                }
            }
            "--history" => {
                if let Some(val) = args.get(i + 1) {
                    history = val.parse().unwrap_or(history);
                    i += 2;
                    continue;
                }
            }
            "--mode" => {
                if let Some(val) = args.get(i + 1) {
                    mode = val.clone();
                    i += 2;
                    continue;
                }
            }
            "--ws-seconds" => {
                if let Some(val) = args.get(i + 1) {
                    ws_seconds = val.parse().unwrap_or(ws_seconds);
                    i += 2;
                    continue;
                }
            }
            "--tp-mult" => {
                if let Some(val) = args.get(i + 1) {
                    tp_mult = val.parse().unwrap_or(tp_mult);
                    i += 2;
                    continue;
                }
            }
            "--sl-mult" => {
                if let Some(val) = args.get(i + 1) {
                    sl_mult = val.parse().unwrap_or(sl_mult);
                    i += 2;
                    continue;
                }
            }
            "--csv" => {
                if let Some(val) = args.get(i + 1) {
                    csv = val.clone();
                    i += 2;
                    continue;
                }
            }
            "--raw" => {
                raw = true;
                i += 1;
                continue;
            }
            "--help" | "-h" => {
                usage();
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    if !symbols.is_empty() {
        symbol_keys.extend(symbols.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
    }
    if symbol_keys.is_empty() {
        symbol_keys.push(DEFAULT_SYMBOL_KEY.to_string());
    }

    let mut csv_rows = Vec::new();

    if mode == "replay" || mode == "both" {
        for symbol_key in &symbol_keys {
            let candles = fetch_history(symbol_key, history)?;
            if candles.len() < 50 {
                eprintln!("{symbol_key}: need at least 50 candles, got {}", candles.len());
                continue;
            }
            let (results, traces, events) = simulate(&candles, tp_mult, sl_mult);
            print_replay(symbol_key, &candles, &results, &traces, &events);
            for result in &results {
                csv_rows.push(format!(
                    "{},{},{:.6},{:.6},{:.6},{}",
                    symbol_key,
                    if result.side > 0 { "LONG" } else { "SHORT" },
                    result.pnl_pct,
                    result.mfe_pct,
                    result.mae_pct,
                    result.reason
                ));
            }
        }
        if !csv.is_empty() {
            std::fs::write(
                &csv,
                format!("symbol,side,pnl_pct,mfe_pct,mae_pct,reason\n{}\n", csv_rows.join("\n")),
            )?;
        }
        if mode == "replay" {
            return Ok(());
        }
    }

    if mode == "live" || mode == "both" {
        for symbol_key in &symbol_keys {
            let symbol = symbol_key.split('#').nth(1).unwrap_or(symbol_key);
            live_trace(symbol, ws_seconds, raw)?;
        }
    }

    Ok(())
}
