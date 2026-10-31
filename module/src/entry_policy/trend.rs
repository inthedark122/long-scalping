use crate::detection::{candle_trend, cascade};
use crate::model::{IndicatorSnapshot, PositionState};
use crate::state::EngineState;
use lte_strategy_bridge::abi::{Direction, ModuleInput};

pub struct TrendContext {
    pub candle: candle_trend::Evaluation,
    pub enabled: bool,
    pub trend_direction: Option<i8>,
    pub sustained_direction: Option<i8>,
    pub entry_direction: Option<i8>,
    pub cascade_spike_direction: Option<i8>,
    pub long_allowed: bool,
    pub short_allowed: bool,
    pub pending_entry: bool,
    pub trend_seen: bool,
    pub direction_allowed: bool,
}

pub fn evaluate(
    input: &ModuleInput,
    state: &mut EngineState,
    snapshot: &IndicatorSnapshot,
    now_ms: u64,
) -> TrendContext {
    let candle = candle_trend::evaluate(
        state.klines.get(crate::market_data::TF_1M),
        now_ms,
        input.price,
        snapshot.tf1m.mrc.lines(),
        input.symbol_info.price_tick_size,
    );
    let trend_enabled = state.mrc_crossing_model.decision_components.support_trend;
    let trend_direction = candle
        .trend
        .map(|trend| trend.direction)
        .filter(|direction| matches!(direction, 1 | -1));
    let latest_closed_ms = candle.latest_closed_bar_end_ms;
    let newer_closed_candle =
        latest_closed_ms.is_some_and(|time_ms| time_ms > state.candle_trend_last_closed_ms);

    if let Some(time_ms) =
        latest_closed_ms.filter(|time_ms| *time_ms > state.candle_trend_last_closed_ms)
    {
        state.candle_trend_last_closed_ms = time_ms;
    }
    if newer_closed_candle {
        let observed_direction = trend_direction.unwrap_or_default();
        if observed_direction != state.candle_trend_direction {
            state.candle_uptrend_seen = false;
            state.candle_downtrend_seen = false;
            state.candle_trend_direction = observed_direction;
        }
    }

    let cascade_spike_direction = match state.cascade_signal.phase {
        cascade::MovementPhase::Reversal if matches!(state.cascade_signal.prev_dir, 1 | -1) => {
            Some(state.cascade_signal.prev_dir)
        }
        _ if state.cascade_signal.stretched && matches!(state.cascade_signal.dir, 1 | -1) => {
            Some(state.cascade_signal.dir)
        }
        _ => None,
    };
    let long_allowed = state.allowed_direction != Direction::Short
        && state.mrc_crossing_model.trade_direction_lock != Some(-1);
    let short_allowed = state.allowed_direction != Direction::Long
        && state.mrc_crossing_model.trade_direction_lock != Some(1);
    let snapshot_current =
        latest_closed_ms.is_some_and(|time_ms| time_ms >= state.candle_trend_last_closed_ms);
    let sustained_direction = trend_direction.filter(|direction| {
        trend_enabled
            && snapshot_current
            && match direction {
                1 => long_allowed,
                -1 => short_allowed,
                _ => false,
            }
    });
    let entry_direction = sustained_direction.filter(|_| cascade_spike_direction.is_none());
    let pending_entry = state.pending_entry_reason.is_some() || state.pending_entry_plan.is_some();
    let direction_allowed = match entry_direction {
        Some(1) => long_allowed,
        Some(-1) => short_allowed,
        _ => false,
    };
    let trend_seen = match entry_direction {
        Some(1) => state.candle_uptrend_seen,
        Some(-1) => state.candle_downtrend_seen,
        _ => false,
    };

    TrendContext {
        candle,
        enabled: trend_enabled,
        trend_direction,
        sustained_direction,
        entry_direction,
        cascade_spike_direction,
        long_allowed,
        short_allowed,
        pending_entry,
        trend_seen,
        direction_allowed,
    }
}

pub fn trend_entry_selected(
    trend: &TrendContext,
    state: &EngineState,
    mrc_entry_direction: Option<i8>,
) -> bool {
    let Some(direction) = trend.entry_direction else {
        return false;
    };
    state.position.side == PositionState::Flat
        && trend.direction_allowed
        && !trend.pending_entry
        && !trend.trend_seen
        && mrc_entry_direction != Some(direction)
        && trend.candle.entry.is_some()
}

pub fn update_seen_state(state: &mut EngineState, trend: &TrendContext) {
    let Some(direction) = trend.sustained_direction else {
        return;
    };
    let follows = matches!(
        (state.position.side, direction),
        (PositionState::Long, 1) | (PositionState::Short, -1)
    );
    if follows {
        match direction {
            1 => state.candle_uptrend_seen = true,
            -1 => state.candle_downtrend_seen = true,
            _ => {}
        }
    }
}
