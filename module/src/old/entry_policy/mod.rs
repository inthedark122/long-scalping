mod gate;
mod notes;
mod state;
mod trend;

use crate::detection::mrc_crossing::Observation;
use crate::detection::{mrc_crossing, mrc_prediction};
use crate::model::{IndicatorSnapshot, MarketState, TradeDecision};
use crate::pattern_matching;
use crate::state::EngineState;
use lte_strategy_bridge::abi::{ModuleInput, ModuleMarketDecision};

pub struct Selection {
    pub decision: TradeDecision,
    pub debug: String,
}

pub fn evaluate(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    market_decision: &ModuleMarketDecision,
    observation: &Observation,
    prediction_context: mrc_prediction::MrcPredictionContext,
) -> Selection {
    let now_ms = market_decision.time.max(0) as u64;
    let trend = trend::evaluate(input, state, snapshot, now_ms);
    let pattern = pattern_matching::evaluate(state.klines.get(crate::market_data::TF_1M), now_ms);
    let selected_analysis = observation.selected_analysis();
    let raw_direction = observation
        .entry
        .as_ref()
        .map(mrc_crossing::reason_direction)
        .or_else(|| {
            state
                .mrc_crossing_model
                .current_entry
                .as_ref()
                .map(mrc_crossing::reason_direction)
        });
    let trend_entry_selected = trend::trend_entry_selected(&trend, state, raw_direction);
    trend::update_seen_state(state, &trend);

    let selected_pattern = observation
        .selected_pattern
        .map(|pattern| pattern.as_str().to_string())
        .unwrap_or_else(|| {
            observation
                .wait_reason
                .map(|reason| format!("wait:{reason:?}"))
                .unwrap_or_else(|| "no-candidate".to_string())
        });
    state::update(
        state,
        selected_analysis,
        &selected_pattern,
        prediction_context,
        trend_entry_selected,
        &trend.candle.summary,
        &pattern.summary,
    );

    let market_state = entry_market_state(state, trend_entry_selected, &trend, raw_direction);
    let mrc_gate = gate::build_mrc_entry_gate(
        input,
        state,
        snapshot,
        observation,
        selected_analysis,
        market_state,
        &pattern,
    );
    let entry_gate = if trend_entry_selected {
        gate::build_candle_trend_gate(
            input,
            trend.candle.entry.clone(),
            &trend.candle.summary,
            &pattern,
        )
    } else {
        mrc_gate
    };
    let debug = format!(
        "{} {} enabled={} long_allowed={} short_allowed={} trend_direction={:?} sustained_trend={:?} trend_entry_direction={:?} spike_direction={:?} mrc_candidate={:?} mrc_selected={:?} mrc_direction={:?} up_seen={} down_seen={} last_closed_ms={} latest_closed_ms={} pending_entry={} trend_entry_selected={}",
        trend.candle.summary,
        pattern.summary,
        trend.enabled,
        trend.long_allowed,
        trend.short_allowed,
        trend.trend_direction,
        trend.sustained_direction,
        trend.entry_direction,
        trend.cascade_spike_direction,
        raw_direction,
        observation.selected_pattern,
        raw_direction,
        state.candle_uptrend_seen,
        state.candle_downtrend_seen,
        state.candle_trend_last_closed_ms,
        trend.candle.latest_closed_bar_end_ms.unwrap_or_default(),
        trend.pending_entry,
        trend_entry_selected,
    );

    Selection {
        decision: TradeDecision {
            market_state,
            entry: entry_gate,
        },
        debug,
    }
}

fn entry_market_state(
    state: &EngineState,
    trend_entry_selected: bool,
    trend: &trend::TrendContext,
    mrc_direction: Option<i8>,
) -> MarketState {
    if trend_entry_selected {
        return if trend.entry_direction == Some(-1) {
            MarketState::Short
        } else {
            MarketState::Long
        };
    }
    mrc_direction
        .filter(|direction| *direction != 0)
        .map(mrc_crossing::market_state)
        .unwrap_or_else(|| mrc_crossing::market_state(state.mrc_crossing_model.last_direction))
}
