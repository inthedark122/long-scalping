use crate::model::PositionState;
use crate::state::EngineState;
use lte_strategy_bridge::abi::ModuleInput;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntrySizing {
    pub base_amount: f64,
    pub amount: f64,
    pub stop_loss_percent: Option<f64>,
    pub adjustment_factor: f64,
    pub target_percent: f64,
}

impl EntrySizing {
    pub fn diagnostics(self) -> String {
        let stop_loss = self
            .stop_loss_percent
            .map(|value| format!("{value:.4}%"))
            .unwrap_or_else(|| "n/a".to_string());
        format!(
            "risk_target={:.4}% sl_pct={} risk_factor={:.4} base_amount={:.4}",
            self.target_percent, stop_loss, self.adjustment_factor, self.base_amount
        )
    }
}

fn max_trade_notional(input: &ModuleInput) -> f64 {
    input.auto_max_amount.max(input.max_amount).max(0.0)
}

fn cap_budget(input: &ModuleInput, state: &EngineState) -> f64 {
    max_trade_notional(input) * state.cfg.cap_ratio()
}

fn min_notional(input: &ModuleInput) -> Option<f64> {
    input.symbol_info.min_notional.filter(|value| *value > 0.0)
}

fn seed_budget(input: &ModuleInput, state: &EngineState) -> f64 {
    let cap_budget = cap_budget(input, state);
    if cap_budget <= 0.0 {
        return 0.0;
    }
    let leverage = (input.leverage as f64).max(1.0);
    match min_notional(input) {
        Some(min_notional) => cap_budget.max(min_notional / leverage),
        None => cap_budget,
    }
}

pub fn entry_budget(input: &ModuleInput, state: &mut EngineState) -> f64 {
    if state.trade_budget <= 0.0 {
        state.trade_budget = seed_budget(input, state);
    }
    let max_budget = max_trade_notional(input).max(0.0);
    (state.trade_budget * state.cfg.trading_ratio()).min(max_budget)
}

fn current_position_margin(input: &ModuleInput, side: PositionState) -> f64 {
    let position = match side {
        PositionState::Long => input.positions.long.as_ref(),
        PositionState::Short => input.positions.short.as_ref(),
        PositionState::Flat => None,
    };
    let Some(position) = position else {
        return 0.0;
    };
    let leverage = (input.leverage as f64).max(1.0);
    let notional = if position.notional > 0.0 {
        position.notional
    } else if position.qty != 0.0 && position.entry_price > 0.0 {
        position.qty.abs() * position.entry_price
    } else {
        0.0
    };
    (notional / leverage).max(0.0)
}

fn stop_loss_percent(side: PositionState, entry_price: f64, stop_loss: f64) -> Option<f64> {
    if !entry_price.is_finite() || entry_price <= 0.0 || !stop_loss.is_finite() {
        return None;
    }
    let distance = match side {
        PositionState::Long => entry_price - stop_loss,
        PositionState::Short => stop_loss - entry_price,
        PositionState::Flat => return None,
    };
    let percent = distance / entry_price * 100.0;
    (percent.is_finite() && percent > 0.0).then_some(percent)
}

pub fn size_entry(
    input: &ModuleInput,
    state: &EngineState,
    side: PositionState,
    base_amount: f64,
    entry_price: f64,
    stop_loss: Option<f64>,
) -> Result<EntrySizing, &'static str> {
    if !base_amount.is_finite() || base_amount <= 0.0 {
        return Err("budget-too-small");
    }

    let target_percent = state.cfg.risk_target_percent();
    if target_percent <= 0.0 {
        return Ok(EntrySizing {
            base_amount,
            amount: base_amount,
            stop_loss_percent: None,
            adjustment_factor: 1.0,
            target_percent,
        });
    }

    let stop_loss_percent = stop_loss
        .and_then(|stop_loss| stop_loss_percent(side, entry_price, stop_loss))
        .ok_or("risk-adjustment-invalid-stop-loss")?;
    let adjustment_factor = (target_percent / stop_loss_percent).clamp(0.0, 1.0);
    if !adjustment_factor.is_finite() {
        return Err("risk-adjustment-invalid-factor");
    }

    // `base_amount` is the desired aggregate margin budget. Subtract any
    // already-open same-side margin so future add orders cannot exceed the
    // volatility-adjusted aggregate target.
    let aggregate_amount = base_amount * adjustment_factor;
    let remaining_amount = (aggregate_amount - current_position_margin(input, side))
        .max(0.0)
        .min(base_amount);

    Ok(EntrySizing {
        base_amount,
        amount: remaining_amount,
        stop_loss_percent: Some(stop_loss_percent),
        adjustment_factor,
        target_percent,
    })
}

pub fn apply_close_pnl(input: &ModuleInput, state: &mut EngineState, pnl: f64) {
    let cap_budget = cap_budget(input, state);
    let max_notional = max_trade_notional(input);
    let leverage = (input.leverage as f64).max(1.0);
    let seed = seed_budget(input, state);

    if state.trade_budget <= 0.0 && seed > 0.0 {
        state.trade_budget = seed;
    }

    if pnl > 0.0 {
        if state.trade_budget < cap_budget {
            state.trade_budget = (state.trade_budget * 2.0).min(cap_budget);
        } else {
            state.trade_budget = (state.trade_budget + pnl * leverage).min(max_notional);
        }
    } else if pnl < 0.0 {
        state.trade_budget = (state.trade_budget + pnl).max(0.0);
    }
}
