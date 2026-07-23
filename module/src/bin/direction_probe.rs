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
#[path = "../format.rs"]
mod format;
#[path = "../detection/levels.rs"]
mod levels_impl;
#[path = "../martingale.rs"]
mod martingale;
#[path = "../model.rs"]
mod model;
#[path = "../detection/mrc.rs"]
mod mrc_impl;
#[path = "../observer/mod.rs"]
mod observer;
#[path = "../orders.rs"]
mod orders;
#[path = "../prediction.rs"]
mod prediction_impl;
#[path = "../detection/regime.rs"]
mod regime_impl;
#[path = "../risk_control.rs"]
mod risk_control;
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
    Candle, EntryGate, EntryWaitSolution, IndicatorSnapshot, MarketState,
    PositionState, SugInfoSnapshot, TimeframeSnapshot,
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

fn f64_key(obj: &Value, key: &str) -> Option<f64> {
    let parse = |v: &Value| {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
    };
    obj.get(key).and_then(parse)
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

    let mean_line = f64_key(mrc, "MeanLine")?;
    let prev_mean_line = prev
        .and_then(|row| row.get("indicators"))
        .and_then(|i| i.get("mrc_200"))
        .and_then(|prev_mrc| f64_key(prev_mrc, "MeanLine"))
        .unwrap_or(mean_line);

    let close = match live_price {
        Some(price) => price,
        None => f64_key(candle, "Close")?,
    };
    let snapshot = TimeframeSnapshot {
        candle: Candle {
            open: f64_key(candle, "Open")?,
            high: f64_key(candle, "High")?,
            low: f64_key(candle, "Low")?,
            close,
        },
        mrc: model::MrcBands {
            down_big: f64_key(mrc, "DownBig")?,
            down_outer: f64_key(mrc, "DownOuter")?,
            down_small: f64_key(mrc, "DownSmall")?,
            down_inner: f64_key(mrc, "DownInner")?,
            mean_line,
            up_inner: f64_key(mrc, "UpInner")?,
            up_small: f64_key(mrc, "UpSmall")?,
            up_outer: f64_key(mrc, "UpOuter")?,
            up_big: f64_key(mrc, "UpBig")?,
            current_cross: f64_key(mrc, "CurrentCross")?,
            mean_line_slope: mean_line - prev_mean_line,
        },
        mrc_signal: mrc::analyze(
            &model::MrcBands {
                down_big: f64_key(mrc, "DownBig")?,
                down_outer: f64_key(mrc, "DownOuter")?,
                down_small: f64_key(mrc, "DownSmall")?,
                down_inner: f64_key(mrc, "DownInner")?,
                mean_line,
                up_inner: f64_key(mrc, "UpInner")?,
                up_small: f64_key(mrc, "UpSmall")?,
                up_outer: f64_key(mrc, "UpOuter")?,
                up_big: f64_key(mrc, "UpBig")?,
                current_cross: f64_key(mrc, "CurrentCross")?,
                mean_line_slope: mean_line - prev_mean_line,
            },
            close,
        ),
        dp_dir: dir_to_int(dp.get("Direction")),
        zz_dir: dir_to_int(zz.get("Direction")),
        dp_zee: f64_key(dp, "Zee")?,
        dp_zee1: f64_key(dp, "Zee1")?,
        zz_buy_price: f64_key(zz, "BuyPrice")?,
        zz_sell_price: f64_key(zz, "SellPrice")?,
        zz_break_price: f64_key(zz, "BreakPrice")?,
        volume_gain: f64_key(volume_50, "Gain")?,
        volume_gain_min: f64_key(volume_50, "GainMin")?,
        volume_gain_max: f64_key(volume_50, "GainMax")?,
        buy_sell_gain: f64_key(volume_50, "BuySellGain")?,
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
        EntryGate::Open { notes, reason } => {
            if notes.is_empty() {
                format!("open:{}", reason.label())
            } else {
                format!("open:{}|{}", reason.label(), notes.join("|"))
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

fn note_value(notes: &[String], key: &str) -> Option<f64> {
    let prefix = format!("{key}=");
    notes.iter().find_map(|note| {
        note.strip_prefix(&prefix)
            .and_then(|value| value.parse::<f64>().ok())
    })
}

fn format_level_point(point: &detection::levels::LevelPoint, price: f64) -> String {
    let distance_pct = if price > 0.0 {
        ((point.value - price).abs() / price) * 100.0
    } else {
        0.0
    };
    format!(
        "{:.6}@{}:{}({:+.2}%)",
        point.value,
        point.source.as_str(),
        point.kind.as_str(),
        distance_pct
    )
}

fn format_level_points(
    points: &[detection::levels::LevelPoint],
    price: f64,
    limit: usize,
) -> String {
    let mut out = Vec::new();
    for point in points.iter().take(limit) {
        out.push(format_level_point(point, price));
    }
    if points.len() > limit {
        out.push(format!("+{} more", points.len() - limit));
    }
    if out.is_empty() {
        "none".to_string()
    } else {
        out.join(", ")
    }
}

fn format_gap_summary(summary: &detection::levels::LevelGapSummary, price: f64) -> String {
    let above = summary
        .nearest_above
        .map(|point| format_level_point(&point, price))
        .unwrap_or_else(|| "none".to_string());
    let below = summary
        .nearest_below
        .map(|point| format_level_point(&point, price))
        .unwrap_or_else(|| "none".to_string());
    let above_gap = summary
        .nearest_above_gap
        .map(|gap| format!("{:.6}", gap))
        .unwrap_or_else(|| "n/a".to_string());
    let below_gap = summary
        .nearest_below_gap
        .map(|gap| format!("{:.6}", gap))
        .unwrap_or_else(|| "n/a".to_string());
    let above_wall = summary
        .next_above_gap
        .map(|gap| format!("{:.6}", gap))
        .unwrap_or_else(|| "n/a".to_string());
    let below_wall = summary
        .next_below_gap
        .map(|gap| format!("{:.6}", gap))
        .unwrap_or_else(|| "n/a".to_string());
    let span = summary
        .span
        .map(|gap| format!("{:.6}", gap))
        .unwrap_or_else(|| "n/a".to_string());

    format!(
        "nearest_above={} gap={} wall={} nearest_below={} gap={} wall={} span={}",
        above, above_gap, above_wall, below, below_gap, below_wall, span
    )
}

fn near_price(a: f64, b: f64) -> bool {
    if a <= 0.0 || b <= 0.0 {
        return (a - b).abs() <= f64::EPSILON;
    }
    (a - b).abs() <= (a.max(b) * 0.0005).max(0.000_001)
}

fn gate_entry_price(gate: &EntryGate, fallback: f64) -> f64 {
    match gate {
        EntryGate::Open { notes, reason } => reason
            .price()
            .or_else(|| note_value(notes, "entry"))
            .or_else(|| note_value(notes, "pullback_entry"))
            .or_else(|| note_value(notes, "price"))
            .unwrap_or(fallback),
        EntryGate::Wait(wait) => {
            for solution in &wait.solutions {
                match solution {
                    EntryWaitSolution::WaitForDirectionBreak { break_price, .. } => {
                        return *break_price;
                    }
                    EntryWaitSolution::WaitForConfirmation { price, .. } => return *price,
                    EntryWaitSolution::ReclaimLevel { level, .. } => return *level,
                    EntryWaitSolution::ReclaimDpsignal { anchor, .. } => return *anchor,
                    EntryWaitSolution::FastCandle { .. } => {}
                    EntryWaitSolution::WaitForSamples { .. } => {}
                }
            }
            fallback
        }
    }
}

fn break_candidate(
    price: f64,
    side: PositionState,
    value: f64,
    source: &'static str,
) -> Option<(f64, &'static str)> {
    if value <= 0.0 || near_price(price, value) {
        return None;
    }
    match side {
        PositionState::Long if value < price => Some((value, source)),
        PositionState::Short if value > price => Some((value, source)),
        PositionState::Flat => None,
        _ => None,
    }
}

fn directional_break_level(
    snapshot: &IndicatorSnapshot,
    gate: &EntryGate,
    side: PositionState,
    entry_price: f64,
) -> Option<(f64, &'static str)> {
    let current_price = snapshot.tf1m.candle.close.max(f64::EPSILON);
    let mut candidates: Vec<(f64, &'static str)> = Vec::new();

    if let EntryGate::Wait(wait) = gate {
        for solution in &wait.solutions {
            match solution {
                EntryWaitSolution::WaitForDirectionBreak { break_price, .. } => {
                    if let Some(candidate) =
                        break_candidate(current_price, side, *break_price, "wait-break")
                    {
                        candidates.push(candidate);
                    }
                }
                EntryWaitSolution::WaitForConfirmation { price, .. } => {
                    if let Some(candidate) =
                        break_candidate(current_price, side, *price, "wait-confirmation")
                    {
                        candidates.push(candidate);
                    }
                }
                EntryWaitSolution::ReclaimLevel { level, .. } => {
                    if let Some(candidate) =
                        break_candidate(current_price, side, *level, "reclaim-level")
                    {
                        candidates.push(candidate);
                    }
                }
                EntryWaitSolution::ReclaimDpsignal { anchor, .. } => {
                    if let Some(candidate) =
                        break_candidate(current_price, side, *anchor, "dpsignal")
                    {
                        candidates.push(candidate);
                    }
                }
                EntryWaitSolution::FastCandle { .. } => {}
                EntryWaitSolution::WaitForSamples { .. } => {}
            }
        }
    }

    let tf5 = &snapshot.tf5m;
    let level_book = detection::levels::structural_levels(snapshot);
    let tf5_candidates = match side {
        PositionState::Long => [
            (tf5.zz_break_price, "5m-zz-break"),
            (tf5.dp_zee1, "5m-dp-zee1"),
            (tf5.dp_zee, "5m-dp-zee"),
            (tf5.mrc.down_outer, "5m-down-outer"),
            (tf5.mrc.down_big, "5m-down-big"),
            (
                match level_book.nearest_below(current_price) {
                    Some(level) => level,
                    None => return None,
                },
                "structural-below",
            ),
        ],
        PositionState::Short => [
            (tf5.zz_break_price, "5m-zz-break"),
            (tf5.dp_zee1, "5m-dp-zee1"),
            (tf5.dp_zee, "5m-dp-zee"),
            (tf5.mrc.up_outer, "5m-up-outer"),
            (tf5.mrc.up_big, "5m-up-big"),
            (
                match level_book.nearest_above(current_price) {
                    Some(level) => level,
                    None => return None,
                },
                "structural-above",
            ),
        ],
        PositionState::Flat => return None,
    };

    candidates.extend(
        tf5_candidates
            .into_iter()
            .filter_map(|(value, source)| break_candidate(current_price, side, value, source)),
    );

    let mut candidates: Vec<(f64, &'static str)> = candidates
        .into_iter()
        .filter(|(value, _)| !near_price(entry_price, *value))
        .collect();

    candidates.sort_by(|(a, _), (b, _)| match side {
        PositionState::Long => b.total_cmp(a),
        PositionState::Short => a.total_cmp(b),
        PositionState::Flat => a.total_cmp(b),
    });

    candidates.into_iter().next()
}

fn gate_break_instruction(
    gate: &EntryGate,
    side: PositionState,
    snapshot: &IndicatorSnapshot,
    entry_price: f64,
) -> String {
    if let Some((break_level, source)) = directional_break_level(snapshot, gate, side, entry_price)
    {
        let trigger = match side {
            PositionState::Long => "reclaim above",
            PositionState::Short => "break below",
            PositionState::Flat => "watch",
        };
        return format!("{} {:.6} via {}", trigger, break_level, source);
    }

    "wait-for-better-anchor".to_string()
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
    let entry_price = gate_entry_price(&recommendation.gate, snapshot.tf1m.candle.close);
    let level_book = detection::levels::structural_levels(&snapshot);
    let level_gaps = level_book.gap_summary(snapshot.tf1m.candle.close);
    let exit_plan = exit::compute(
        &snapshot,
        &SugInfoSnapshot::default(),
        &cascade_state,
        entry_price,
        recommendation_side,
        false,
    );

    println!("=== {} ===", symbol_key);
    println!(
        "recommendation side={:?} enter_now={} reason={} 1m_confirmation={} enter_price={:.6} exit_tp={:.6} exit_sl={:.6} tp_source={:?} break={}",
        recommendation.side,
        recommendation.enter_now,
        recommendation.reason,
        confirmation_label(confirmation),
        entry_price,
        match exit_plan.as_ref() {
            Some(plan) => plan.take_profit.price(),
            None => 0.0,
        },
        match exit_plan.as_ref() {
            Some(plan) => plan.stop_loss.price(),
            None => 0.0,
        },
        exit_plan
            .as_ref()
            .map(|plan| plan.take_profit.source_label()),
        gate_break_instruction(
            &recommendation.gate,
            recommendation.side,
            &snapshot,
            entry_price
        )
    );

    if opts.mode == ProbeMode::Full {
        println!("{}", tf_line("1m", &snapshot.tf1m));
        println!("{}", tf_line("5m", &snapshot.tf5m));
        println!("{}", tf_line("15m", &snapshot.tf15m));
        println!("{}", tf_line("1h", &snapshot.tf1h));
        println!(
            "levels above=[{}] below=[{}]",
            format_level_points(&level_book.above, snapshot.tf1m.candle.close, 6),
            format_level_points(&level_book.below, snapshot.tf1m.candle.close, 6)
        );
        println!(
            "levels_gap {}",
            format_gap_summary(&level_gaps, snapshot.tf1m.candle.close)
        );
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
                    "expected_exit source={} tp={:.6} sl={:.6} sl_reason={} reliable={}",
                    plan.take_profit.source_label(),
                    plan.take_profit.price(),
                    plan.stop_loss.price(),
                    plan.stop_loss.reason(),
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
                "history[{}] recommendation={:?} enter_now={} reason={} 1m_confirmation={} enter_price={:.6} long={} short={} cascade dir={} samples={} avg={:.6}",
                window,
                recommendation.side,
                recommendation.enter_now,
                recommendation.reason,
                confirmation_label(confirmation),
                gate_entry_price(&recommendation.gate, snapshot.tf1m.candle.close),
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
