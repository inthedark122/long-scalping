use long_scalping::model::PositionState;
use long_scalping::{
    detection::{
        decision::{PrimaryKind, PrimaryPhase},
        mrc_pattern_classification::MrcPatternKind,
    },
    state,
};

#[test]
fn pattern_learning_is_restored_from_bridge_state() {
    let mut source = state::EngineState::default();
    source.ticks = 42;
    source.mrc_crossing_model.pattern_performance[MrcPatternKind::StairsUp.index()]
        .record_trade(1.25);
    source.mrc_crossing_model.active_pattern = Some(MrcPatternKind::StairsUp.index() as u8);
    source.mrc_crossing_model.active_pattern_score = 1.25;
    source.cfg.decision_components =
        vec!["primary-stairs".to_string(), "support-volume".to_string()];
    source.mrc_crossing_model.decision_components =
        long_scalping::detection::decision::DecisionComponents::from_names(
            &source.cfg.decision_components,
        )
        .0;
    source.mrc_crossing_model.decision_state.primary =
        Some(long_scalping::detection::decision::PrimaryState {
            kind: PrimaryKind::Stairs,
            direction: -1,
            phase: PrimaryPhase::Exhaustion,
            started_at_ms: 123,
            best_favorable_pct: 0.8,
            last_price: 100.0,
            volume_climax: true,
        });
    source.mrc_crossing_model.trade_direction_lock = Some(-1);

    let saved = state::save(&source).expect("state should serialize");
    let mut restored = state::EngineState::default();

    assert!(state::restore(&mut restored, Some(&saved)));
    assert_eq!(restored.ticks, 42);
    assert_eq!(
        restored.mrc_crossing_model.pattern_performance[MrcPatternKind::StairsUp.index()].trades,
        1
    );
    assert_eq!(
        restored.mrc_crossing_model.active_pattern,
        Some(MrcPatternKind::StairsUp.index() as u8)
    );
    assert_eq!(restored.mrc_crossing_model.active_pattern_score, 1.25);
    assert_eq!(
        restored.cfg.decision_components,
        vec!["primary-stairs", "support-volume"]
    );
    assert!(
        restored
            .mrc_crossing_model
            .decision_components
            .primary_stairs
    );
    assert_eq!(
        restored.mrc_crossing_model.decision_state.primary,
        Some(long_scalping::detection::decision::PrimaryState {
            kind: PrimaryKind::Stairs,
            direction: -1,
            phase: PrimaryPhase::Exhaustion,
            started_at_ms: 123,
            best_favorable_pct: 0.8,
            last_price: 100.0,
            volume_climax: true,
        })
    );
    assert_eq!(restored.mrc_crossing_model.trade_direction_lock, Some(-1));
}

#[test]
fn flat_position_clears_only_the_current_plan_deduplication_key() {
    let mut engine = state::EngineState::default();
    engine.position.side = PositionState::Long;
    engine.mrc_crossing_model.last_plan_key = Some((1, 2, 3));
    engine.mrc_crossing_model.trade_direction_lock = Some(1);

    state::sync_position_state(&mut engine, None, None, 100.0);

    assert_eq!(engine.mrc_crossing_model.last_plan_key, None);
    assert_eq!(engine.mrc_crossing_model.trade_direction_lock, Some(1));
}

#[test]
fn take_profit_allocation_is_restored_with_the_bot_state() {
    let mut source = state::EngineState::default();
    source.take_profit_allocation = state::TakeProfitAllocation {
        side: PositionState::Short,
        total_qty: 157.0,
        planned_qty: [63.0, 47.0, 47.0],
        filled_qty: [63.0, 0.0, 0.0],
    };

    let saved = state::save(&source).expect("state should serialize");
    let mut restored = state::EngineState::default();
    assert!(state::restore(&mut restored, Some(&saved)));
    assert_eq!(
        restored.take_profit_allocation,
        source.take_profit_allocation
    );
}
