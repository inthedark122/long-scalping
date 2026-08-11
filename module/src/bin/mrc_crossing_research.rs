#![allow(dead_code)]

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use long_scalping::model::Config;

mod research;

use research::data::fetch_history_for_timeframe;
use research::mrc_crossing::{AnalysisReport, Analyzer};
use research::mrc_crossing_html;
use serde_json::json;

const DEFAULT_SYMBOL_KEY: &str = "OKX#ETH-USDT-SWAP#SWAP";

fn usage() {
    eprintln!(
        "mrc_crossing_research [--symbol-key KEY ...] [--symbols KEY1,KEY2] [--history N] [--required-samples N] [--output FILE|DIR] [--html FILE|DIR]"
    );
}

fn symbol_slug(symbol_key: &str) -> String {
    symbol_key
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn symbol_output_path(
    requested: &str,
    symbol_key: &str,
    extension: &str,
    multiple_symbols: bool,
) -> Result<PathBuf, Box<dyn Error>> {
    let requested_path = Path::new(requested);
    let folder_style = requested_path.is_dir()
        || requested.ends_with('/')
        || requested.ends_with('\\')
        || requested_path.extension().is_none();
    if !folder_style {
        if multiple_symbols {
            return Err(format!(
                "output path '{}' is a file; use a directory when processing multiple symbols",
                requested
            )
            .into());
        }
        return Ok(requested_path.to_path_buf());
    }

    if !requested_path.exists() {
        fs::create_dir_all(requested_path)?;
    }
    Ok(requested_path.join(format!(
        "mrc-crossing-{}.{}",
        symbol_slug(symbol_key),
        extension
    )))
}

fn format_minutes(value_ms: Option<f64>) -> String {
    value_ms
        .map(|value| format!("{:.2} minutes", value / 60_000.0))
        .unwrap_or_else(|| "n/a".to_string())
}

fn print_trade_log(report: &AnalysisReport) {
    if report.trades.is_empty() && report.open_trade.is_none() {
        println!("positions               : none");
        return;
    }

    println!("positions:");
    for (index, trade) in report.trades.iter().enumerate() {
        println!(
            "  #{} {} {} -> {} entry={:.6} target={:.6} stop={:.6} exit={:.6} outcome={} gross={:+.4}% net={:+.4}% fees={:.4}% mfe={:.4}% mae={:.4}% duration={}",
            index + 1,
            if trade.side > 0 { "LONG" } else { "SHORT" },
            trade.source_name,
            trade.target_name,
            trade.entry_price,
            trade.target_price,
            trade.stop_price,
            trade.exit_price,
            trade.exit_reason,
            trade.gross_pnl_pct,
            trade.pnl_pct,
            trade.entry_fee_pct + trade.exit_fee_pct,
            trade.mfe_pct,
            trade.mae_pct,
            format_minutes(Some(
                trade.closed_at.saturating_sub(trade.opened_at) as f64
            )),
        );
    }
    if let Some(trade) = &report.open_trade {
        println!(
            "  #open {} {} -> {} entry={:.6} target={:.6} stop={:.6} opened_at={} reason={}",
            if trade.side > 0 { "LONG" } else { "SHORT" },
            trade.source_name,
            trade.target_name,
            trade.entry_price,
            trade.target_price,
            trade.stop_price,
            trade.opened_at,
            trade.reason,
        );
    }
}

fn print_summary(
    symbol_key: &str,
    minute_count: usize,
    hourly_count: usize,
    report: &AnalysisReport,
    print_positions: bool,
) {
    println!("MRC CROSSING RESEARCH");
    println!("symbol                 : {symbol_key}");
    println!("history                : 1m={minute_count} 1h={hourly_count}");
    println!("crossings              : {}", report.crossing_count);
    println!("transitions            : {}", report.transition_count);
    println!("break-line events      : {}", report.break_event_count);
    println!("scenario events        : {}", report.scenario_event_count);
    if let Some(event) = report.latest_event {
        println!(
            "latest                 : {} line={} direction={} ntps={} small_ntps={} frequent={} break={} mode={}",
            event.time_ms,
            event.line_name,
            event.direction,
            event.ntps,
            event.small_ntps,
            event.frequent_nearest_crossings,
            event.break_line,
            event.mode,
        );
    }
    println!("forecast source        : {:?}", report.forecast.source_name);
    println!("forecast condition     : {}", report.forecast.condition);
    println!("forecast samples       : {}", report.forecast.sample_count);
    println!(
        "probability next cross : {}",
        report
            .forecast
            .probability_next_crossing
            .map(|value| format!("{:.1}%", value * 100.0))
            .unwrap_or_else(|| "n/a".to_string())
    );
    println!(
        "estimated time         : {}",
        format_minutes(report.forecast.estimated_time_ms)
    );
    if !report.forecast.likely_destinations.is_empty() {
        println!("likely destinations:");
        for destination in &report.forecast.likely_destinations {
            println!(
                "  {} -> {} probability={:.1}% time={}",
                destination.from_name,
                destination.to_name,
                destination.probability * 100.0,
                format_minutes(destination.average_duration_ms),
            );
        }
    }
    let mean_transitions = report
        .transitions
        .iter()
        .filter(|transition| transition.from == 4)
        .collect::<Vec<_>>();
    if !mean_transitions.is_empty() {
        println!("mean-line transitions:");
        for transition in mean_transitions {
            println!(
                "  mean -> {} probability={:.1}% time={}",
                transition.to_name,
                transition.probability * 100.0,
                format_minutes(transition.average_duration_ms),
            );
        }
    }
    println!(
        "virtual trades         : {} wins={} losses={} win_rate={:.1}% gross={:.3}% fees={:.3}% net={:.3}% max_drawdown={:.3}%",
        report.trade_summary.trades,
        report.trade_summary.wins,
        report.trade_summary.losses,
        report.trade_summary.win_rate_pct,
        report.trade_summary.total_gross_pnl_pct,
        report.trade_summary.total_fees_pct,
        report.trade_summary.total_pnl_pct,
        report.trade_summary.max_drawdown_pct,
    );
    println!(
        "trade wait mode        : {} direction={}",
        report.wait_mode, report.wait_direction
    );
    if print_positions {
        print_trade_log(report);
    }
}

fn analyze_symbol(
    symbol_key: &str,
    history: usize,
    required_samples: u32,
) -> Result<(usize, usize, AnalysisReport), Box<dyn Error>> {
    let minute = fetch_history_for_timeframe(symbol_key, 60, history, None)?;
    let hourly_limit = ((history + 3_599) / 3_600 + 10).max(10);
    let hourly = fetch_history_for_timeframe(symbol_key, 3_600, hourly_limit, None)?;
    let mut analyzer = Analyzer::new(required_samples);
    let mut hourly_index = 0usize;
    let mut active_ntps = Vec::new();

    for row in &minute {
        while hourly_index < hourly.len() && hourly[hourly_index].time_ms <= row.time_ms {
            active_ntps.push(hourly[hourly_index].ntps);
            if active_ntps.len() > 10 {
                active_ntps.remove(0);
            }
            hourly_index += 1;
        }
        analyzer.observe(row, &active_ntps);
    }
    if let Some(last) = minute.last() {
        analyzer.finish(last);
    }

    Ok((minute.len(), hourly.len(), analyzer.report()))
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut symbol_keys = Vec::new();
    let mut history = 500usize;
    let mut required_samples = Config::default().required_samples();
    let mut output = None::<String>;
    let mut html = None::<String>;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--symbol-key" if args.get(index + 1).is_some() => {
                symbol_keys.push(args[index + 1].clone());
                index += 2;
            }
            "--symbols" if args.get(index + 1).is_some() => {
                symbol_keys.extend(
                    args[index + 1]
                        .split(',')
                        .map(str::trim)
                        .filter(|symbol| !symbol.is_empty())
                        .map(str::to_string),
                );
                index += 2;
            }
            "--history" if args.get(index + 1).is_some() => {
                history = args[index + 1].parse().unwrap_or(history);
                index += 2;
            }
            "--required-samples" if args.get(index + 1).is_some() => {
                required_samples = args[index + 1].parse().unwrap_or(required_samples);
                index += 2;
            }
            "--output" if args.get(index + 1).is_some() => {
                output = Some(args[index + 1].clone());
                index += 2;
            }
            "--html" if args.get(index + 1).is_some() => {
                html = Some(args[index + 1].clone());
                index += 2;
            }
            "--help" | "-h" => {
                usage();
                return Ok(());
            }
            _ => index += 1,
        }
    }

    if symbol_keys.is_empty() {
        symbol_keys.push(DEFAULT_SYMBOL_KEY.to_string());
    }
    symbol_keys.dedup();
    let multiple_symbols = symbol_keys.len() > 1;
    let print_positions = html.is_none();

    for (symbol_index, symbol_key) in symbol_keys.iter().enumerate() {
        if multiple_symbols {
            println!(
                "\n==================== {}/{} ====================",
                symbol_index + 1,
                symbol_keys.len()
            );
        }
        let (minute_count, hourly_count, report) =
            analyze_symbol(symbol_key, history, required_samples)?;
        print_summary(
            symbol_key,
            minute_count,
            hourly_count,
            &report,
            print_positions,
        );
        if let Some(requested) = output.as_deref() {
            let path = symbol_output_path(requested, symbol_key, "json", multiple_symbols)?;
            let document = json!({
                "symbol_key": symbol_key,
                "history_requested_1m": history,
                "required_samples": required_samples,
                "history_loaded": {"1m": minute_count, "1h": hourly_count},
                "ntps_baseline": {"timeframe": "1h", "lookback_candles": 10, "small_ntps": "lower-third"},
                "crossing_features": {"evidence": "wick-penetration", "frequent_window_minutes": 30, "frequent_threshold": 2, "break_minutes": 30},
                "analysis": &report,
            });
            fs::write(&path, serde_json::to_string_pretty(&document)?)?;
            println!("json output            : {}", path.display());
        }
        if let Some(requested) = html.as_deref() {
            let path = symbol_output_path(requested, symbol_key, "html", multiple_symbols)?;
            mrc_crossing_html::write_report(
                path.to_string_lossy().as_ref(),
                symbol_key,
                history,
                minute_count,
                hourly_count,
                &report,
            )?;
            println!("html report            : {}", path.display());
            let absolute_path = path.canonicalize().unwrap_or(path);
            let file_url = absolute_path.to_string_lossy().replace(' ', "%20");
            println!("open html              : file://{file_url}");
        }
    }
    Ok(())
}
