#![allow(dead_code)]

use std::error::Error;
use std::fs;

mod research;

use long_scalping::observer::VolumeGainTracker;
use research::data::fetch_history_for_timeframe;
use research::mrc_volume::{summarize_trades, CompletedTrade, MrcVolumeEvent, MrcVolumeState};
use serde_json::{json, Value};

const DEFAULT_SYMBOL_KEY: &str = "OKX#ETH-USDT-SWAP#SWAP";

fn usage() {
    eprintln!("mrc_volume_research [--symbol-key KEY] [--history N] [--revert] [--output FILE]");
}

fn print_new_trades(label: &str, state: &MrcVolumeState, printed: &mut usize) {
    for trade in state.completed_trades.iter().skip(*printed) {
        println!(
            "{} EXIT {} entry={:.6} exit={:.6} pnl={:.4}% opened_at={} closed_at={} reason={}",
            label,
            if trade.side > 0 { "LONG" } else { "SHORT" },
            trade.entry_price,
            trade.exit_price,
            trade.pnl_pct,
            trade.opened_at,
            trade.closed_at,
            trade.exit_reason,
        );
    }
    *printed = state.completed_trades.len();
}

fn print_section(title: &str, body: &str) {
    println!("\n{title}");
    println!("{}", "-".repeat(title.len()));
    for line in body.lines() {
        println!("  {line}");
    }
}

fn distance_label(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.1} candles"))
        .unwrap_or_else(|| "n/a".to_string())
}

fn print_volume_section(title: &str, tracker: &VolumeGainTracker) {
    println!("\n{title}");
    println!("{}", "-".repeat(title.len()));
    println!(
        "  records={} low_gain_candles={} risk={} avg_distance_3={} avg_distance_8={}",
        tracker.records.len(),
        tracker.low_gain_candles,
        tracker.risk(),
        distance_label(tracker.average_distance(3)),
        distance_label(tracker.average_distance(8)),
    );
    if tracker.records.is_empty() {
        println!("  recent records: none");
        return;
    }
    println!("  recent records:");
    for (index, record) in tracker.records.iter().rev().enumerate() {
        println!(
            "    #{} candle={} gain={:.3} bsg={:.3} mrc_zone={} cross_mask=0x{:03x} distance={} low_before={} post={:.6}..{:.6}",
            index + 1,
            record.candle,
            record.gain,
            record.buy_sell_gain,
            record.mrc_zone,
            record.mrc_cross_mask,
            record
                .distance
                .map(|distance| distance.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
            record.low_gain_before,
            record.post_gain_low,
            record.post_gain_high,
        );
    }
}

fn event_json(event: &MrcVolumeEvent) -> Value {
    json!({
        "time_ms": event.time_ms,
        "price": event.price,
        "signal_direction": event.direction,
        "trade_side": event.trade_side,
        "action": event.action,
        "pullback_ready": event.pullback_ready,
        "no_pullback_count": event.no_pullback_count,
        "reverted_line": event.reverted_line,
        "crossed_line": event.crossed_line,
        "previous_direction": event.previous_direction,
        "crossing_kind": event.crossing_kind,
        "pullback_detected": event.pullback_detected,
        "sequence": event.sequence[..event.sequence_len].to_vec(),
        "sequence_times_ms": event.sequence_times_ms[..event.sequence_len].to_vec(),
        "gain": event.gain,
        "buy_sell_gain": event.buy_sell_gain,
        "movement_line": event.line,
        "movement_target": event.target,
        "entry_allowed": event.entry_allowed,
        "entry_skip_reason": event.entry_skip_reason,
        "risk_pct": event.risk_pct,
        "virtual_target": event.virtual_target,
        "virtual_stop": event.virtual_stop,
    })
}

fn trade_json(trade: &CompletedTrade) -> Value {
    json!({
        "side": if trade.side > 0 { "long" } else { "short" },
        "entry_price": trade.entry_price,
        "exit_price": trade.exit_price,
        "pnl_pct": trade.pnl_pct,
        "opened_at": trade.opened_at,
        "closed_at": trade.closed_at,
        "exit_reason": trade.exit_reason,
    })
}

fn trade_summary_json(trades: &[CompletedTrade]) -> Value {
    let wins = trades.iter().filter(|trade| trade.pnl_pct > 0.0).count();
    let losses = trades.len().saturating_sub(wins);
    let total_pnl = trades.iter().map(|trade| trade.pnl_pct).sum::<f64>();
    let average_pnl = if trades.is_empty() {
        0.0
    } else {
        total_pnl / trades.len() as f64
    };
    let average_win = if wins == 0 {
        0.0
    } else {
        trades
            .iter()
            .filter(|trade| trade.pnl_pct > 0.0)
            .map(|trade| trade.pnl_pct)
            .sum::<f64>()
            / wins as f64
    };
    let average_loss = if losses == 0 {
        0.0
    } else {
        trades
            .iter()
            .filter(|trade| trade.pnl_pct <= 0.0)
            .map(|trade| trade.pnl_pct)
            .sum::<f64>()
            / losses as f64
    };
    json!({
        "trades": trades.len(),
        "wins": wins,
        "losses": losses,
        "win_rate_pct": if trades.is_empty() {
            0.0
        } else {
            wins as f64 / trades.len() as f64 * 100.0
        },
        "total_pnl_pct": total_pnl,
        "avg_pnl_pct": average_pnl,
        "avg_win_pct": average_win,
        "avg_loss_pct": average_loss,
    })
}

fn volume_json(tracker: &VolumeGainTracker) -> Value {
    let records = tracker
        .records
        .iter()
        .map(|record| {
            json!({
                "candle": record.candle,
                "gain": record.gain,
                "buy_sell_gain": record.buy_sell_gain,
                "mrc_zone": record.mrc_zone,
                "mrc_cross_mask": record.mrc_cross_mask,
                "distance": record.distance,
                "low_gain_before": record.low_gain_before,
                "post_gain_low": record.post_gain_low,
                "post_gain_high": record.post_gain_high,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "records": records,
        "low_gain_candles": tracker.low_gain_candles,
        "risk": tracker.risk(),
        "average_distance_3": tracker.average_distance(3),
        "average_distance_8": tracker.average_distance(8),
    })
}

fn save_trace(
    path: &str,
    symbol_key: &str,
    history_requested: usize,
    revert: bool,
    minute_count: usize,
    five_minute_count: usize,
    minute_events: &[MrcVolumeEvent],
    five_minute_events: &[MrcVolumeEvent],
    minute: &MrcVolumeState,
    five_minute: &MrcVolumeState,
) -> Result<(), Box<dyn Error>> {
    let trace = json!({
        "symbol_key": symbol_key,
        "history_requested_1m": history_requested,
        "history_requested_5m": (history_requested / 5).max(100),
        "history_loaded": {
            "1m": minute_count,
            "5m": five_minute_count,
        },
        "mode": if revert { "revert" } else { "normal" },
        "timeframes": {
            "1m": {
                "event_count": minute_events.len(),
                "events": minute_events.iter().map(event_json).collect::<Vec<_>>(),
                "trade_summary": trade_summary_json(&minute.completed_trades),
                "trades": minute.completed_trades.iter().map(trade_json).collect::<Vec<_>>(),
                "volume": volume_json(&minute.volume),
            },
            "5m": {
                "event_count": five_minute_events.len(),
                "events": five_minute_events.iter().map(event_json).collect::<Vec<_>>(),
                "trade_summary": trade_summary_json(&five_minute.completed_trades),
                "trades": five_minute.completed_trades.iter().map(trade_json).collect::<Vec<_>>(),
                "volume": volume_json(&five_minute.volume),
            },
        },
    });
    fs::write(path, serde_json::to_string_pretty(&trace)?)?;
    println!("trace  : {path}");
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut symbol_key = DEFAULT_SYMBOL_KEY.to_string();
    let mut history = 500usize;
    let mut revert = false;
    let mut output = None::<String>;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--symbol-key" if args.get(index + 1).is_some() => {
                symbol_key = args[index + 1].clone();
                index += 2;
            }
            "--history" if args.get(index + 1).is_some() => {
                history = args[index + 1].parse().unwrap_or(history);
                index += 2;
            }
            "--revert" => {
                revert = true;
                index += 1;
            }
            "--output" if args.get(index + 1).is_some() => {
                output = Some(args[index + 1].clone());
                index += 2;
            }
            "--help" | "-h" => {
                usage();
                return Ok(());
            }
            _ => index += 1,
        }
    }

    let minute = fetch_history_for_timeframe(&symbol_key, 60, history, None)?;
    let five_minute = fetch_history_for_timeframe(&symbol_key, 300, (history / 5).max(100), None)?;

    let mut minute_state = MrcVolumeState::new(revert);
    let mut five_minute_state = MrcVolumeState::new(revert);
    let mut five_index = 0usize;
    let mut events = 0usize;
    let mut printed_minute_trades = 0usize;
    let mut printed_five_minute_trades = 0usize;
    let mut minute_events = Vec::<MrcVolumeEvent>::new();
    let mut five_minute_events = Vec::<MrcVolumeEvent>::new();

    for row in &minute {
        while five_index < five_minute.len() && five_minute[five_index].time_ms <= row.time_ms {
            if let Some(event) = five_minute_state.observe(&five_minute[five_index]) {
                five_minute_events.push(event);
                println!(
                    "5m time={} cross={} signal_dir={} trade_side={} action={} pullback_ready={} no_pullback={} reverted_line={:?} gain={:.3} bsg={:.3} line={:.6} target={:.6}",
                    event.time_ms,
                    event.price,
                    event.direction,
                    event.trade_side,
                    event.action,
                    event.pullback_ready,
                    event.no_pullback_count,
                    event.reverted_line,
                    event.gain,
                    event.buy_sell_gain,
                    event.line,
                    event.target,
                );
            }
            print_new_trades("5m", &five_minute_state, &mut printed_five_minute_trades);
            five_index += 1;
        }

        if let Some(event) = minute_state.observe(row) {
            events += 1;
            minute_events.push(event);
            let higher_tf_direction = five_minute_state.crossing.movement_direction;
            let higher_tf_allows =
                higher_tf_direction == 0 || higher_tf_direction == event.direction;
            let blocked_by_higher_tf =
                !higher_tf_allows && minute_state.has_position_at(event.time_ms);
            if blocked_by_higher_tf {
                minute_state.clear_position_at(event.time_ms);
            }
            println!(
                "1m time={} cross={} signal_dir={} trade_side={} action={} pullback_ready={} no_pullback={} reverted_line={:?} higher_tf={} gain={:.3} bsg={:.3} line={:.6} target={:.6} trade={}",
                event.time_ms,
                event.price,
                event.direction,
                event.trade_side,
                event.action,
                event.pullback_ready,
                event.no_pullback_count,
                event.reverted_line,
                higher_tf_direction,
                event.gain,
                event.buy_sell_gain,
                event.line,
                event.target,
                if blocked_by_higher_tf {
                    "blocked-5m"
                } else if minute_state.open_position_count() > 0 {
                    "open"
                } else {
                    "flat"
                },
            );
        }
        print_new_trades("1m", &minute_state, &mut printed_minute_trades);
    }

    if let Some(last) = minute.last() {
        minute_state.finish(last);
    }
    if let Some(last) = five_minute.last() {
        five_minute_state.finish(last);
    }
    print_new_trades("1m", &minute_state, &mut printed_minute_trades);
    print_new_trades("5m", &five_minute_state, &mut printed_five_minute_trades);

    println!("\n============================================================");
    println!("MRC / VOLUME RESEARCH SUMMARY");
    println!("============================================================");
    println!("symbol : {symbol_key}");
    println!(
        "mode   : {}",
        if revert { "REVERT (SL↔TP)" } else { "NORMAL" }
    );
    println!(
        "history: 1m={} 5m={} crossing_events={events}",
        minute.len(),
        five_minute.len()
    );
    print_section(
        "1M TRADES",
        &summarize_trades(&minute_state.completed_trades),
    );
    print_section(
        "5M TRADES",
        &summarize_trades(&five_minute_state.completed_trades),
    );
    print_volume_section("1M VOLUME", &minute_state.volume);
    print_volume_section("5M VOLUME", &five_minute_state.volume);
    println!("\n============================================================");
    if let Some(path) = output.as_deref() {
        save_trace(
            path,
            &symbol_key,
            history,
            revert,
            minute.len(),
            five_minute.len(),
            &minute_events,
            &five_minute_events,
            &minute_state,
            &five_minute_state,
        )?;
    }
    Ok(())
}
