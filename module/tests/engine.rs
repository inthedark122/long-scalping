use std::collections::BTreeMap;

use long_scalping::{
    engine::run,
    model::{ExitMode, ExitType, IndicatorSnapshot, PositionState},
    state::{EngineState, PositionRuntime},
};
use lte_strategy_bridge::abi::{ModuleKline, ModuleKlineSnapshot, ModuleSymbolInfo};
use lte_strategy_bridge::{
    ModuleEvent, ModuleInput, ModuleMarketCandle, ModuleMarketDecision, ModulePositionSummary,
    ModulePositions,
};

fn market_decision(time: i64, price: f64) -> ModuleMarketDecision {
    ModuleMarketDecision {
        time,
        system_time: time,
        symbol_key: "OKX#TEST-USDT-SWAP#SWAP".to_string(),
        price,
        candle: ModuleMarketCandle {
            min: price,
            max: price,
            enter: price,
            exit: price,
            ..Default::default()
        },
        fc: None,
        ntp_5s: 0,
        tdt: None,
        order_book: None,
        oi: None,
    }
}

fn input(price: f64, positions: ModulePositions) -> ModuleInput {
    ModuleInput {
        event: ModuleEvent::MarketDecision,
        price,
        symbol: "TEST".to_string(),
        max_amount: 100.0,
        symbol_info: ModuleSymbolInfo {
            price_tick_size: 0.1,
            ..Default::default()
        },
        auto_max_amount: 100.0,
        leverage: 10,
        indicators: BTreeMap::new(),
        internal_indicators: BTreeMap::new(),
        klines: BTreeMap::new(),
        positions,
        market_decision: Some(market_decision(1_000, price)),
        state: None,
    }
}

fn startup_kline(index: usize, timeframe_seconds: i32, first_time_ms: i64) -> ModuleKline {
    let base = 100.0 + index as f64 * 0.0003 + (index % 17) as f64 * 0.001;
    ModuleKline {
        time: first_time_ms + index as i64 * i64::from(timeframe_seconds) * 1_000,
        open: base,
        high: base + 0.04,
        low: base - 0.035,
        close: base + (index % 3) as f64 * 0.002,
        qty_buy: 1.2,
        qty_sell: 0.8,
        asset_buy: 1.2,
        asset_sell: 0.8,
    }
}

fn init_input(klines: ModuleKlineSnapshot) -> ModuleInput {
    ModuleInput {
        event: ModuleEvent::Init,
        price: 100.0,
        symbol: "TEST".to_string(),
        max_amount: 100.0,
        symbol_info: ModuleSymbolInfo::default(),
        auto_max_amount: 100.0,
        leverage: 10,
        indicators: BTreeMap::new(),
        internal_indicators: BTreeMap::new(),
        klines,
        positions: ModulePositions {
            long: None,
            short: None,
        },
        market_decision: None,
        state: None,
    }
}

#[test]
fn init_loads_requested_history_replays_eight_hours_and_emits_no_orders() {
    use long_scalping::market_data::{
        MARKET_REPLAY_BARS, MRC_WARMUP_BARS, TF_1M, TF_1M_HISTORY_COUNT, TF_5M, TF_5M_HISTORY_COUNT,
    };

    let mut state = EngineState::default();
    state.ticks = 9_999;

    let requirements = run(
        &ModuleInput {
            event: ModuleEvent::Requirements,
            price: 0.0,
            symbol: "TEST".to_string(),
            max_amount: 0.0,
            symbol_info: ModuleSymbolInfo::default(),
            auto_max_amount: 0.0,
            leverage: 0,
            indicators: BTreeMap::new(),
            internal_indicators: BTreeMap::new(),
            klines: BTreeMap::new(),
            positions: ModulePositions {
                long: None,
                short: None,
            },
            market_decision: None,
            state: None,
        },
        &mut state,
    );
    assert_eq!(
        requirements.data_requirements.klines,
        BTreeMap::from([(TF_1M, TF_1M_HISTORY_COUNT), (TF_5M, TF_5M_HISTORY_COUNT),])
    );

    let latest_close_ms = 1_700_000_000_000_i64;
    let one_minute_count = TF_1M_HISTORY_COUNT as usize;
    let five_minute_count = TF_5M_HISTORY_COUNT as usize;
    let klines = ModuleKlineSnapshot::from([
        (
            TF_1M,
            (0..one_minute_count)
                .map(|index| {
                    startup_kline(
                        index,
                        TF_1M,
                        latest_close_ms - one_minute_count as i64 * i64::from(TF_1M) * 1_000,
                    )
                })
                .collect(),
        ),
        (
            TF_5M,
            (0..five_minute_count)
                .map(|index| {
                    startup_kline(
                        index,
                        TF_5M,
                        latest_close_ms - five_minute_count as i64 * i64::from(TF_5M) * 1_000,
                    )
                })
                .collect(),
        ),
    ]);
    let output = run(&init_input(klines), &mut state);

    assert!(
        output.debug.contains(&format!(
            "market-replay={MARKET_REPLAY_BARS}/{MARKET_REPLAY_BARS} warmup={MRC_WARMUP_BARS}"
        )),
        "{}",
        output.debug
    );
    assert!(state.last_error.is_none(), "{:?}", state.last_error);
    assert!(state.last_snapshot.is_some());
    assert!(state.mrc_crossing_model.seeded_from_history);
    assert_eq!(state.ticks, MARKET_REPLAY_BARS as u64);
    assert!(output.open_positions.is_empty());
    assert!(output.close_positions.is_empty());
    assert!(output.place_orders.is_empty());
    assert!(output.cancel_orders.is_empty());
}

#[test]
fn next_market_decision_reprices_trailing_tp_from_current_price() {
    let mut state = EngineState::default();
    state.last_snapshot = Some(IndicatorSnapshot::default());
    state.position = PositionRuntime {
        side: PositionState::Long,
        entry_price: 100.0,
        peak_price: 100.0,
        trough_price: 100.0,
        take_profit: 100.2,
        stop_loss: 99.5,
        stop_loss_reason: "fixed".to_string(),
        exit_tp_reliable: true,
        exit_mode: ExitMode::Fixed,
        exit_type: ExitType::VirtualTrailingExit { ticks: 2 },
        entry_reason: None,
    };

    let positions = ModulePositions {
        long: Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: -0.8,
            qty: 0.1,
        }),
        short: None,
    };
    let mut first = input(99.0, positions);

    let first_output = run(&first, &mut state);
    assert_eq!(first_output.place_orders.len(), 1);
    assert_eq!(first_output.place_orders[0].enter_price, 99.2);
    assert_eq!(
        first_output.cancel_orders,
        vec!["tp-long-far", "tp-long-1", "tp-long-2", "tp-long-3"]
    );

    first.price = 98.0;
    first.market_decision = Some(market_decision(2_000, first.price));

    let next_output = run(&first, &mut state);
    assert_eq!(next_output.place_orders.len(), 1);
    assert_eq!(next_output.place_orders[0].enter_price, 98.2);
    assert_eq!(next_output.place_orders[0].mark, "tp-long");
    assert!(next_output.place_orders[0].reduce_only);
    assert!(next_output.cancel_orders.is_empty());
}
