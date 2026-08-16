use std::collections::{BTreeMap, HashMap};

use long_scalping::{
    engine::run,
    model::{
        EntryReason, EntryType, ExitMode, ExitType, IndicatorSnapshot, PositionState, StrategyMode,
    },
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

#[test]
fn mrc_crossing_keeps_entry_tp_and_sl_on_realtime_updates() {
    let mut state = EngineState::default();
    let mut snapshot = IndicatorSnapshot::default();
    snapshot.tf1m.mrc = long_scalping::model::MrcBands {
        down_big: 80.0,
        down_outer: 85.0,
        down_small: 90.0,
        down_inner: 95.0,
        mean_line: 100.0,
        up_inner: 105.0,
        up_small: 110.0,
        up_outer: 115.0,
        up_big: 120.0,
        current_cross: 0.0,
        mean_line_slope: 1.0,
    };
    snapshot.tf1m.zz_buy_price = Some(95.0);
    snapshot.tf1m.zz_sell_price = Some(110.0);
    snapshot.tf1m.candle.close = 102.0;
    snapshot.tf5m = snapshot.tf1m;
    snapshot.tf15m = snapshot.tf1m;
    snapshot.tf1h = snapshot.tf1m;
    state.last_snapshot = Some(snapshot);
    state.last_active_tp_recalc_bar = Some(state.bar_1m);
    state.position = PositionRuntime {
        side: PositionState::Long,
        entry_price: 100.0,
        peak_price: 100.0,
        trough_price: 100.0,
        take_profit: 100.2,
        stop_loss: 90.0,
        stop_loss_reason: "mrc-crossing:adjacent-line".to_string(),
        exit_tp_reliable: true,
        exit_mode: ExitMode::Structural,
        exit_type: ExitType::VirtualTrailingExit { ticks: 2 },
        entry_reason: Some(long_scalping::model::EntryReason::MrcCrossing {
            price: 100.0,
            target_price: 100.2,
            stop_price: 90.0,
            probability: 0.8,
            estimated_time_ms: 1_000.0,
            source_line: 1,
            target_line: 2,
            favorable_move_pct: 3.0,
            adverse_move_pct: -1.0,
        }),
    };

    let positions = ModulePositions {
        long: Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: 0.2,
            qty: 0.1,
        }),
        short: None,
    };

    let first = run(&input(102.0, positions.clone()), &mut state);
    assert!(first.place_orders.is_empty());
    assert!(first.cancel_orders.is_empty());
    assert_eq!(state.position.take_profit, 100.2);
    assert_eq!(state.position.stop_loss, 90.0);

    let second = run(&input(101.0, positions), &mut state);
    assert!(second.place_orders.is_empty());
    assert!(second.cancel_orders.is_empty());
    assert_eq!(state.position.take_profit, 100.2);
    assert_eq!(state.position.stop_loss, 90.0);
}

#[test]
fn expired_mrc_limit_does_not_block_a_new_opposite_signal() {
    let mut state = EngineState::default();
    state.cfg.strategy_mode = StrategyMode::MrcCrossing;
    state.cfg.entry_type = EntryType::Limit;
    state.cfg.required_samples = 1;
    state.bar_1m = 2;
    state.pending_entry_bar = Some(1);
    state.position.entry_reason = Some(EntryReason::MrcCrossing {
        price: 101.0,
        target_price: 104.0,
        stop_price: 96.0,
        probability: 0.8,
        estimated_time_ms: 1_000.0,
        source_line: 4,
        target_line: 5,
        favorable_move_pct: 3.0,
        adverse_move_pct: -1.0,
    });
    state.mrc_crossing_model.pattern_len = 3;
    state.mrc_crossing_model.pattern_lines = [5, 5, 5];
    state.mrc_crossing_model.patterns[5][5][5].next[2].count = 1;

    let lines = [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0];
    let mut snapshot = IndicatorSnapshot::default();
    snapshot.tf1m.mrc = long_scalping::model::MrcBands {
        down_big: lines[0],
        down_outer: lines[1],
        down_small: lines[2],
        down_inner: lines[3],
        mean_line: lines[4],
        up_inner: lines[5],
        up_small: lines[6],
        up_outer: lines[7],
        up_big: lines[8],
        current_cross: 0.0,
        mean_line_slope: 1.0,
    };
    snapshot.tf1m.candle.close = 99.0;
    snapshot.tf5m = snapshot.tf1m;
    snapshot.tf15m = snapshot.tf1m;
    snapshot.tf1h = snapshot.tf1m;
    state.last_snapshot = Some(snapshot);
    state.mrc_crossing.last_price = Some(105.0);
    for (index, line) in state.vlines.iter_mut().enumerate() {
        line.was_above = Some(index == 5);
    }

    let output = run(
        &input(
            99.0,
            ModulePositions {
                long: Some(ModulePositionSummary {
                    entry_price: 99.98,
                    notional: 0.0,
                    pnl: 0.0,
                    qty: 0.0,
                }),
                short: None,
            },
        ),
        &mut state,
    );
    assert_eq!(output.close_positions.len(), 1);
    assert_eq!(output.close_positions[0].direction, Direction::Long);
    assert_eq!(output.open_positions.len(), 1);
    assert_eq!(output.open_positions[0].direction, Direction::Short);
    assert_eq!(state.pending_entry_bar, Some(2));
    assert!(matches!(
        state.position.entry_reason,
        Some(EntryReason::MrcCrossing {
            target_line: 2,
            source_line: 5,
            ..
        })
    ));
}
