use crate::detection::mrc_cross_analysis::MrcCrossAnalysis;
use crate::detection::mrc_crossing::LINE_LABELS;

pub fn mrc_analysis(analysis: &MrcCrossAnalysis, forecast: bool) -> String {
    let stair_sequence = line_sequence(&analysis.stair_sequence);
    let target_line = line_name(analysis.take_profit_line.or(analysis.trade_target_line));
    let stop_line = line_name(analysis.stop_loss_line);
    let protected_line = line_name(analysis.staircase_protected_line);
    let extreme_line = line_name(analysis.staircase_extreme_line);
    let barrier_line = line_name(analysis.barrier_line);
    let predicted_line = line_name(analysis.target_line);
    let speed = optional_number(analysis.cascade_speed_pct_per_min);
    let potential = optional_number(analysis.cascade_potential_move_pct);
    let adverse = optional_number(analysis.cascade_adverse_move_pct);
    let effective_move = analysis
        .effective_move_pct
        .map(|value| format!("{value:.4}%"))
        .unwrap_or_else(|| "na".to_string());

    [
        format!("forecast={forecast}"),
        format!(
            "pattern={} steps={} sequence={stair_sequence}",
            analysis
                .pattern_kind
                .map(|kind| kind.as_str())
                .unwrap_or("none"),
            analysis.pattern_steps
        ),
        format!(
            "stair_move={:.4}% favorable={:.4}% adverse={:.4}%",
            analysis.stair_price_move_pct,
            analysis.stair_favorable_move_pct,
            analysis.stair_adverse_move_pct
        ),
        format!(
            "lifecycle={} transition={} protected={} extreme={}",
            analysis
                .staircase_lifecycle
                .map(|value| value.as_str())
                .unwrap_or("na"),
            analysis
                .staircase_transition
                .map(|value| value.as_str())
                .unwrap_or("na"),
            protected_line,
            extreme_line
        ),
        format!(
            "entry_line={} entry_state={} entry_reason={}",
            line_name(analysis.stair_entry_line),
            analysis
                .stair_entry_state
                .map(|value| value.as_str())
                .unwrap_or("na"),
            analysis.stair_entry_reason.as_deref().unwrap_or("na")
        ),
        format!(
            "reversal={} reversal_reason={}",
            analysis.reversal_state.as_str(),
            analysis.reversal_reason.as_deref().unwrap_or("na")
        ),
        format!(
            "cascade={} direction={} speed={} phase={} potential={} adverse={} trade_state={}",
            analysis.cascade_enabled,
            analysis.cascade_direction,
            speed,
            analysis.cascade_phase.as_str(),
            potential,
            adverse,
            analysis.cascade_trade_state.as_str()
        ),
        format!(
            "barrier={}:{}@{:.8} predicted_line={}@{:.8} predicted_direction={} trade_direction={}",
            analysis.barrier_state.as_str(),
            barrier_line,
            analysis.barrier_price.unwrap_or_default(),
            predicted_line,
            analysis.predicted_line_price.unwrap_or_default(),
            analysis.predicted_direction,
            analysis.trade_direction
        ),
        format!(
            "tp_line={target_line} tp={:.8} sl_line={stop_line} sl={:.8}",
            analysis.target_price.unwrap_or_default(),
            analysis.stop_price.unwrap_or_default()
        ),
        format!(
            "history_scope={} samples={} context_samples={} target_samples={} movement_samples={} successful_moves={} failed_moves={} effective_move={effective_move}",
            if analysis.context_samples > 0 {
                "contextual"
            } else {
                "broad-fallback"
            },
            analysis.pattern_samples,
            analysis.context_samples,
            analysis.target_samples,
            analysis.movement_samples,
            analysis.successful_moves,
            analysis.failed_moves,
        ),
        format!(
            "volume_gain={:.3} volume_gain_min={:.3} volume_gain_max={:.3} buy_sell_gain={:.3}",
            analysis.volume_gain,
            analysis.volume_gain_min,
            analysis.volume_gain_max,
            analysis.buy_sell_gain
        ),
    ]
    .join(" ")
}

fn line_sequence(sequence: &[u8]) -> String {
    sequence
        .iter()
        .map(|line| line_name(Some(*line)))
        .collect::<Vec<_>>()
        .join(">")
}

fn line_name(line: Option<u8>) -> &'static str {
    line.and_then(|index| LINE_LABELS.get(index as usize).copied())
        .unwrap_or("na")
}

fn optional_number(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "na".to_string())
}
