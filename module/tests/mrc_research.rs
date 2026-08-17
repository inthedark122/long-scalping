#![allow(dead_code)]

#[path = "../src/bin/research/data.rs"]
mod data;
#[path = "../src/bin/research/mrc_crossing.rs"]
mod mrc_crossing;
#[path = "../src/bin/research/virtual_trade.rs"]
mod virtual_trade;

use data::CandleRow;
use long_scalping::model::MrcBands;
use mrc_crossing::Analyzer;
use virtual_trade::{TradeSetup, VirtualPredictionSimulator, VirtualTradeSimulator};

fn row(time_ms: i64, close: f64, high: f64, low: f64) -> CandleRow {
    CandleRow {
        time_ms,
        open: close,
        high,
        low,
        close,
        gain: 0.0,
        buy_sell_gain: 1.0,
        mrc: MrcBands {
            down_big: 90.0,
            down_outer: 92.0,
            down_small: 94.0,
            down_inner: 96.0,
            mean_line: 100.0,
            up_inner: 104.0,
            up_small: 106.0,
            up_outer: 108.0,
            up_big: 110.0,
            current_cross: 0.0,
            mean_line_slope: 0.0,
        },
    }
}

#[test]
fn research_retains_variable_length_event_history() {
    let mut analyzer = Analyzer::new();
    analyzer.observe(&row(0, 99.0, 99.0, 99.0));
    analyzer.observe(&row(60_000, 111.0, 111.0, 99.0));
    let report = analyzer.report();

    assert_eq!(report.crossing_count, 5);
    assert_eq!(report.events.len(), 5);
    assert!(report.events.iter().all(|event| event.time_ms > 0));
    assert!(report.events.iter().any(|event| event.elapsed_ms == 0));
    assert!(report
        .events
        .iter()
        .all(|event| event.mrc_trace.contains("no_pullback=")
            && event.mrc_trace.contains("revert=")
            && event.mrc_trace.contains("move=")));
    assert_eq!(report.pattern_performance.len(), 7);
    assert_eq!(report.parallel_pattern_trades.len(), 7);
}

#[test]
fn virtual_predictions_can_overlap_and_keep_fill_outcomes_separate() {
    let mut simulator = VirtualPredictionSimulator::default();
    simulator.queue(TradeSetup::with_levels(
        1,
        4,
        "mean",
        5,
        "up_inner",
        100.0,
        102.0,
        98.0,
        0,
        "long prediction",
    ));
    simulator.queue(TradeSetup::with_levels(
        -1,
        4,
        "mean",
        3,
        "down_inner",
        100.0,
        98.0,
        102.0,
        0,
        "short prediction",
    ));

    simulator.update(&row(60_000, 101.0, 103.0, 99.0));
    assert_eq!(simulator.active_count(), 2);

    simulator.update(&row(120_000, 100.0, 101.0, 97.0));
    let summary = simulator.summary();
    assert_eq!(summary.predictions, 2);
    assert_eq!(summary.filled, 2);
    assert_eq!(summary.reached_and_followed, 1);
    assert_eq!(summary.prediction_broken, 1);
}

#[test]
fn prediction_tracks_furthest_line_before_returning_to_source() {
    let mut simulator = VirtualPredictionSimulator::default();
    simulator.queue(TradeSetup::with_levels(
        1,
        4,
        "mean",
        5,
        "up_inner",
        100.0,
        102.0,
        98.0,
        0,
        "line path",
    ));

    simulator.observe_crossings(&[5, 6, 5, 4, 8]);
    simulator.finish(&row(60_000, 100.0, 100.0, 100.0));

    let result = &simulator.results()[0];
    assert_eq!(result.furthest_line_before_return, Some(6));
    assert_eq!(result.furthest_line_before_return_name, Some("up_small"));
    assert!(result.returned_to_source_line);
}

#[test]
fn core_position_keeps_original_fill_and_swapped_trade_levels() {
    let mut simulator = VirtualTradeSimulator::default();
    simulator.queue_next_candle(
        TradeSetup::with_levels(
            -1,
            4,
            "mean",
            5,
            "up_inner",
            100.0,
            98.0,
            102.0,
            0,
            "core prediction",
        )
        .with_execution(1, "market", "limit"),
    );

    simulator.update(&row(60_000, 100.0, 101.0, 99.0));
    let active = simulator.active().expect("original limit should fill");
    assert_eq!(active.side, -1);
    assert_eq!(active.target_price, 98.0);
    assert_eq!(active.stop_price, 102.0);

    simulator.update(&row(120_000, 98.0, 99.0, 97.0));
    let trades = simulator.completed();
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0].side, -1);
    assert_eq!(trades[0].exit_reason, "target");
    assert!((trades[0].gross_pnl_pct - 2.0).abs() < 1e-12);
}
