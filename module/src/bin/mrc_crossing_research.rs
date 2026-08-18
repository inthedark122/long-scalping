#![allow(dead_code)]

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

mod research;

use long_scalping::detection::decision::DecisionComponents;
use research::data::{fetch_history_for_timeframe_cached, CacheSettings, CacheStatus};
use research::mrc_crossing::{AnalysisReport, Analyzer};
use research::mrc_crossing_html;
use serde_json::json;

const DEFAULT_SYMBOL_KEY: &str = "OKX#ETH-USDT-SWAP#SWAP";

fn usage() {
    eprintln!(
        "mrc_crossing_research [--symbol-key KEY ...] [--symbols KEY1,KEY2] [--direction both|long|short] [--primary all|stairs|outward] [--history N] [--end-time UNIX_MS] [--no-cache] [--clear-cache] [--refresh-cache] [--output FILE|DIR] [--html FILE|DIR]"
    );
}

fn parse_direction(value: &str) -> Result<Option<i8>, Box<dyn Error>> {
    match value.to_ascii_lowercase().as_str() {
        "both" | "auto" => Ok(None),
        "long" => Ok(Some(1)),
        "short" => Ok(Some(-1)),
        _ => Err(format!("invalid direction '{value}', expected both, long, or short").into()),
    }
}

fn parse_primary(value: &str) -> Result<DecisionComponents, Box<dyn Error>> {
    DecisionComponents::for_primary_mode(value).ok_or_else(|| {
        format!("invalid primary mode '{value}', expected all, stairs, or outward").into()
    })
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
            "  #{} {} {} -> {} entry={:.6} target={:.6} stop={:.6} exit={:.6} outcome={} gross={:+.4}% net={:+.4}% fees={:.4}% duration={}",
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
            format_minutes(Some(
                trade.closed_at.saturating_sub(trade.opened_at) as f64
            )),
        );
        println!("      reason={}", trade.reason);
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
    minute_cache: CacheStatus,
    report: &AnalysisReport,
    print_positions: bool,
) {
    println!("MRC CROSSING RESEARCH");
    println!("symbol                 : {symbol_key}");
    println!("history                : 1m={minute_count}");
    println!("cache                  : 1m={minute_cache}");
    println!("crossings              : {}", report.crossing_count);
    println!(
        "positions              : {} wins={} losses={} win_rate={:.1}% gross={:+.3}% fees={:.3}% net={:+.3}% max_drawdown={:.3}%",
        report.trade_summary.trades,
        report.trade_summary.wins,
        report.trade_summary.losses,
        report.trade_summary.win_rate_pct,
        report.trade_summary.total_gross_pnl_pct,
        report.trade_summary.total_fees_pct,
        report.trade_summary.total_pnl_pct,
        report.trade_summary.max_drawdown_pct,
    );
    if let Some(event) = report.latest_event.as_ref() {
        println!(
            "latest                 : {} line={} direction={}",
            event.time_ms, event.line_name, event.direction,
        );
        println!("mrc_cross             : {}", event.mrc_trace);
    }
    println!("forecast source        : {:?}", report.forecast.source_name);
    println!("forecast condition     : {}", report.forecast.condition);
    println!("forecast samples       : {}", report.forecast.sample_count);
    println!(
        "estimated time         : {}",
        format_minutes(report.forecast.estimated_time_ms)
    );
    if !report.forecast.likely_destinations.is_empty() {
        for destination in &report.forecast.likely_destinations {
            println!(
                "forecast pattern      : {} target={} time={}",
                destination.pattern_kind,
                destination.to_name,
                format_minutes(destination.average_duration_ms),
            );
        }
    }
    println!(
        "virtual predictions    : {} filled={} followed={} reached={} missed_entry={} broken={} unresolved={} followed_rate={:.1}% broken_rate={:.1}% active={} pending={}",
        report.prediction_summary.predictions,
        report.prediction_summary.filled,
        report.prediction_summary.reached_and_followed
            + report.prediction_summary.not_reached_but_followed,
        report.prediction_summary.reached_and_followed,
        report.prediction_summary.not_reached_but_followed,
        report.prediction_summary.prediction_broken,
        report.prediction_summary.unresolved,
        report.prediction_summary.followed_rate_pct,
        report.prediction_summary.broken_rate_pct,
        report.active_predictions,
        report.pending_predictions,
    );
    println!(
        "active pattern         : {:?} score={:+.4}%",
        report.active_pattern, report.active_pattern_score,
    );
    println!("adaptive pattern PnL:");
    for pattern in &report.pattern_performance {
        println!(
            "  pattern={} observations={} trades={} win_rate={:.1}% total={:+.3}% avg={:+.3}% ema={:+.3}% last={:+.3}%",
            pattern.pattern,
            pattern.observations,
            pattern.trades,
            pattern.win_rate_pct,
            pattern.total_pnl_pct,
            pattern.average_pnl_pct,
            pattern.ema_pnl_pct,
            pattern.last_pnl_pct,
        );
    }
    if !report.pattern_win_rates.is_empty() {
        println!("win rate by pattern:");
        for pattern in &report.pattern_win_rates {
            println!(
                "  pattern={} matches={} filled={} closed={} wins={} losses={} win_rate={:.1}% followed={} broken={} gross={:+.3}% fees={:.3}% net={:+.3}% avg_win={:+.3}% avg_loss={:+.3}%",
                pattern.pattern,
                pattern.matches,
                pattern.filled_predictions,
                pattern.closed_positions,
                pattern.wins,
                pattern.losses,
                pattern.win_rate_pct,
                pattern.followed_predictions,
                pattern.broken_predictions,
                pattern.total_gross_pnl_pct,
                pattern.total_fees_pct,
                pattern.total_pnl_pct,
                pattern.average_win_pnl_pct,
                pattern.average_loss_pnl_pct,
            );
        }
    }
    if print_positions {
        print_trade_log(report);
    }
}

fn analyze_symbol(
    symbol_key: &str,
    history: usize,
    end_time_ms: Option<i64>,
    direction_lock: Option<i8>,
    decision_components: DecisionComponents,
    cache_settings: CacheSettings,
) -> Result<(usize, CacheStatus, AnalysisReport), Box<dyn Error>> {
    let started = Instant::now();
    eprintln!(
        "[mrc-research][symbol] start symbol={} history={} end_time_ms={:?}",
        symbol_key, history, end_time_ms
    );
    let (minute, minute_cache) =
        fetch_history_for_timeframe_cached(symbol_key, 60, history, None, cache_settings)?;
    eprintln!(
        "[mrc-research][symbol] loaded-1m symbol={} rows={} cache_status={} elapsed_ms={}",
        symbol_key,
        minute.len(),
        minute_cache,
        started.elapsed().as_millis()
    );
    let minute = minute
        .into_iter()
        .filter(|row| end_time_ms.is_none_or(|end| row.time_ms < end))
        .collect::<Vec<_>>();
    let mut analyzer = Analyzer::new(direction_lock, decision_components);

    for row in &minute {
        analyzer.observe(row);
    }
    if let Some(last) = minute.last() {
        analyzer.finish(last);
    }

    Ok((minute.len(), minute_cache, {
        let report = analyzer.report();
        eprintln!(
                "[mrc-research][symbol] complete symbol={} rows_1m={} crossings={} trades={} elapsed_ms={}",
                symbol_key,
                minute.len(),
                report.crossing_count,
                report.prediction_summary.predictions,
                started.elapsed().as_millis()
            );
        report
    }))
}

fn main() -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut symbol_keys = Vec::new();
    let mut history = 500usize;
    let mut direction_lock = None::<i8>;
    let mut decision_components = DecisionComponents::default();
    let mut end_time_ms = None::<i64>;
    let mut output = None::<String>;
    let mut html = None::<String>;
    let mut cache_settings = CacheSettings::default();
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
            "--direction" if args.get(index + 1).is_some() => {
                direction_lock = parse_direction(&args[index + 1])?;
                index += 2;
            }
            "--primary" if args.get(index + 1).is_some() => {
                decision_components = parse_primary(&args[index + 1])?;
                index += 2;
            }
            "--end-time" if args.get(index + 1).is_some() => {
                end_time_ms = args[index + 1].parse().ok();
                index += 2;
            }
            "--no-cache" => {
                cache_settings.enabled = false;
                index += 1;
            }
            "--clear-cache" => {
                cache_settings.clear = true;
                index += 1;
            }
            "--refresh-cache" => {
                cache_settings.refresh = true;
                index += 1;
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
    eprintln!(
        "[mrc-research] start cwd={} symbols={:?} direction={} primary={} history={} end_time_ms={:?} cache_enabled={} cache_clear={} cache_refresh={} output={:?} html={:?}",
        std::env::current_dir()?.display(),
        symbol_keys,
        direction_lock
            .map(|direction| if direction > 0 { "long" } else { "short" })
            .unwrap_or("both"),
        if decision_components.primary_stairs && decision_components.primary_outward {
            "all"
        } else if decision_components.primary_stairs {
            "stairs"
        } else if decision_components.primary_outward {
            "outward"
        } else {
            "none"
        },
        history,
        end_time_ms,
        cache_settings.enabled,
        cache_settings.clear,
        cache_settings.refresh,
        output,
        html
    );
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
        let (minute_count, minute_cache, report) = analyze_symbol(
            symbol_key,
            history,
            end_time_ms,
            direction_lock,
            decision_components,
            cache_settings,
        )?;
        print_summary(
            symbol_key,
            minute_count,
            minute_cache,
            &report,
            print_positions,
        );
        if let Some(requested) = output.as_deref() {
            let path = symbol_output_path(requested, symbol_key, "json", multiple_symbols)?;
            let document = json!({
                "symbol_key": symbol_key,
                "direction": direction_lock.map(|direction| if direction > 0 { "long" } else { "short" }).unwrap_or("both"),
                "primary": if decision_components.primary_stairs && decision_components.primary_outward { "all" } else if decision_components.primary_stairs { "stairs" } else if decision_components.primary_outward { "outward" } else { "none" },
                "decision_components": decision_components.names(),
                "history_requested_1m": history,
                "end_time_ms": end_time_ms,
                "cache": {"1m": minute_cache.to_string()},
                "history_loaded": {"1m": minute_count},
                "trade_summary": report.trade_summary,
                "trades": report.trades,
                "open_trade": report.open_trade,
                "crossing_features": {"evidence": "recent adjacent MRC transition observations", "learning_window": "recent five direction changes"},
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
                &report,
            )?;
            println!("html report            : {}", path.display());
            let absolute_path = path.canonicalize().unwrap_or(path);
            let file_url = absolute_path.to_string_lossy().replace(' ', "%20");
            println!("open html              : file://{file_url}");
        }
        eprintln!(
            "[mrc-research] symbol-finished symbol={} elapsed_ms={}",
            symbol_key,
            started.elapsed().as_millis()
        );
    }
    eprintln!(
        "[mrc-research] complete symbols={} elapsed_ms={}",
        symbol_keys.len(),
        started.elapsed().as_millis()
    );
    Ok(())
}
