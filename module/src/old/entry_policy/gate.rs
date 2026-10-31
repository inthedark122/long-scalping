use super::notes;
use crate::detection::mrc_cross_analysis::MrcCrossAnalysis;
use crate::detection::mrc_crossing::Observation;
use crate::detection::{mrc_crossing, mrc_pattern_classification};
use crate::model::IndicatorSnapshot;
use crate::model::{EntryGate, EntryReason, EntryWait, MarketState, PositionState};
use crate::pattern_matching;
use crate::state::EngineState;
use lte_strategy_bridge::abi::ModuleInput;

pub fn build_mrc_entry_gate(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    observation: &Observation,
    selected_analysis: Option<&MrcCrossAnalysis>,
    entry_direction: MarketState,
    pattern: &pattern_matching::Evaluation,
) -> EntryGate {
    let mut gate = mrc_crossing::entry_gate(&state.mrc_crossing_model);

    if let Some(mut reason) = observation.entry.clone() {
        let direction = direction_sign(entry_direction);
        let target_extended = pattern.forecast.is_some_and(|forecast| {
            pattern_matching::extend_target_to_forecast(&mut reason, forecast)
        });
        let stop_buffer_note = widen_entry_stop(&mut reason, direction, snapshot);
        if !valid_entry_levels(&reason, direction, input.symbol_info.price_tick_size) {
            state.mrc_crossing_model.current_entry = None;
            state.mrc_crossing_model.last_plan_key = None;
            return EntryGate::Wait(EntryWait {
                reason: "mrc-invalid-exit-levels".to_string(),
                solutions: Vec::new(),
                notes: vec!["MRC target/stop collapse to the entry tick".to_string()],
            });
        }

        let mut notes = vec![
            "strategy=pattern-engine".to_string(),
            format!("samples={}", observation.samples),
            format!(
                "selected_pattern={}",
                observation
                    .selected_pattern
                    .map(|pattern| pattern.as_str())
                    .unwrap_or("none")
            ),
            format!(
                "selected_score={:.4}",
                observation.selected_score.unwrap_or_default()
            ),
        ];
        if let Some(analysis) = selected_analysis {
            notes.push(notes::mrc_analysis(
                analysis,
                observation.forecast.is_some(),
            ));
        }
        if let Some(note) = stop_buffer_note {
            notes.push(note);
        }
        notes.push(pattern.summary.clone());
        if target_extended {
            notes.push("take-profit-extended-by-pattern-history".to_string());
        }
        gate = EntryGate::Open {
            notes,
            reason: crate::model::EntryOpenReason::Entry(reason),
        };
    } else if let Some(wait_reason) = observation.wait_reason {
        gate = mrc_crossing::wait_entry_gate(wait_reason);
    }
    gate
}

pub fn build_candle_trend_gate(
    input: &ModuleInput,
    entry: Option<EntryReason>,
    summary: &str,
    pattern: &pattern_matching::Evaluation,
) -> EntryGate {
    match entry.map(|mut reason| {
        if let Some(forecast) = pattern.forecast {
            pattern_matching::extend_target_to_forecast(&mut reason, forecast);
        }
        reason
    }) {
        Some(
            reason @ EntryReason::CandleTrend {
                direction,
                target_price,
                stop_price,
                ..
            },
        ) if mrc_crossing::valid_price_levels_for_pattern(
            direction,
            reason.price(),
            target_price,
            stop_price,
            Some(input.symbol_info.price_tick_size),
            None,
        ) =>
        {
            EntryGate::Open {
                notes: vec![
                    "strategy=candle-trend".to_string(),
                    pattern.summary.clone(),
                    format!(
                        "selected_pattern={}",
                        if direction > 0 {
                            "CANDLE_UPTREND"
                        } else {
                            "CANDLE_DOWNTREND"
                        }
                    ),
                    summary.to_string(),
                ],
                reason: crate::model::EntryOpenReason::Entry(reason),
            }
        }
        _ => EntryGate::Wait(EntryWait {
            reason: "candle-trend-invalid-exit-levels".to_string(),
            solutions: Vec::new(),
            notes: vec![summary.to_string()],
        }),
    }
}

fn direction_sign(direction: MarketState) -> i8 {
    match direction {
        MarketState::Long => 1,
        MarketState::Short => -1,
        MarketState::Sideways => 0,
    }
}

fn widen_entry_stop(
    reason: &mut EntryReason,
    direction: i8,
    snapshot: &IndicatorSnapshot,
) -> Option<String> {
    let EntryReason::MrcCrossing {
        price, stop_price, ..
    } = reason
    else {
        return None;
    };
    let side = match direction {
        1 => PositionState::Long,
        -1 => PositionState::Short,
        _ => PositionState::Flat,
    };
    let raw_stop = *stop_price;
    let buffered_stop =
        crate::guard::mrc_atr_stop::widen_stop(side, *price, raw_stop, snapshot.tf5m.atr)?;
    *stop_price = buffered_stop;
    Some(format!(
        "sl_buffer=atr5m({:.0}x) atr={:.8} raw={raw_stop:.8} effective={buffered_stop:.8}",
        crate::guard::mrc_atr_stop::ATR_MULTIPLIER,
        snapshot.tf5m.atr,
    ))
}

fn valid_entry_levels(reason: &EntryReason, direction: i8, tick_size: f64) -> bool {
    match reason {
        EntryReason::MrcCrossing {
            price,
            target_price,
            stop_price,
            pattern_kind,
            target_is_movement,
            ..
        } => {
            if *target_is_movement {
                mrc_crossing::valid_movement_price_levels(
                    direction,
                    *price,
                    *target_price,
                    *stop_price,
                    Some(tick_size),
                )
            } else {
                mrc_crossing::valid_price_levels_for_pattern(
                    direction,
                    *price,
                    *target_price,
                    *stop_price,
                    Some(tick_size),
                    mrc_pattern_classification::MrcPatternKind::from_str(pattern_kind),
                )
            }
        }
        EntryReason::CandleTrend {
            direction,
            price,
            target_price,
            stop_price,
            ..
        } => mrc_crossing::valid_price_levels_for_pattern(
            *direction,
            *price,
            *target_price,
            *stop_price,
            Some(tick_size),
            None,
        ),
    }
}
