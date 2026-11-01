use std::collections::{BTreeMap, HashMap};

use long_scalping::{
    detection::cascade::{CascadeSignal, MovementPhase},
    detection::mrc_crossing,
    detection::mrc_prediction::MrcPredictionContext,
    engine::run,
    model::{EntryReason, EntryType, ExitMode, ExitType, IndicatorSnapshot, PositionState},
    state::{
        EngineState, MrcCrossingModel, MrcHistoryEvent, MrcMovementSnapshot, PositionContext,
        PositionExitPlanSource, PositionLifetime, PositionPlanSnapshot, PositionRuntime,
        TakeProfitAllocation,
    },
};
use lte_strategy_bridge::abi::ModuleSymbolInfo;
use lte_strategy_bridge::{
    Direction, ModuleEvent, ModuleInput, ModuleMarketCandle, ModuleMarketDecision,
    ModulePositionSummary, ModulePositions,
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

#[test]
fn active_position_without_entry_reason_reports_exit_plan_diagnostic() {
    let mut state = EngineState::default();
    state.last_snapshot = Some(IndicatorSnapshot::default());

    let output = run(
        &input(
            100.0,
            ModulePositions {
                long: Some(ModulePositionSummary {
                    entry_price: 100.0,
                    notional: 10.0,
                    pnl: 0.0,
                    qty: 0.1,
                }),
                short: None,
            },
        ),
        &mut state,
    );

    assert!(output
        .debug
        .contains("active-exit=emergency-close side=Long reason=missing-entry-reason"));
    assert_eq!(output.close_positions.len(), 1);
    assert!(output.close_positions[0]
        .reason
        .contains("position-controller-emergency-no-exit-plan"));
    assert!(output
        .debug
        .contains("position-sync=runtime-rebuilt side=Long"));
}

fn position(side: PositionState, stop_loss: f64) -> PositionRuntime {
    PositionRuntime {
        side,
        entry_price: 100.0,
        peak_price: 100.0,
        trough_price: 100.0,
        take_profit: 100.2,
        stop_loss,
        stop_loss_reason: "fixed".to_string(),
        exit_tp_reliable: true,
        exit_mode: ExitMode::Fixed,
        exit_type: ExitType::VirtualTrailingExit { ticks: 2 },
        entry_reason: None,
    }
}

fn mrc_reason(target_price: f64, stop_price: f64) -> EntryReason {
    EntryReason::MrcCrossing {
        price: 100.0,
        market_price: 100.0,
        mrc_min: 90.0,
        mrc_max: 110.0,
        target_price,
        stop_price,
        source_line: 4,
        target_line: 5,
        target_is_movement: false,
        pattern_kind: "STAIRS_UP".to_string(),
        pattern: vec![4, 5],
        pattern_elapsed_ms: vec![0, 12_000],
        pattern_samples: 1,
        context_samples: 0,
        no_pullback_count: 0,
        reverted_line: None,
        target_samples: 1,
        movement_samples: 1,
        next_line_crosses: 1,
        support_resistance_moves: 0,
        price_only_moves: 0,
        successful_moves: 1,
        failed_moves: 0,
        movement_success_rate: 1.0,
        expected_value_pct: 3.0,
        expected_price_low: 100.0,
        expected_price_high: target_price,
        estimated_time_ms: 12_000.0,
        favorable_move_pct: 3.0,
        adverse_move_pct: -1.0,
    }
}

fn staircase_snapshot() -> IndicatorSnapshot {
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
    snapshot.tf5m = snapshot.tf1m;
    snapshot
}

fn active_mrc_route_fixture() -> (
    EntryReason,
    long_scalping::detection::mrc_cross_analysis::MrcCrossAnalysis,
    MrcPredictionContext,
) {
    let lines = [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0];
    let movement = MrcMovementSnapshot {
        start_line: 5,
        start_price: 104.0,
        market_start_price: 104.0,
        target_price: 106.0,
        target_low: 106.0,
        target_high: 106.0,
        mrc_low: 90.0,
        mrc_high: 110.0,
        high: 106.0,
        low: 103.0,
        extreme: 106.0,
        direction: 1,
        duration_ms: 2_000,
    };
    let mut model = MrcCrossingModel::default();
    for sample in 0..3u64 {
        let base = sample * 10_000;
        for (offset, line) in [3, 4, 5, 6].into_iter().enumerate() {
            model.history.push(MrcHistoryEvent {
                line,
                direction: 1,
                now_ms: base + offset as u64 * 1_000,
                market_price: lines[line as usize],
                mrc_low: lines[0],
                mrc_high: lines[8],
                no_pullback_count: 0,
                reverted_line: None,
                completed_movement: (line == 6).then_some(movement),
            });
        }
    }
    let base = 30_000;
    model
        .history
        .extend(
            [3, 4, 3, 4]
                .into_iter()
                .enumerate()
                .map(|(offset, line)| MrcHistoryEvent {
                    line,
                    direction: if offset % 2 == 0 { 1 } else { -1 },
                    now_ms: base + offset as u64 * 1_000,
                    market_price: lines[line as usize],
                    mrc_low: lines[0],
                    mrc_high: lines[8],
                    no_pullback_count: 0,
                    reverted_line: None,
                    completed_movement: None,
                }),
        );
    model.history.push(MrcHistoryEvent {
        line: 5,
        direction: 1,
        now_ms: 40_000,
        market_price: lines[5],
        mrc_low: lines[0],
        mrc_high: lines[8],
        no_pullback_count: 0,
        reverted_line: None,
        completed_movement: None,
    });
    model.last_line = Some(5);
    model.last_direction = 1;
    let observation = mrc_crossing::observe_with_forecast(
        &mut model,
        &[],
        lines,
        &Default::default(),
        (0.0, 0.0, 0.0, 0.0),
        107.0,
        50_000,
    );
    let reason = observation.entry.expect("fixture should produce an entry");
    let mut analysis = observation
        .forecast
        .expect("fixture should produce a forecast");
    analysis.line_stats[7] = long_scalping::state::MrcForecastStats {
        count: 8,
        movement_samples: 8,
        duration_sum_ms: 8 * 240_000,
        favorable_pct_sum: 36.0,
        adverse_pct_sum: 8.0,
        ..Default::default()
    };
    let context = MrcPredictionContext {
        cascade_enabled: true,
        cascade_direction: 1,
        cascade_speed_pct_per_min: Some(1.0),
        cascade_phase: MovementPhase::Building,
        cascade_potential_move_pct: Some(5.0),
        cascade_adverse_move_pct: Some(0.2),
        ..Default::default()
    };
    (reason, analysis, context)
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
    assert_eq!(
        first.cancel_orders,
        vec!["tp-long-far", "tp-long-1", "tp-long-2", "tp-long-3"]
    );

    let second = run(&input(98.0, positions), &mut state);
    assert_eq!(second.place_orders.len(), 1);
    assert_eq!(second.place_orders[0].enter_price, 98.2);
    assert_eq!(second.place_orders[0].mark, "tp-long");
    assert!(second.place_orders[0].reduce_only);
    assert!(second.cancel_orders.is_empty());
}

#[test]
fn mrc_entry_reason_exits_are_fixed_without_structural_repricing() {
    let mut state = EngineState::default();
    state.last_snapshot = Some(IndicatorSnapshot::default());
    state.position = position(PositionState::Long, 0.0);
    state.position.take_profit = 0.0;
    state.position.entry_reason = Some(mrc_reason(106.0, 97.0));

    let output = run(
        &input(
            100.0,
            ModulePositions {
                long: Some(ModulePositionSummary {
                    entry_price: 100.0,
                    notional: 10.0,
                    pnl: 0.0,
                    qty: 0.1,
                }),
                short: None,
            },
        ),
        &mut state,
    );

    assert_eq!(output.place_orders.len(), 1);
    assert_eq!(output.place_orders[0].enter_price, 106.0);
    assert_eq!(state.position.take_profit, 106.0);
    assert_eq!(state.position.stop_loss, 97.0);
    assert_eq!(state.position.exit_mode, ExitMode::Fixed);

    let second = run(
        &input(
            103.0,
            ModulePositions {
                long: Some(ModulePositionSummary {
                    entry_price: 100.0,
                    notional: 10.0,
                    pnl: 0.3,
                    qty: 0.1,
                }),
                short: None,
            },
        ),
        &mut state,
    );
    assert!(second.place_orders.is_empty());
    assert_eq!(state.position.take_profit, 106.0);
    assert_eq!(state.position.stop_loss, 97.0);
}

#[test]
fn new_position_uses_mrc_entry_reason_exits() {
    let mut state = EngineState::default();
    state.last_snapshot = Some(IndicatorSnapshot::default());
    state.position.entry_reason = Some(mrc_reason(106.0, 97.0));

    let mut new_position = input(
        100.0,
        ModulePositions {
            long: Some(ModulePositionSummary {
                entry_price: 100.0,
                notional: 10.0,
                pnl: 0.0,
                qty: 0.1,
            }),
            short: None,
        },
    );
    new_position.event = ModuleEvent::NewPosition {
        direction: Direction::Long,
        entry_price: 100.0,
        qty: 0.1,
        take_profit: None,
        stop_loss: None,
    };

    let output = run(&new_position, &mut state);

    assert_eq!(output.place_orders.len(), 1);
    assert_eq!(output.place_orders[0].enter_price, 106.0);
    assert_eq!(state.position.take_profit, 106.0);
    assert_eq!(state.position.stop_loss, 97.0);
    assert_eq!(state.position.exit_mode, ExitMode::Fixed);
}

#[test]
fn staircase_position_uses_primary_and_far_structural_take_profits() {
    let mut state = EngineState::default();
    state.last_snapshot = Some(staircase_snapshot());
    state.cascade_signal = CascadeSignal {
        dir: -1,
        phase: MovementPhase::Reversal,
        speed_pct_per_min: None,
        potential_move_pct: None,
        ..Default::default()
    };
    state.staircase_progress.direction = 1;
    state.staircase_progress.extreme_line = Some(6);
    state.staircase_progress.favorable_extreme_price = 104.0;
    state.staircase_progress.favorable_extreme_time_ms = 1_000;
    state.position.entry_reason = Some(mrc_reason(106.0, 97.0));

    let mut new_position = input(
        100.0,
        ModulePositions {
            long: Some(ModulePositionSummary {
                entry_price: 100.0,
                notional: 10.0,
                pnl: 0.0,
                qty: 0.1,
            }),
            short: None,
        },
    );
    new_position.event = ModuleEvent::NewPosition {
        direction: Direction::Long,
        entry_price: 100.0,
        qty: 0.1,
        take_profit: None,
        stop_loss: None,
    };

    let output = run(&new_position, &mut state);

    assert_eq!(output.place_orders.len(), 2);
    assert_eq!(
        output
            .place_orders
            .iter()
            .map(|order| order.mark.as_str())
            .collect::<Vec<_>>(),
        vec!["tp-long", "tp-long-far"]
    );
    assert!(output.place_orders.iter().all(|order| order.reduce_only));
    assert_eq!(
        output
            .place_orders
            .iter()
            .map(|order| order.qty.unwrap())
            .collect::<Vec<_>>(),
        vec![0.07, 0.03]
    );
    assert_eq!(output.place_orders[0].enter_price, 105.0);
    assert_eq!(output.place_orders[1].enter_price, 110.0);
    assert_eq!(
        state.exit_progress.mode,
        long_scalping::state::ExitProgressMode::Continuation
    );
}

#[test]
fn cascade_values_do_not_switch_or_reprice_the_tp_structure() {
    let mut state = EngineState::default();
    state.last_snapshot = Some(staircase_snapshot());
    state.cascade_signal = CascadeSignal {
        dir: 1,
        phase: MovementPhase::Building,
        speed_pct_per_min: Some(0.8),
        potential_move_pct: Some(5.0),
        ..Default::default()
    };
    state.staircase_progress.direction = 1;
    state.staircase_progress.extreme_line = Some(6);
    state.staircase_progress.favorable_extreme_price = 104.0;
    state.staircase_progress.favorable_extreme_time_ms = 1_000;
    state.position.entry_reason = Some(mrc_reason(106.0, 97.0));

    let mut new_position = input(
        100.0,
        ModulePositions {
            long: Some(ModulePositionSummary {
                entry_price: 100.0,
                notional: 100.0,
                pnl: 0.0,
                qty: 1.0,
            }),
            short: None,
        },
    );
    new_position.event = ModuleEvent::NewPosition {
        direction: Direction::Long,
        entry_price: 100.0,
        qty: 1.0,
        take_profit: None,
        stop_loss: None,
    };

    let initial = run(&new_position, &mut state);
    assert_eq!(initial.place_orders.len(), 2);

    let mut next = input(
        103.0,
        ModulePositions {
            long: Some(ModulePositionSummary {
                entry_price: 100.0,
                notional: 10.0,
                pnl: 0.3,
                qty: 0.1,
            }),
            short: None,
        },
    );
    next.market_decision.as_mut().unwrap().time = 5_000;
    state.cascade_signal = CascadeSignal::default();
    let output = run(&next, &mut state);
    assert!(output.place_orders.is_empty());
    assert!(output.cancel_orders.is_empty());
}

#[test]
fn stalled_staircase_moves_only_the_far_partial_closer() {
    let mut state = EngineState::default();
    state.last_snapshot = Some(staircase_snapshot());
    state.position = PositionRuntime {
        side: PositionState::Long,
        entry_price: 100.0,
        peak_price: 104.0,
        trough_price: 100.0,
        take_profit: 106.0,
        stop_loss: 97.0,
        stop_loss_reason: "fixed".to_string(),
        exit_tp_reliable: true,
        exit_mode: ExitMode::Fixed,
        exit_type: ExitType::LevelExit,
        entry_reason: Some(mrc_reason(106.0, 97.0)),
    };
    state.exit_progress.begin(PositionState::Long, 1_000, 100.0);
    state.exit_progress.mode = long_scalping::state::ExitProgressMode::Continuation;
    state.exit_progress.last_extreme_price = 104.0;
    state.exit_progress.last_extreme_update_ms = 1_000;
    state.exit_progress.far_target_price = 110.0;
    state.take_profit_allocation = long_scalping::state::TakeProfitAllocation {
        side: PositionState::Long,
        total_qty: 0.1,
        planned_qty: [0.07, 0.03, 0.0],
        filled_qty: [0.0, 0.0, 0.0],
    };
    state.cascade_signal.average_duration_ms = Some(12_000.0);

    let mut current = input(
        104.0,
        ModulePositions {
            long: Some(ModulePositionSummary {
                entry_price: 100.0,
                notional: 10.0,
                pnl: 0.4,
                qty: 0.1,
            }),
            short: None,
        },
    );
    current.market_decision.as_mut().unwrap().time = 20_000;
    let output = run(&current, &mut state);
    assert_eq!(
        state.exit_progress.mode,
        long_scalping::state::ExitProgressMode::Stalled
    );
    assert_eq!(output.place_orders.len(), 2);
    assert_eq!(output.place_orders[0].mark, "tp-long");
    assert_eq!(output.place_orders[1].mark, "tp-long-far");
    assert_eq!(output.place_orders[1].enter_price, 104.0);
}

#[test]
fn both_direction_filled_position_does_not_create_a_permanent_lock() {
    let mut state = EngineState::default();
    state.last_snapshot = Some(IndicatorSnapshot::default());
    let mut new_position = input(
        100.0,
        ModulePositions {
            long: Some(ModulePositionSummary {
                entry_price: 100.0,
                notional: 10.0,
                pnl: 0.0,
                qty: 0.1,
            }),
            short: None,
        },
    );
    new_position.event = ModuleEvent::NewPosition {
        direction: Direction::Long,
        entry_price: 100.0,
        qty: 0.1,
        take_profit: None,
        stop_loss: None,
    };

    run(&new_position, &mut state);
    assert_eq!(state.mrc_crossing_model.trade_direction_lock, None);

    let mut finish = input(
        100.0,
        ModulePositions {
            long: None,
            short: None,
        },
    );
    finish.event = ModuleEvent::FinishPosition {
        direction: Direction::Long,
        pnl: 0.5,
    };
    run(&finish, &mut state);
    assert_eq!(state.mrc_crossing_model.trade_direction_lock, None);
}

#[test]
fn explicit_long_start_keeps_the_long_direction_reservation() {
    let mut state = EngineState::default();
    let mut start = input(
        100.0,
        ModulePositions {
            long: None,
            short: None,
        },
    );
    start.event = ModuleEvent::Signal {
        direction: Direction::Long,
        code: "start".to_string(),
        values: HashMap::new(),
    };

    run(&start, &mut state);
    assert_eq!(state.mrc_crossing_model.trade_direction_lock, Some(1));
}

#[test]
fn mrc_entry_reason_exits_apply_minimums_once_from_entry_price() {
    let mut state = EngineState::default();
    state.cfg.min_tp_percent = 1.5;
    state.cfg.min_sl_percent = 1.5;
    state.last_snapshot = Some(IndicatorSnapshot::default());
    state.position = position(PositionState::Long, 0.0);
    state.position.take_profit = 0.0;
    state.position.entry_reason = Some(mrc_reason(100.5, 99.5));

    let positions = ModulePositions {
        long: Some(ModulePositionSummary {
            entry_price: 100.0,
            notional: 10.0,
            pnl: 0.0,
            qty: 0.1,
        }),
        short: None,
    };
    let first = run(&input(100.0, positions.clone()), &mut state);
    assert_eq!(first.place_orders.len(), 1);
    assert!((first.place_orders[0].enter_price - 101.5).abs() < 1e-9);
    assert!((state.position.take_profit - 101.5).abs() < 1e-9);
    assert!((state.position.stop_loss - 98.5).abs() < 1e-9);
    assert_eq!(state.position.exit_mode, ExitMode::Fixed);

    let second = run(&input(110.0, positions), &mut state);
    assert!(second.place_orders.is_empty());
    assert!((state.position.take_profit - 101.5).abs() < 1e-9);
    assert!((state.position.stop_loss - 98.5).abs() < 1e-9);
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
    assert_eq!(
        first.cancel_orders,
        vec!["tp-short-far", "tp-short-1", "tp-short-2", "tp-short-3"]
    );

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
    assert_eq!(
        output.cancel_orders,
        vec!["tp-long-far", "tp-long-1", "tp-long-2", "tp-long-3"]
    );
}

#[test]
fn active_mrc_position_reprices_to_the_maximum_route_candidate() {
    let (reason, analysis, context) = active_mrc_route_fixture();
    let mut state = EngineState::default();
    state.last_snapshot = Some(staircase_snapshot());
    state.active_mrc_analysis = Some(analysis);
    state.active_mrc_context = context;
    state.position = position(PositionState::Long, 100.0);
    state.position.entry_price = 105.0;
    state.position.take_profit = reason.price();
    state.position.entry_reason = Some(reason);
    state.exit_progress.begin(PositionState::Long, 1_000, 105.0);

    let output = run(
        &input(
            105.0,
            ModulePositions {
                long: Some(ModulePositionSummary {
                    entry_price: 105.0,
                    notional: 10.0,
                    pnl: 0.0,
                    qty: 0.1,
                }),
                short: None,
            },
        ),
        &mut state,
    );

    assert_eq!(output.place_orders.len(), 1);
    assert_eq!(output.place_orders[0].mark, "tp-long");
    assert!((output.place_orders[0].enter_price - 109.725).abs() < 1e-9);
    assert_eq!(
        state.position.take_profit,
        output.place_orders[0].enter_price
    );
}

#[test]
fn opposite_future_route_keeps_cached_position_protection() {
    let (_, mut future_analysis, _) = active_mrc_route_fixture();
    future_analysis.trade_direction = -1;

    let mut state = EngineState::default();
    state.last_snapshot = Some(staircase_snapshot());
    state.market_mrc_analysis = Some(future_analysis);
    state.position = position(PositionState::Long, 0.0);
    state.position.entry_reason = None;
    state.position_context = PositionContext {
        side: PositionState::Long,
        entry_price: 100.0,
        entry_anchor: Some(PositionPlanSnapshot {
            take_profit: 106.0,
            stop_loss: 97.0,
            tp_reliable: true,
        }),
        last_valid_plan: Some(PositionPlanSnapshot {
            take_profit: 106.0,
            stop_loss: 97.0,
            tp_reliable: true,
        }),
        lifetime: PositionLifetime::OneShot,
        reentry_reason: "one-shot-position".to_string(),
        ..Default::default()
    };

    let output = run(
        &input(
            100.0,
            ModulePositions {
                long: Some(ModulePositionSummary {
                    entry_price: 100.0,
                    notional: 10.0,
                    pnl: 0.0,
                    qty: 0.1,
                }),
                short: None,
            },
        ),
        &mut state,
    );

    assert_eq!(output.close_positions.len(), 0);
    assert_eq!(output.place_orders.len(), 1);
    assert_eq!(output.place_orders[0].enter_price, 106.0);
    assert_eq!(
        state.position_context.last_plan_source,
        PositionExitPlanSource::LastValidPositionPlan
    );
}

#[test]
fn invalidated_same_side_stairs_requests_position_close() {
    let (_, mut analysis, _) = active_mrc_route_fixture();
    analysis.staircase_lifecycle =
        Some(long_scalping::detection::mrc_pattern_classification::StaircaseLifecycle::Invalidated);

    let mut state = EngineState::default();
    state.last_snapshot = Some(staircase_snapshot());
    state.active_mrc_analysis = Some(analysis);
    state.position = position(PositionState::Long, 97.0);

    let output = run(
        &input(
            100.0,
            ModulePositions {
                long: Some(ModulePositionSummary {
                    entry_price: 100.0,
                    notional: 10.0,
                    pnl: 0.0,
                    qty: 0.1,
                }),
                short: None,
            },
        ),
        &mut state,
    );

    assert_eq!(output.close_positions.len(), 1);
    assert!(output.close_positions[0]
        .reason
        .contains("same-side-staircase-invalidated"));
    assert!(output.cancel_orders.contains(&"tp-long".to_string()));
    assert!(output.debug.contains("position-controller=close"));
}

#[test]
fn mrc_crossing_uses_the_shared_exit_plan_on_realtime_updates() {
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
    snapshot.tf1m.candle.close = 102.0;
    snapshot.tf5m = snapshot.tf1m;
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
        exit_mode: ExitMode::Fixed,
        exit_type: ExitType::VirtualTrailingExit { ticks: 2 },
        entry_reason: Some(long_scalping::model::EntryReason::MrcCrossing {
            price: 100.0,
            market_price: 100.0,
            mrc_min: 90.0,
            mrc_max: 110.0,
            target_price: 100.2,
            stop_price: 90.0,
            estimated_time_ms: 1_000.0,
            source_line: 1,
            target_line: 2,
            target_is_movement: false,
            pattern_kind: "STAIRS_UP".to_string(),
            pattern: vec![1, 2],
            pattern_elapsed_ms: vec![0, 1_000],
            pattern_samples: 1,
            context_samples: 0,
            no_pullback_count: 0,
            reverted_line: None,
            target_samples: 1,
            movement_samples: 1,
            next_line_crosses: 1,
            support_resistance_moves: 0,
            price_only_moves: 0,
            successful_moves: 1,
            failed_moves: 0,
            movement_success_rate: 1.0,
            expected_value_pct: 0.2,
            expected_price_low: 99.0,
            expected_price_high: 103.0,
            favorable_move_pct: 3.0,
            adverse_move_pct: -1.0,
        }),
    };
    state.take_profit_allocation = TakeProfitAllocation {
        side: PositionState::Long,
        total_qty: 0.1,
        planned_qty: [0.1, 0.0, 0.0],
        filled_qty: [0.0, 0.0, 0.0],
    };
    state.exit_progress.begin(PositionState::Long, 1_000, 100.0);
    state.exit_progress.orders_initialized = true;
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
fn mrc_crossing_applies_minimum_tp_and_sl_to_long_exit() {
    let mut state = EngineState::default();
    state.cfg.min_tp_percent = 1.0;
    state.cfg.min_sl_percent = 0.5;
    let mut snapshot = IndicatorSnapshot::default();
    snapshot.tf1m.mrc = long_scalping::model::MrcBands {
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
    };
    state.last_snapshot = Some(snapshot);
    state.position = position(PositionState::Long, 0.0);
    state.position.take_profit = 0.0;
    state.position.entry_reason = Some(EntryReason::MrcCrossing {
        price: 100.0,
        market_price: 100.0,
        mrc_min: 90.0,
        mrc_max: 110.0,
        target_price: 100.2,
        stop_price: 99.8,
        estimated_time_ms: 1_000.0,
        source_line: 4,
        target_line: 5,
        target_is_movement: false,
        pattern_kind: "STAIRS_UP".to_string(),
        pattern: vec![4, 5],
        pattern_elapsed_ms: vec![0, 1_000],
        pattern_samples: 1,
        context_samples: 0,
        no_pullback_count: 0,
        reverted_line: None,
        target_samples: 1,
        movement_samples: 1,
        next_line_crosses: 1,
        support_resistance_moves: 0,
        price_only_moves: 0,
        successful_moves: 0,
        failed_moves: 1,
        movement_success_rate: 0.0,
        expected_value_pct: -0.2,
        expected_price_low: 99.8,
        expected_price_high: 100.2,
        favorable_move_pct: 0.2,
        adverse_move_pct: -0.2,
    });
    let output = run(
        &input(
            100.0,
            ModulePositions {
                long: Some(ModulePositionSummary {
                    entry_price: 100.0,
                    notional: 10.0,
                    pnl: 0.0,
                    qty: 0.1,
                }),
                short: None,
            },
        ),
        &mut state,
    );

    assert!(state.position.take_profit >= 101.0);
    assert!(state.position.stop_loss <= 99.5);
    assert_eq!(
        output.place_orders[0].enter_price,
        state.position.take_profit
    );
}

#[test]
fn mrc_crossing_applies_minimum_tp_and_sl_to_short_exit() {
    let mut state = EngineState::default();
    state.cfg.min_tp_percent = 1.0;
    state.cfg.min_sl_percent = 0.5;
    let mut snapshot = IndicatorSnapshot::default();
    snapshot.tf1m.mrc = long_scalping::model::MrcBands {
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
    };
    state.last_snapshot = Some(snapshot);
    state.position = position(PositionState::Short, 0.0);
    state.position.take_profit = 0.0;
    state.position.entry_reason = Some(EntryReason::MrcCrossing {
        price: 100.0,
        market_price: 100.0,
        mrc_min: 90.0,
        mrc_max: 110.0,
        target_price: 99.8,
        stop_price: 100.2,
        estimated_time_ms: 1_000.0,
        source_line: 4,
        target_line: 3,
        target_is_movement: false,
        pattern_kind: "STAIRS_DOWN".to_string(),
        pattern: vec![4, 3],
        pattern_elapsed_ms: vec![0, 1_000],
        pattern_samples: 1,
        context_samples: 0,
        no_pullback_count: 0,
        reverted_line: None,
        target_samples: 1,
        movement_samples: 1,
        next_line_crosses: 1,
        support_resistance_moves: 0,
        price_only_moves: 0,
        successful_moves: 0,
        failed_moves: 1,
        movement_success_rate: 0.0,
        expected_value_pct: -0.2,
        expected_price_low: 99.8,
        expected_price_high: 100.2,
        favorable_move_pct: 0.2,
        adverse_move_pct: -0.2,
    });
    let output = run(
        &input(
            100.0,
            ModulePositions {
                long: None,
                short: Some(ModulePositionSummary {
                    entry_price: 100.0,
                    notional: 10.0,
                    pnl: 0.0,
                    qty: 0.1,
                }),
            },
        ),
        &mut state,
    );

    assert!(state.position.take_profit <= 99.0);
    assert!(state.position.stop_loss >= 100.49);
    assert_eq!(
        output.place_orders[0].enter_price,
        state.position.take_profit
    );
}

#[test]
fn expired_mrc_limit_is_cancelled_without_replacement() {
    let mut state = EngineState::default();
    state.cfg.entry_type = EntryType::Limit;
    state.cfg.required_samples = 1;
    state.bar_1m = 2;
    state.pending_entry_bar = Some(1);
    state.position.entry_reason = Some(EntryReason::MrcCrossing {
        price: 101.0,
        market_price: 101.0,
        mrc_min: 90.0,
        mrc_max: 110.0,
        target_price: 104.0,
        stop_price: 96.0,
        estimated_time_ms: 1_000.0,
        source_line: 4,
        target_line: 5,
        target_is_movement: false,
        pattern_kind: "STAIRS_UP".to_string(),
        pattern: vec![4, 5],
        pattern_elapsed_ms: vec![0, 1_000],
        pattern_samples: 1,
        context_samples: 0,
        no_pullback_count: 0,
        reverted_line: None,
        target_samples: 1,
        movement_samples: 1,
        next_line_crosses: 1,
        support_resistance_moves: 0,
        price_only_moves: 0,
        successful_moves: 1,
        failed_moves: 0,
        movement_success_rate: 1.0,
        expected_value_pct: 3.0,
        expected_price_low: 100.0,
        expected_price_high: 104.0,
        favorable_move_pct: 3.0,
        adverse_move_pct: -1.0,
    });
    state.mrc_crossing_model.history = vec![
        long_scalping::state::MrcHistoryEvent {
            line: 5,
            direction: -1,
            now_ms: 1_000,
            market_price: 0.0,
            mrc_low: 0.0,
            mrc_high: 0.0,
            no_pullback_count: 0,
            reverted_line: None,
            completed_movement: None,
        },
        long_scalping::state::MrcHistoryEvent {
            line: 2,
            direction: -1,
            now_ms: 2_000,
            market_price: 0.0,
            mrc_low: 0.0,
            mrc_high: 0.0,
            no_pullback_count: 0,
            reverted_line: None,
            completed_movement: Some(long_scalping::state::MrcMovementSnapshot {
                start_line: 5,
                start_price: 104.0,
                market_start_price: 104.0,
                target_price: 94.0,
                target_low: 94.0,
                target_high: 94.0,
                mrc_low: 90.0,
                mrc_high: 110.0,
                high: 104.0,
                low: 103.0,
                extreme: 103.0,
                direction: -1,
                duration_ms: 1_000,
            }),
        },
        long_scalping::state::MrcHistoryEvent {
            line: 5,
            direction: -1,
            now_ms: 3_000,
            market_price: 0.0,
            mrc_low: 0.0,
            mrc_high: 0.0,
            no_pullback_count: 0,
            reverted_line: None,
            completed_movement: None,
        },
        long_scalping::state::MrcHistoryEvent {
            line: 2,
            direction: -1,
            now_ms: 4_000,
            market_price: 0.0,
            mrc_low: 0.0,
            mrc_high: 0.0,
            no_pullback_count: 0,
            reverted_line: None,
            completed_movement: Some(long_scalping::state::MrcMovementSnapshot {
                start_line: 5,
                start_price: 104.0,
                market_start_price: 104.0,
                target_price: 94.0,
                target_low: 94.0,
                target_high: 94.0,
                mrc_low: 90.0,
                mrc_high: 110.0,
                high: 104.0,
                low: 103.0,
                extreme: 103.0,
                direction: -1,
                duration_ms: 1_000,
            }),
        },
    ];

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
    assert!(output.close_positions[0]
        .reason
        .contains("limit-entry-expired-after-one-candle"));
    assert!(output.open_positions.is_empty());
    assert!(output.update_positions.is_empty());
    assert_eq!(state.pending_entry_bar, None);
    assert!(state.position.entry_reason.is_none());
}
