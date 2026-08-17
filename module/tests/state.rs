use long_scalping::{detection::mrc_pattern_classification::MrcPatternKind, state};

#[test]
fn pattern_learning_is_restored_from_bridge_state() {
    let mut source = state::EngineState::default();
    source.ticks = 42;
    source.mrc_crossing_model.pattern_performance[MrcPatternKind::StairsUp.index()]
        .record_trade(1.25);
    source.mrc_crossing_model.active_pattern = Some(MrcPatternKind::StairsUp.index() as u8);
    source.mrc_crossing_model.active_pattern_score = 1.25;

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
}
