use crate::exit;
use crate::model::{ExitKind, FastCandlePhase, IndicatorSnapshot, PositionState};
use crate::state::EngineState;

fn candle_range_pct(candle: &crate::model::Candle) -> f64 {
    if candle.close <= 0.0 {
        return 0.0;
    }
    ((candle.high - candle.low).abs() / candle.close) * 100.0
}

fn candidate_exit_plan(
    snapshot: &IndicatorSnapshot,
    price: f64,
    side: PositionState,
) -> Option<crate::exit::ExitPlan> {
    exit::plan(snapshot, price, side)
}

pub fn fast_candle_entry_block(snapshot: &IndicatorSnapshot) -> Option<String> {
    let candle_pct = candle_range_pct(&snapshot.tf1m.candle);
    let small_candle = candle_pct <= 0.40;
    let fc_phase = snapshot
        .fast_candle
        .map(|fc| fc.phase(&snapshot.tf1m.candle))
        .unwrap_or(FastCandlePhase::Absent);

    match fc_phase {
        FastCandlePhase::Building | FastCandlePhase::Tiring => Some(format!(
            "fc-phase={:?} fc={:.2}% candle={:.2}% gain_zone={:?} gain_prog={:.2}",
            fc_phase,
            snapshot.fast_candle.map(|fc| fc.range_pct()).unwrap_or(0.0),
            candle_pct,
            snapshot.tf1m.gain_zone(),
            snapshot.tf1m.gain_progress_pct()
        )),
        FastCandlePhase::Cooling if !small_candle => Some(format!(
            "fc-phase=Cooling candle-not-small fc={:.2}% candle={:.2}% gain_zone={:?} gain_prog={:.2}",
            snapshot.fast_candle.map(|fc| fc.range_pct()).unwrap_or(0.0),
            candle_pct,
            snapshot.tf1m.gain_zone(),
            snapshot.tf1m.gain_progress_pct()
        )),
        _ => None,
    }
}

pub fn entry_wait_reason(
    snapshot: &IndicatorSnapshot,
    state: &EngineState,
    side: PositionState,
) -> Option<String> {
    if let Some(reason) = fast_candle_entry_block(snapshot) {
        return Some(reason);
    }

    if matches!(state.last_exit_kind, Some(ExitKind::StopLoss))
        && state.last_exit_side == side
        && state.last_exit_price > 0.0
    {
        let price = snapshot.tf1m.candle.close;
        let reclaimed = match side {
            PositionState::Long => price >= state.last_exit_price,
            PositionState::Short => price <= state.last_exit_price,
            PositionState::Flat => true,
        };
        if !reclaimed {
            return Some(format!(
                "post-sl-wait failed_price={:.4} current={:.4}",
                state.last_exit_price, price
            ));
        }
    }

    let plan = candidate_exit_plan(snapshot, snapshot.tf1m.candle.close, side)?;
    let candle_pct = candle_range_pct(&snapshot.tf1m.candle);
    let small_candle = candle_pct <= 0.40;
    let fc_phase = snapshot
        .fast_candle
        .map(|fc| fc.phase(&snapshot.tf1m.candle))
        .unwrap_or(FastCandlePhase::Absent);

    if !plan.tp_reliable && !(matches!(fc_phase, FastCandlePhase::Cooling) && small_candle) {
        return Some(format!(
            "tp-unreliable tp={:.4} sl={:.4}",
            plan.take_profit, plan.stop_loss
        ));
    }

    if matches!(state.last_exit_kind, Some(ExitKind::TakeProfit))
        && state.last_exit_side == side
        && !state.last_exit_tp_reliable
    {
        return Some(format!(
            "post-tp-tight tp={:.4} sl={:.4}",
            plan.take_profit, plan.stop_loss
        ));
    }

    None
}
