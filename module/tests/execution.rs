use std::collections::{BTreeMap, HashMap};

use long_scalping::{
    engine::run,
    model::{ExitMode, ExitType, IndicatorSnapshot, PositionState},
    state::{EngineState, PositionRuntime},
};
use lte_strategy_bridge::abi::ModuleSymbolInfo;
use lte_strategy_bridge::{
    AlgoSuggestionTradeStatus, Direction, ModuleEvent, ModuleInput, ModulePositionSummary,
    ModulePositions, SuggestionInfo, SuggestionInfoCandle, SuggestionInfoIndicator,
};

fn sug_info(time: i64, price: f64) -> SuggestionInfo {
    SuggestionInfo {
        time,
        system_time: time,
        symbol: "TEST".to_string(),
        symbol_id: 1,
        symbol_key: "OKX#TEST-USDT-SWAP#SWAP".parse().expect("symbol key"),
        status: AlgoSuggestionTradeStatus::Normal,
        price,
        candle: SuggestionInfoCandle::new(price),
        klines: HashMap::new(),
        fast_candle: None,
        direction: Direction::Both,
        indicator: SuggestionInfoIndicator::default(),
        order_book: None,
        oi: None,
    }
}

fn input(price: f64, positions: ModulePositions) -> ModuleInput {
    ModuleInput {
        event: ModuleEvent::SugInfo,
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
        positions,
        sug_info: Some(sug_info(1_000, price)),
        state: None,
    }
}

fn position(side: PositionState, stop_loss: f64) -> PositionRuntime {
    PositionRuntime {
        side,
        entry_price: 100.0,
        peak_price: 100.0,
        trough_price: 100.0,
        take_profit: 100.2,
        stop_loss,
        stop_loss_reason: "structural:1m:below:mean".to_string(),
        exit_tp_reliable: true,
        exit_mode: ExitMode::Structural,
        exit_type: ExitType::VirtualTrailingExit { ticks: 2 },
        entry_reason: None,
    }
}

#[test]
fn trailing_stop_reprices_two_tick_tp_from_latest_long_price() {
    let mut state = EngineState::default();
    state.last_snapshot = Some(IndicatorSnapshot::default());
    state.position = position(PositionState::Long, 99.5);

    let positions = ModulePositions {
        long: Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: -0.8,
            qty: 0.1,
        }),
        short: None,
    };
    let first = run(&input(99.0, positions.clone()), &mut state);
    assert_eq!(first.place_orders[0].enter_price, 99.2);
    assert_eq!(first.cancel_orders, vec!["tp-long"]);

    let second = run(&input(98.0, positions), &mut state);
    assert_eq!(second.place_orders.len(), 1);
    assert_eq!(second.place_orders[0].enter_price, 98.2);
    assert_eq!(second.place_orders[0].mark, "tp-long");
    assert!(second.place_orders[0].reduce_only);
    assert!(second.cancel_orders.is_empty());
}

#[test]
fn trailing_stop_reprices_two_tick_tp_from_latest_short_price() {
    let mut state = EngineState::default();
    state.last_snapshot = Some(IndicatorSnapshot::default());
    state.position = position(PositionState::Short, 100.5);

    let positions = ModulePositions {
        long: None,
        short: Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: -0.8,
            qty: 0.1,
        }),
    };
    let first = run(&input(101.0, positions.clone()), &mut state);
    assert_eq!(first.place_orders[0].enter_price, 100.8);
    assert_eq!(first.cancel_orders, vec!["tp-short"]);

    let second = run(&input(99.0, positions), &mut state);
    assert_eq!(second.place_orders.len(), 1);
    assert_eq!(second.place_orders[0].enter_price, 98.8);
    assert_eq!(second.place_orders[0].mark, "tp-short");
    assert!(second.place_orders[0].reduce_only);
    assert!(second.cancel_orders.is_empty());
}

#[test]
fn trail_after_virtual_sl_reprices_single_tp_from_current_price() {
    let mut state = EngineState::default();
    state.last_snapshot = Some(IndicatorSnapshot::default());
    let mut trailing = position(PositionState::Long, 90.0);
    trailing.exit_mode = ExitMode::TrailAfterVirtualSl;
    state.position = trailing;

    let output = run(
        &input(
            99.0,
            ModulePositions {
                long: Some(ModulePositionSummary {
                    entry_price: 100.0,
                    notional: 10.0,
                    pnl: -0.8,
                    qty: 0.1,
                }),
                short: None,
            },
        ),
        &mut state,
    );

    assert_eq!(output.place_orders.len(), 1);
    assert_eq!(output.place_orders[0].enter_price, 99.2);
    assert_eq!(output.place_orders[0].qty, Some(0.1));
    assert!(output.place_orders[0].reduce_only);
    assert!(output.cancel_orders.is_empty());
}
