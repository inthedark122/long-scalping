#[path = "../src/bin/research/data.rs"]
mod data;
#[path = "../src/bin/research/virtual_trade.rs"]
mod virtual_trade;

use data::CandleRow;
use virtual_trade::{TradeSetup, VirtualTradeSimulator};

fn candle(time_ms: i64, high: f64, low: f64, close: f64) -> CandleRow {
    CandleRow {
        time_ms,
        open: close,
        high,
        low,
        close,
        zz_dir: 0,
        zz_break_price: 0.0,
        gain: 0.0,
        buy_sell_gain: 0.0,
        ntps: 0,
        mrc: Default::default(),
    }
}

fn short_setup(signal_time_ms: i64) -> TradeSetup {
    TradeSetup::with_levels(
        -1,
        5,
        "up_inner",
        4,
        "mean",
        0.066917,
        0.065917,
        0.067688,
        signal_time_ms,
        "test",
    )
}

#[test]
fn next_candle_limit_entry_expires_when_price_does_not_reach_entry() {
    let mut simulator = VirtualTradeSimulator::default();
    simulator.queue_next_candle(short_setup(1_000));

    // This is the only candle eligible to fill the setup. Its high is below
    // the short entry, matching the CAP example.
    simulator.update(&candle(61_000, 0.06673, 0.06620, 0.06626));
    assert!(simulator.active().is_none());
    assert_eq!(simulator.completed_count(), 0);

    // The setup must not remain live and fill on a later candle.
    simulator.update(&candle(121_000, 0.06710, 0.06580, 0.06600));
    assert!(simulator.active().is_none());
    assert_eq!(simulator.completed_count(), 0);
}

#[test]
fn next_candle_limit_entry_uses_fill_candle_time_and_exits_later() {
    let mut simulator = VirtualTradeSimulator::default();
    simulator.queue_next_candle(short_setup(1_000));

    simulator.update(&candle(61_000, 0.06700, 0.06620, 0.06650));
    let active = simulator.active().expect("entry should fill");
    assert_eq!(active.opened_at, 61_000);
    assert_eq!(active.entry_price, 0.066917);

    // The fill candle is not also used to resolve the exit. The following
    // candle reaches the target and closes the position.
    simulator.update(&candle(121_000, 0.06650, 0.06580, 0.06600));
    let trade = simulator.last_completed().expect("target should be hit");
    assert_eq!(trade.opened_at, 61_000);
    assert_eq!(trade.closed_at, 121_000);
    assert_eq!(trade.exit_reason, "target");
}
