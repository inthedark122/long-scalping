use long_scalping::{
    detection::mrc_pattern_classification::MrcPatternKind, model::PositionState, state,
};

#[test]
fn market_analysis_is_rebuilt_instead_of_restored_from_bridge_state() {
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
    source.mrc_crossing_model.trade_direction_lock = Some(-1);

    let saved = state::save(&source).expect("state should serialize");
    let mut restored = state::EngineState::default();

    assert!(saved.get("ticks").is_none());
    assert!(saved.get("pattern_performance").is_none());
    assert!(saved.get("active_pattern").is_none());
    assert!(saved.get("trade_direction_lock").is_none());
    assert!(state::restore(&mut restored, Some(&saved)));
    assert_eq!(restored.ticks, 0);
    assert_eq!(
        restored.mrc_crossing_model.pattern_performance[MrcPatternKind::StairsUp.index()].trades,
        0
    );
    assert_eq!(restored.mrc_crossing_model.active_pattern, None);
    assert_eq!(restored.mrc_crossing_model.active_pattern_score, 0.0);
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
    assert_eq!(restored.mrc_crossing_model.trade_direction_lock, None);
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

#[test]
fn position_context_anchor_and_lifecycle_are_restored() {
    let mut source = state::EngineState::default();
    source.position_context.begin(
        PositionState::Long,
        100.0,
        1_000,
        None,
        state::PositionLifetime::RecyclableStairs,
        vec![4, 5, 6],
    );
    source.position_context.entry_anchor = Some(state::PositionPlanSnapshot {
        take_profit: 106.0,
        stop_loss: 97.0,
        tp_reliable: true,
    });
    source.position_context.last_valid_plan = source.position_context.entry_anchor;
    source.position_context.last_plan_source = state::PositionExitPlanSource::EntryAnchor;

    let saved = state::save(&source).expect("state should serialize");
    let mut restored = state::EngineState::default();
    assert!(state::restore(&mut restored, Some(&saved)));

    assert_eq!(restored.position_context.side, PositionState::Long);
    assert_eq!(restored.position_context.entry_price, 100.0);
    assert_eq!(
        restored.position_context.entry_anchor,
        source.position_context.entry_anchor
    );
    assert_eq!(
        restored.position_context.last_plan_source,
        state::PositionExitPlanSource::EntryAnchor
    );
    assert_eq!(
        restored.position_context.lifetime,
        state::PositionLifetime::RecyclableStairs
    );
}

#[test]
fn recyclable_position_retains_only_bounded_reentry_context_after_flat() {
    let mut context = state::PositionContext::default();
    context.begin(
        PositionState::Long,
        100.0,
        1_000,
        None,
        state::PositionLifetime::RecyclableStairs,
        vec![4, 5, 6],
    );
    context.on_flat("tp-filled");

    assert_eq!(context.side, PositionState::Flat);
    assert!(context.reentry_allowed);
    assert_eq!(context.entry_price, 0.0);
    assert!(context.entry_anchor.is_none());
    assert!(context.last_valid_plan.is_none());
}
