use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

#[path = "research/data.rs"]
mod data;
#[path = "research/window_prediction.rs"]
mod window_prediction;
#[path = "research/window_prediction_html.rs"]
mod window_prediction_html;

use data::{compute_mrc, fetch_history_for_timeframe_cached, CacheSettings, CacheStatus};
use window_prediction::{analyze, ResearchReport, MRC_WARMUP_BARS};

const DEFAULT_SYMBOL_KEY: &str = "OKX#APR-USDT-SWAP#SWAP";
const TIMEFRAME_SECONDS: u64 = 300;
const LEARNING_HISTORY_BARS: usize = 100;

#[derive(Debug)]
struct Args {
    symbol_key: String,
    history: usize,
    analysis_offset_minutes: usize,
    output: String,
    cache: CacheSettings,
    watch_seconds: Option<u64>,
}

fn usage() {
    eprintln!(
        "mrc_window_research [--symbol-key KEY] [--history N] [--analysis-offset-minutes N] [--output FILE|DIR] [--no-cache] [--clear-cache] [--refresh-cache] [--watch-seconds N]"
    );
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let values = std::env::args().skip(1).collect::<Vec<_>>();
    let mut args = Args {
        symbol_key: DEFAULT_SYMBOL_KEY.to_string(),
        history: 100,
        analysis_offset_minutes: 60,
        output: "tmp/reports/".to_string(),
        cache: CacheSettings::default(),
        watch_seconds: None,
    };
    let mut index = 0;
    while index < values.len() {
        match values[index].as_str() {
            "--symbol-key" if values.get(index + 1).is_some() => {
                args.symbol_key = values[index + 1].clone();
                index += 2;
            }
            "--history" if values.get(index + 1).is_some() => {
                args.history = values[index + 1].parse()?;
                index += 2;
            }
            "--analysis-offset-minutes" if values.get(index + 1).is_some() => {
                args.analysis_offset_minutes = values[index + 1].parse()?;
                index += 2;
            }
            "--output" if values.get(index + 1).is_some() => {
                args.output = values[index + 1].clone();
                index += 2;
            }
            "--no-cache" => {
                args.cache.enabled = false;
                index += 1;
            }
            "--clear-cache" => {
                args.cache.clear = true;
                index += 1;
            }
            "--refresh-cache" => {
                args.cache.refresh = true;
                index += 1;
            }
            "--watch-seconds" if values.get(index + 1).is_some() => {
                args.watch_seconds = Some(values[index + 1].parse::<u64>()?.max(5));
                index += 2;
            }
            "--help" | "-h" => {
                usage();
                std::process::exit(0);
            }
            option => return Err(format!("unknown or incomplete option: {option}").into()),
        }
    }
    if args.history < 25 {
        return Err("--history must be at least 25 candles for analog learning".into());
    }
    if args.analysis_offset_minutes % (TIMEFRAME_SECONDS as usize / 60) != 0 {
        return Err("--analysis-offset-minutes must be a multiple of five".into());
    }
    let offset_bars = args.analysis_offset_minutes * 60 / TIMEFRAME_SECONDS as usize;
    if offset_bars >= args.history {
        return Err("analysis offset must be smaller than the visible history".into());
    }
    Ok(args)
}

fn symbol_slug(symbol_key: &str) -> String {
    symbol_key
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_ascii_lowercase()
}

fn output_paths(requested: &str, symbol_key: &str) -> Result<(PathBuf, PathBuf), Box<dyn Error>> {
    let requested_path = Path::new(requested);
    let directory_style = requested_path.is_dir()
        || requested.ends_with('/')
        || requested.ends_with('\\')
        || requested_path.extension().is_none();
    if directory_style {
        fs::create_dir_all(requested_path)?;
        let stem = format!("mrc-window-{}", symbol_slug(symbol_key));
        return Ok((
            requested_path.join(format!("{stem}.html")),
            requested_path.join(format!("{stem}.json")),
        ));
    }
    let mut json_path = requested_path.to_path_buf();
    json_path.set_extension("json");
    Ok((requested_path.to_path_buf(), json_path))
}

fn number(value: Option<f64>, digits: usize, suffix: &str) -> String {
    value
        .map(|value| format!("{value:.digits$}{suffix}"))
        .unwrap_or_else(|| "n/a".to_string())
}

fn duration_label(minutes: usize) -> String {
    if minutes % 60 == 0 {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    }
}

fn print_summary(report: &ResearchReport, cache: CacheStatus, html: &Path, json: &Path) {
    println!("MRC WINDOW CASCADE PREDICTION RESEARCH");
    println!("symbol                 : {}", report.symbol_key);
    println!(
        "timeframe              : {}m",
        report.timeframe_seconds / 60
    );
    println!(
        "history                : requested={} loaded={} learning_base={} MRC-warmup={}",
        report.history_requested,
        report.history_loaded,
        report.training_history_loaded,
        report.mrc_warmup_bars,
    );
    println!("cache                  : {cache}");
    println!(
        "analysis as-of         : {} before latest · realized future bars={}",
        duration_label(report.analysis_offset_minutes),
        report.realized_future_bars
    );
    println!("forecast timing        : per-candle cascade remaining-duration estimate");
    println!("forecasts              : {}", report.validation.forecasts);
    println!(
        "walk-forward checks    : {} resolved direction_accuracy={} mean_abs_error={}",
        report.validation.resolved_forecasts,
        number(report.validation.direction_accuracy_pct, 1, "%"),
        number(report.validation.mean_absolute_error_pct, 4, "%")
    );
    if let Some(forecast) = &report.latest_forecast {
        println!(
            "latest forecast        : close={:.6} predicted={:.6} return={:+.3}% duration={} bars ({}m) target_ms={} range={:.6}..{:.6} confidence={:.1}% training_samples={}",
            forecast.close,
            forecast.predicted_price,
            forecast.predicted_return_pct,
            forecast.duration_bars,
            forecast.duration_minutes,
            forecast.target_time_ms,
            forecast.lower_price,
            forecast.upper_price,
            forecast.direction_confidence_pct,
            forecast.training_samples,
        );
    } else {
        println!("latest forecast        : warming up; more resolved analogs are needed");
    }
    println!("entries                : not simulated");
    println!("completed positions    : not simulated");
    println!("trade PnL / drawdown   : not simulated");
    println!("html report            : {}", html.display());
    println!("json report            : {}", json.display());
}

fn generate_report(
    args: &Args,
    cache_settings: CacheSettings,
) -> Result<(ResearchReport, CacheStatus), Box<dyn Error>> {
    let fetch_limit = args
        .history
        .saturating_add(MRC_WARMUP_BARS)
        .saturating_add(LEARNING_HISTORY_BARS);
    let (mut rows, mut cache_status) = fetch_history_for_timeframe_cached(
        &args.symbol_key,
        TIMEFRAME_SECONDS,
        fetch_limit,
        None,
        cache_settings,
    )?;
    if rows.len() < fetch_limit && cache_settings.enabled && !cache_settings.refresh {
        let mut refresh_settings = cache_settings;
        refresh_settings.refresh = true;
        (rows, cache_status) = fetch_history_for_timeframe_cached(
            &args.symbol_key,
            TIMEFRAME_SECONDS,
            fetch_limit,
            None,
            refresh_settings,
        )?;
    }
    compute_mrc(&mut rows);
    let report = analyze(
        &args.symbol_key,
        TIMEFRAME_SECONDS,
        args.history,
        args.analysis_offset_minutes,
        &rows,
    );
    Ok((report, cache_status))
}

fn write_outputs(
    args: &Args,
    report: &ResearchReport,
) -> Result<(PathBuf, PathBuf), Box<dyn Error>> {
    let (html_path, json_path) = output_paths(&args.output, &args.symbol_key)?;
    if let Some(parent) = json_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&json_path, serde_json::to_vec_pretty(report)?)?;
    window_prediction_html::write_report(&html_path, report)?;
    Ok((html_path, json_path))
}

fn run_once(args: &Args, cache_settings: CacheSettings) -> Result<i64, Box<dyn Error>> {
    let (report, cache_status) = generate_report(args, cache_settings)?;
    let (html, json) = write_outputs(args, &report)?;
    print_summary(&report, cache_status, &html, &json);
    Ok(report.source_end_ms.unwrap_or_default())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let Some(interval) = args.watch_seconds else {
        run_once(&args, args.cache)?;
        return Ok(());
    };

    let mut settings = args.cache;
    let mut last_time = run_once(&args, settings)?;
    loop {
        thread::sleep(Duration::from_secs(interval));
        settings.clear = false;
        settings.refresh = true;
        let (report, cache_status) = generate_report(&args, settings)?;
        let current_time = report.source_end_ms.unwrap_or_default();
        if current_time <= last_time {
            println!("waiting for next closed 5m candle; latest={last_time}");
            continue;
        }
        let (html, json) = write_outputs(&args, &report)?;
        print_summary(&report, cache_status, &html, &json);
        last_time = current_time;
    }
}
