use long_scalping::market_data::{kline_requirements, MrcCalculation, MRC_PERIOD, TF_1M, TF_5M};
use lte_strategy_bridge::abi::{ModuleKline, ModuleKlineSnapshot};
use lte_strategy_bridge::kline_access::KlineHistory;
use ta::indicators::MeanReversionChannel;
use ta::{Close, High, Low, Next};

struct TestCandle {
    high: f64,
    low: f64,
    close: f64,
}

impl High for TestCandle {
    fn high(&self) -> f64 {
        self.high
    }
}

impl Low for TestCandle {
    fn low(&self) -> f64 {
        self.low
    }
}

impl Close for TestCandle {
    fn close(&self) -> f64 {
        self.close
    }
}

fn candle(index: usize, timeframe_ms: i64) -> ModuleKline {
    let base = 10.0 + index as f64 * 0.002 + (index % 13) as f64 * 0.0007;
    ModuleKline {
        time: index as i64 * timeframe_ms,
        open: base,
        high: base + 0.03 + (index % 5) as f64 * 0.001,
        low: base - 0.025 - (index % 7) as f64 * 0.001,
        close: base + (index % 3) as f64 * 0.002,
        qty_buy: 1.0,
        qty_sell: 1.0,
        asset_buy: 1.0,
        asset_sell: 1.0,
    }
}

fn expected_lines(rows: &[ModuleKline]) -> [f64; 9] {
    let mut channel = MeanReversionChannel::new(MRC_PERIOD);
    let mut output = None;
    for row in rows {
        output = Some(channel.next(&TestCandle {
            high: row.high,
            low: row.low,
            close: row.close,
        }));
    }
    let output = output.expect("test history is non-empty");
    [
        output.down_big,
        output.down_outer,
        output.down_small,
        output.down_inner,
        output.middle,
        output.up_inner,
        output.up_small,
        output.up_outer,
        output.up_big,
    ]
}

fn assert_lines_close(actual: [f64; 9], expected: [f64; 9]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
    }
}

#[test]
fn kline_input_diagnostics_counts_zero_buy_and_sell_quantities() {
    let mut first = candle(0, i64::from(TF_1M) * 1_000);
    first.qty_buy = 0.0;
    first.qty_sell = 0.0;
    let mut second = candle(1, i64::from(TF_1M) * 1_000);
    second.qty_buy = 2.0;
    second.qty_sell = 0.0;
    let mut five_minute = candle(0, i64::from(TF_5M) * 1_000);
    five_minute.qty_buy = 0.0;
    five_minute.qty_sell = 3.0;
    let klines =
        ModuleKlineSnapshot::from([(TF_1M, vec![first, second]), (TF_5M, vec![five_minute])]);

    let diagnostics = long_scalping::market_data::render_kline_input_diagnostics(&klines);

    assert_eq!(
        diagnostics,
        "input-klines=[60:rows=2 qty_buy_zero=1 qty_sell_zero=2 qty_buy_invalid=0 qty_sell_invalid=0 300:rows=1 qty_buy_zero=1 qty_sell_zero=0 qty_buy_invalid=0 qty_sell_invalid=0]"
    );
}

#[test]
fn mrc_uses_the_same_ta_rs_filter_across_kline_updates_and_revisions() {
    let mut one_minute = (0..1_400)
        .map(|index| candle(index, i64::from(TF_1M) * 1_000))
        .collect::<Vec<_>>();
    let five_minute = (0..MRC_PERIOD)
        .map(|index| candle(index, i64::from(TF_5M) * 1_000))
        .collect::<Vec<_>>();
    let init = ModuleKlineSnapshot::from([
        (TF_1M, one_minute[..MRC_PERIOD].to_vec()),
        (TF_5M, five_minute),
    ]);
    let mut history = KlineHistory::new(&kline_requirements());
    history.init(&init);
    let mut calculation = MrcCalculation::default();
    calculation.initialize(&history).expect("initialize MRC");

    for row in &one_minute[MRC_PERIOD..] {
        let update = ModuleKlineSnapshot::from([(TF_1M, vec![row.clone()])]);
        history.update(&update);
        calculation.update(&update, &history).expect("advance MRC");
    }

    assert_lines_close(
        calculation
            .latest_bands(TF_1M)
            .expect("latest MRC bands")
            .lines(),
        expected_lines(&one_minute),
    );

    let revised_index = one_minute.len() - 50;
    let mut revised = one_minute[revised_index].clone();
    revised.high += 0.2;
    revised.close += 0.1;
    one_minute[revised_index] = revised.clone();
    let update = ModuleKlineSnapshot::from([(TF_1M, vec![revised])]);
    history.update(&update);
    calculation
        .update(&update, &history)
        .expect("recalculate revised MRC history");

    assert_lines_close(
        calculation
            .latest_bands(TF_1M)
            .expect("latest revised MRC bands")
            .lines(),
        expected_lines(&one_minute),
    );
}

#[test]
fn mrc_calculation_is_runtime_only_and_not_in_saved_module_state() {
    let state = long_scalping::state::EngineState::default();
    let saved = long_scalping::state::save(&state).expect("state should serialize");
    assert!(saved.get("mrc_calculation").is_none());
    assert!(state.mrc_calculation.latest_bands(TF_1M).is_none());
    assert!(state.mrc_calculation.latest_bands(TF_5M).is_none());
}
