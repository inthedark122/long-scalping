use crate::detection::cascade::CascadeTradeState;
use crate::detection::mrc_cross_analysis::MrcCrossAnalysis;
use crate::detection::mrc_crossing;
use crate::detection::mrc_pattern_classification::MrcPatternKind;
use crate::detection::mrc_prediction::MrcPredictionContext;
use crate::model::PositionState;
use crate::state::EngineState;

pub fn update(
    state: &mut EngineState,
    analysis: Option<&MrcCrossAnalysis>,
    selected_pattern_reason: &str,
    prediction_context: MrcPredictionContext,
    trend_entry_selected: bool,
    trend_summary: &str,
    pattern_summary: &str,
) {
    state.direction_source = "pattern-selector".to_string();
    update_active_analysis(state, analysis, prediction_context);
    update_staircase(state, analysis);

    state.selected_cascade_trade_state = analysis
        .map(|analysis| analysis.cascade_trade_state)
        .unwrap_or(CascadeTradeState::NotConfigured);
    state.direction_reason = analysis
        .map(mrc_crossing::line_plan_summary)
        .unwrap_or_else(|| format!("pattern={selected_pattern_reason}"));
    state.direction_reason.push(' ');
    state.direction_reason.push_str(pattern_summary);
    if trend_entry_selected {
        state.direction_source = "kline-candle-trend".to_string();
        state.direction_reason = format!("{trend_summary} {pattern_summary}");
    }
}

fn update_active_analysis(
    state: &mut EngineState,
    analysis: Option<&MrcCrossAnalysis>,
    prediction_context: MrcPredictionContext,
) {
    let Some(analysis) = analysis.filter(|analysis| is_staircase(analysis.pattern_kind)) else {
        state.market_mrc_analysis = None;
        return;
    };

    state.market_mrc_analysis = Some(analysis.clone());
    let position_matches = matches!(
        (state.position.side, analysis.trade_direction),
        (PositionState::Long, 1) | (PositionState::Short, -1)
    );
    if state.position.side == PositionState::Flat || position_matches {
        state.active_mrc_analysis = Some(analysis.clone());
        state.active_mrc_context = prediction_context;
    }
}

fn update_staircase(state: &mut EngineState, analysis: Option<&MrcCrossAnalysis>) {
    let Some(analysis) = analysis.filter(|analysis| is_staircase(analysis.pattern_kind)) else {
        return;
    };
    let direction = structural_direction(analysis.pattern_kind);
    let staircase = &mut state.staircase_progress;
    staircase.direction = direction;
    staircase.lifecycle = analysis.staircase_lifecycle;
    staircase.transition = analysis.staircase_transition;
    staircase.protected_line = analysis.staircase_protected_line;
    staircase.extreme_line = analysis.staircase_extreme_line;
    remember_extreme(
        &mut staircase.favorable_extreme_price,
        &mut staircase.favorable_extreme_time_ms,
        analysis.stair_favorable_extreme_price,
        analysis.stair_favorable_extreme_time_ms,
    );
    remember_extreme(
        &mut staircase.adverse_extreme_price,
        &mut staircase.adverse_extreme_time_ms,
        analysis.stair_adverse_extreme_price,
        analysis.stair_adverse_extreme_time_ms,
    );

    let position_direction = match state.position.side {
        PositionState::Long => 1,
        PositionState::Short => -1,
        PositionState::Flat => 0,
    };
    if matches!(position_direction, 1 | -1) {
        let (price, time_ms) = if position_direction == direction {
            (
                analysis.stair_favorable_extreme_price,
                analysis.stair_favorable_extreme_time_ms,
            )
        } else {
            (
                analysis.stair_adverse_extreme_price,
                analysis.stair_adverse_extreme_time_ms,
            )
        };
        state
            .exit_progress
            .observe_extreme(position_direction, price, time_ms);
    }
    staircase.pattern_elapsed_ms = analysis.pattern_elapsed_ms;
    staircase.updated_at_ms = analysis.now_ms;
}

fn remember_extreme(price: &mut f64, time_ms: &mut u64, next_price: f64, next_time_ms: u64) {
    if next_time_ms >= *time_ms {
        *price = next_price;
        *time_ms = next_time_ms;
    }
}

fn is_staircase(kind: Option<MrcPatternKind>) -> bool {
    matches!(
        kind,
        Some(MrcPatternKind::StairsUp | MrcPatternKind::StairsDown)
    )
}

fn structural_direction(kind: Option<MrcPatternKind>) -> i8 {
    match kind {
        Some(MrcPatternKind::StairsUp) => 1,
        Some(MrcPatternKind::StairsDown) => -1,
        _ => 0,
    }
}
