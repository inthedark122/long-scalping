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

#[test]
fn next_sug_info_reprices_trailing_tp_from_current_price() {
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
    assert_eq!(first_output.cancel_orders, vec!["tp-long"]);

    first.price = 98.0;
    first.sug_info = Some(sug_info(2_000, first.price));

    let next_output = run(&first, &mut state);
    assert_eq!(next_output.place_orders.len(), 1);
    assert_eq!(next_output.place_orders[0].enter_price, 98.2);
    assert_eq!(next_output.place_orders[0].mark, "tp-long");
    assert!(next_output.place_orders[0].reduce_only);
    assert!(next_output.cancel_orders.is_empty());
}
