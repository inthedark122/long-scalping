use std::collections::{BTreeMap, HashMap};

use long_scalping::detection::decision::{
    CandidateView, DecisionComponents, DecisionState, PrimaryKind, PrimaryPhase,
};
use long_scalping::detection::mrc_pattern_classification::{
    MrcPatternKind, MrcTransitionKind, StaircaseLifecycle,
};
use long_scalping::engine::run;
use long_scalping::state::EngineState;
use lte_strategy_bridge::abi::{ModuleEvent, ModuleInput, ModulePositions, ModuleSymbolInfo};

fn candidate(
    index: usize,
    pattern_kind: MrcPatternKind,
    score: f64,
    direction: i8,
) -> CandidateView {
    CandidateView {
        index,
        pattern_kind,
        score,
        entry: true,
        direction,
        volume_gain: 1.0,
        realized_favorable_pct: None,
        realized_adverse_pct: None,
        market_price: 100.0,
        staircase_lifecycle: None,
        staircase_transition: None,
        transition: None,
    }
}

#[test]
fn stair_is_primary_and_fbr_only_marks_a_pullback() {
    let mut state = DecisionState::default();
    let components = DecisionComponents::default();

    let first = candidate(0, MrcPatternKind::StairsUp, 0.2, 1);
    let result = state.choose(components, &[first], 1_000);
    assert_eq!(result.primary_kind, Some(PrimaryKind::Stairs));
    assert_eq!(result.selected_index, Some(0));

    let mut stair = candidate(0, MrcPatternKind::StairsUp, 0.2, 1);
    stair.realized_favorable_pct = Some(0.8);
    let mut fbr = candidate(1, MrcPatternKind::FastBreakReturn, 10.0, -1);
    fbr.entry = false;
    let result = state.choose(components, &[stair, fbr], 2_000);

    assert_eq!(result.selected_index, Some(0));
    assert_eq!(result.primary_kind, Some(PrimaryKind::Stairs));
    assert_eq!(result.primary_phase, Some(PrimaryPhase::Pullback));
    assert!(result.support.fast_break_return);
    assert!(result.support.pullback);
}

#[test]
fn stairs_end_after_the_favorable_extreme_stops_updating() {
    let mut state = DecisionState::default();
    let components = DecisionComponents::default();
    let mut first = candidate(0, MrcPatternKind::StairsUp, 0.2, 1);
    first.realized_favorable_pct = Some(1.0);
    assert_eq!(
        state.choose(components, &[first], 1_000).selected_index,
        Some(0)
    );

    let mut failed = candidate(0, MrcPatternKind::StairsUp, 0.2, 1);
    failed.realized_favorable_pct = Some(0.3);
    failed.realized_adverse_pct = Some(-0.6);
    let result = state.choose(components, &[failed], 2_000);

    assert_eq!(result.selected_index, None);
    assert_eq!(result.primary_kind, None);
    assert!(result.support.favorable_extreme_failed);
}

#[test]
fn protected_swing_break_ends_stairs_and_blocks_rearming() {
    let mut state = DecisionState::default();
    let mut stairs = candidate(0, MrcPatternKind::StairsUp, 0.2, 1);
    assert_eq!(
        state
            .choose(DecisionComponents::default(), &[stairs], 1_000)
            .selected_index,
        Some(0)
    );

    stairs.staircase_lifecycle = Some(StaircaseLifecycle::Invalidated);
    let result = state.choose(DecisionComponents::default(), &[stairs], 2_000);
    assert_eq!(result.selected_index, None);
    assert!(state.trace.ended_by_protected_swing);
    assert_eq!(state.trace.reason, "primary-ended-awaiting-rearm");
}

#[test]
fn confirmed_reversal_replaces_an_invalidated_staircase_primary() {
    let mut state = DecisionState::default();
    let components = DecisionComponents::default();
    let stairs = candidate(0, MrcPatternKind::StairsUp, 0.2, 1);
    assert_eq!(
        state.choose(components, &[stairs], 1_000).selected_index,
        Some(0)
    );

    let mut invalidated = candidate(0, MrcPatternKind::StairsUp, 0.2, 1);
    invalidated.staircase_lifecycle = Some(StaircaseLifecycle::Invalidated);
    let mut reversal = candidate(1, MrcPatternKind::StairsDown, 0.0, -1);
    reversal.transition = Some(MrcTransitionKind::Reversal);
    let result = state.choose(components, &[invalidated, reversal], 2_000);

    assert_eq!(result.selected_index, Some(1));
    assert_eq!(result.primary_kind, None);
    assert_eq!(result.primary_direction, 0);
    assert!(state.trace.ended_by_protected_swing);
    assert_eq!(state.trace.reason, "selected-transition-route");
}

#[test]
fn reversal_transition_does_not_replace_an_active_staircase_primary() {
    let mut state = DecisionState::default();
    let components = DecisionComponents::default();
    let stairs = candidate(0, MrcPatternKind::StairsUp, 0.2, 1);
    assert_eq!(
        state.choose(components, &[stairs], 1_000).selected_index,
        Some(0)
    );

    let mut reversal = candidate(1, MrcPatternKind::StairsDown, 10.0, -1);
    reversal.transition = Some(MrcTransitionKind::Reversal);
    let result = state.choose(components, &[reversal], 2_000);

    assert_eq!(result.selected_index, Some(1));
    assert_eq!(result.primary_kind, Some(PrimaryKind::Stairs));
    assert_eq!(result.primary_direction, 1);
    assert_eq!(state.trace.reason, "selected-transition-route");
}

#[test]
fn direction_lock_blocks_opposite_routes_and_keeps_stairs_authority() {
    let mut state = DecisionState::default();
    let components = DecisionComponents::default();
    let long_stairs = candidate(0, MrcPatternKind::StairsUp, 0.2, 1);
    assert_eq!(
        state
            .choose_with_direction_lock(components, &[long_stairs], Some(1), 1_000)
            .selected_index,
        Some(0)
    );

    let short_stairs = candidate(1, MrcPatternKind::StairsDown, 10.0, -1);
    let result = state.choose_with_direction_lock(components, &[short_stairs], Some(1), 2_000);
    assert_eq!(result.selected_index, None);
    assert_eq!(result.primary_kind, Some(PrimaryKind::Stairs));
    assert_eq!(state.trace.blocked_by_direction_lock, 1);
    assert_eq!(state.trace.reason, "direction-lock-wait");
}

#[test]
fn direction_lock_resets_an_opposite_primary_before_rearming() {
    let mut state = DecisionState::default();
    let components = DecisionComponents::default();
    let short_stairs = candidate(0, MrcPatternKind::StairsDown, 0.2, -1);
    assert_eq!(
        state
            .choose(components, &[short_stairs], 1_000)
            .selected_index,
        Some(0)
    );

    let long_fbr = candidate(1, MrcPatternKind::FastBreakReturn, 0.1, 1);
    let result = state.choose_with_direction_lock(components, &[long_fbr], Some(1), 2_000);
    assert_eq!(result.selected_index, None);
    assert_eq!(result.primary_kind, None);
    assert!(state.trace.direction_lock_reset_primary);
}

#[test]
fn volume_and_trend_are_support_evidence_not_entry_authority() {
    let mut state = DecisionState::default();
    let mut stairs = candidate(0, MrcPatternKind::StairsUp, 0.2, 1);
    stairs.volume_gain = 2.5;
    let trend = candidate(1, MrcPatternKind::TrendingCycle, 100.0, 1);

    let result = state.choose(DecisionComponents::default(), &[stairs, trend], 1_000);
    assert_eq!(result.selected_index, Some(0));
    assert!(result.support.volume_climax);
    assert!(result.support.trend);
    assert_eq!(result.primary_phase, Some(PrimaryPhase::Exhaustion));
}

#[test]
fn disabling_fbr_removes_pullback_support_without_changing_the_primary() {
    let mut state = DecisionState::default();
    let mut components = DecisionComponents::default();
    let stairs = candidate(0, MrcPatternKind::StairsUp, 0.2, 1);
    state.choose(components, &[stairs], 1_000);

    components.support_fast_break_return = false;
    let stairs = candidate(0, MrcPatternKind::StairsUp, 0.2, 1);
    let mut fbr = candidate(1, MrcPatternKind::FastBreakReturn, 5.0, -1);
    fbr.entry = false;
    let result = state.choose(components, &[stairs, fbr], 2_000);

    assert_eq!(result.primary_kind, Some(PrimaryKind::Stairs));
    assert!(!result.support.fast_break_return);
    assert!(!result.support.pullback);
}

fn action_input(name: &str, values: HashMap<String, serde_json::Value>) -> ModuleInput {
    ModuleInput {
        event: ModuleEvent::Action {
            name: name.to_string(),
            values,
        },
        price: 100.0,
        symbol: "TEST".to_string(),
        max_amount: 100.0,
        symbol_info: ModuleSymbolInfo::default(),
        auto_max_amount: 100.0,
        leverage: 5,
        indicators: BTreeMap::new(),
        positions: ModulePositions {
            long: None,
            short: None,
        },
        sug_info: None,
        state: None,
    }
}

#[test]
fn decision_components_can_be_replaced_added_and_removed_at_runtime() {
    let mut state = EngineState::default();
    let mut values = HashMap::new();
    values.insert(
        "components".to_string(),
        serde_json::json!(["primary-stairs", "support-volume"]),
    );
    run(&action_input("decision_components", values), &mut state);
    assert_eq!(
        state.cfg.decision_components,
        vec!["primary-stairs", "support-volume"]
    );
    assert!(state.mrc_crossing_model.decision_components.primary_stairs);
    assert!(state.mrc_crossing_model.decision_components.primary_stairs);

    let mut values = HashMap::new();
    values.insert("add".to_string(), serde_json::json!("support-trend"));
    run(&action_input("decision-components", values), &mut state);
    assert!(state
        .cfg
        .decision_components
        .contains(&"support-trend".to_string()));

    let mut values = HashMap::new();
    values.insert("remove".to_string(), serde_json::json!("support-volume"));
    run(&action_input("decision_components", values), &mut state);
    assert!(!state
        .cfg
        .decision_components
        .contains(&"support-volume".to_string()));
}

#[test]
fn primary_modes_keep_support_but_limit_entry_authority() {
    let stairs = DecisionComponents::for_primary_mode("stairs").expect("stairs mode");
    assert!(stairs.primary_stairs);
    assert!(stairs.support_fast_break_return);
}
