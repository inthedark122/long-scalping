use long_scalping::detection::cascade::MovementPhase;
use long_scalping::detection::mrc_cross_analysis;
use long_scalping::detection::mrc_crossing::MrcPrediction;
use long_scalping::detection::mrc_pattern_classification::{
    MrcPatternKind, StaircaseLifecycle, StaircaseTransition,
};
use long_scalping::detection::mrc_prediction::MrcPredictionContext;
use long_scalping::detection::mrc_stair_entry::{self, StairEntryState};
use long_scalping::detection::mrc_touch::{MrcLineTouchEma, MrcTouchRegime, MrcTouchSignal};
use long_scalping::detection::vlines::MrcCrossEvent;
use long_scalping::state::{MrcCrossingState, MrcForecastStats};

fn lines() -> [f64; 9] {
    [90.0, 92.0, 94.0, 96.0, 100.0, 104.0, 106.0, 108.0, 110.0]
}

fn event(line_idx: u8, now_ms: u64) -> MrcCrossEvent {
    MrcCrossEvent {
        line_idx,
        line: lines()[line_idx as usize],
        price: lines()[line_idx as usize],
        mrc_low: lines()[0],
        mrc_high: lines()[8],
        direction: 1,
        tick: now_ms / 1_000,
        now_ms,
        no_pullback_count: 0,
        reverted_line: None,
        completed_movement: None,
    }
}

fn prediction(
    lifecycle: StaircaseLifecycle,
    transition: StaircaseTransition,
    extreme_line: u8,
    target_line: u8,
) -> MrcPrediction {
    let mut line_stats = [MrcForecastStats::default(); 9];
    line_stats[target_line as usize] = MrcForecastStats {
        count: 8,
        movement_samples: 8,
        duration_sum_ms: 8 * 60_000,
        favorable_pct_sum: 24.0,
        adverse_pct_sum: 8.0,
        outcome_pct_sum: 16.0,
        success_count: 6,
        failure_count: 2,
        ..Default::default()
    };
    MrcPrediction {
        pattern_kind: MrcPatternKind::StairsUp,
        pattern_direction: 1,
        pattern_steps: 3,
        pattern_elapsed_ms: 10 * 60_000,
        stair_sequence: vec![4, 5, 6, 5, 6],
        stair_price_move_pct: 5.0,
        stair_favorable_move_pct: 6.0,
        stair_adverse_move_pct: 0.5,
        favorable_extreme_price: lines()[extreme_line as usize],
        favorable_extreme_time_ms: 9 * 60_000,
        adverse_extreme_price: lines()[4],
        adverse_extreme_time_ms: 2 * 60_000,
        staircase_lifecycle: Some(lifecycle),
        staircase_transition: Some(transition),
        staircase_protected_line: Some(4),
        staircase_extreme_line: Some(extreme_line),
        pattern: vec![target_line],
        elapsed_ms: vec![8 * 60_000],
        target_line,
        samples: 8,
        stats: line_stats[target_line as usize],
        line_stats,
        next_line_crosses: 8,
        support_resistance_moves: 2,
        price_only_moves: 0,
        context_samples: 4,
        failed_break_line: None,
        failed_break_direction: 0,
    }
}

fn context(phase: MovementPhase, speed: f64, adverse: f64) -> MrcPredictionContext {
    MrcPredictionContext {
        cascade_enabled: true,
        cascade_direction: 1,
        cascade_speed_pct_per_min: Some(speed),
        cascade_phase: phase,
        cascade_potential_move_pct: Some(5.0),
        cascade_adverse_move_pct: Some(adverse),
        higher_timeframe_slope: 1,
        higher_timeframe_bullish: true,
        ..Default::default()
    }
}

fn supportive_touch() -> MrcTouchSignal {
    let mut lines = [MrcLineTouchEma::default(); 9];
    lines[5].near_ema = 0.50;
    MrcTouchSignal {
        lines,
        nearest_line: Some(5),
        price_speed_pct_per_min: 0.30,
        stairs_pressure: 0.60,
        interest: 0.60,
        regime: MrcTouchRegime::Long,
        ..Default::default()
    }
}

fn analysis(
    line_idx: u8,
    lifecycle: StaircaseLifecycle,
    transition: StaircaseTransition,
    extreme_line: u8,
    phase: MovementPhase,
    speed: f64,
    adverse: f64,
) -> long_scalping::detection::mrc_cross_analysis::MrcCrossAnalysis {
    analysis_with_touch(
        line_idx,
        lifecycle,
        transition,
        extreme_line,
        phase,
        speed,
        adverse,
        supportive_touch(),
    )
}

fn analysis_with_touch(
    line_idx: u8,
    lifecycle: StaircaseLifecycle,
    transition: StaircaseTransition,
    extreme_line: u8,
    phase: MovementPhase,
    speed: f64,
    adverse: f64,
    touch: MrcTouchSignal,
) -> long_scalping::detection::mrc_cross_analysis::MrcCrossAnalysis {
    let prediction = prediction(lifecycle, transition, extreme_line, 7);
    let context = context(phase, speed, adverse);
    let mut analysis = mrc_cross_analysis::build_with_context(
        &event(line_idx, 10 * 60_000),
        Some(&prediction),
        lines(),
        &MrcCrossingState::default(),
        (0.0, 0.0, 0.0, 0.0),
        context,
    );
    mrc_stair_entry::apply(&mut analysis, lines(), touch, context);
    analysis
}

#[test]
fn stairs_up_at_fresh_extreme_uses_retest_price() {
    let analysis = analysis(
        6,
        StaircaseLifecycle::Progressing,
        StaircaseTransition::HigherExtreme,
        6,
        MovementPhase::Building,
        0.8,
        0.2,
    );

    assert_eq!(analysis.stair_entry_state, Some(StairEntryState::Retest));
    assert_eq!(analysis.stair_entry_line, Some(5));
    assert_eq!(analysis.effective_entry_price, Some(104.0));
    assert_eq!(analysis.take_profit_line, Some(7));
    assert_eq!(analysis.stop_loss_line, Some(4));
    assert!(analysis.tradeable());
}

#[test]
fn stairs_up_pullback_uses_protected_line_without_cascade_direction() {
    let analysis = analysis(
        5,
        StaircaseLifecycle::Pullback,
        StaircaseTransition::PullbackWithinStructure,
        6,
        MovementPhase::CounterMove,
        0.5,
        0.2,
    );

    assert_eq!(analysis.stair_entry_state, Some(StairEntryState::Pullback));
    assert_eq!(analysis.stair_entry_line, Some(5));
    assert_eq!(analysis.effective_entry_price, Some(104.0));
    assert_eq!(analysis.take_profit_line, Some(7));
    assert!(analysis.tradeable());
}

#[test]
fn stairs_up_stops_at_extreme_when_movement_is_not_fresh() {
    let analysis = analysis_with_touch(
        6,
        StaircaseLifecycle::Stalled,
        StaircaseTransition::AttemptingExtreme,
        6,
        MovementPhase::Mature,
        0.3,
        0.2,
        MrcTouchSignal::default(),
    );

    assert_eq!(analysis.stair_entry_state, Some(StairEntryState::Top));
    assert_eq!(
        analysis.stair_entry_reason.as_deref(),
        Some("latest-extreme-without-fresh-movement")
    );
    assert!(!analysis.stair_entry_approved);
    assert!(!analysis.tradeable());
}

#[test]
fn stairs_up_hard_adverse_move_is_rejected_even_with_structure() {
    let analysis = analysis(
        6,
        StaircaseLifecycle::Progressing,
        StaircaseTransition::HigherExtreme,
        6,
        MovementPhase::Building,
        0.8,
        8.0,
    );

    assert_eq!(
        analysis.stair_entry_state,
        Some(StairEntryState::RiskRejected)
    );
    assert!(!analysis.tradeable());
}
