use crate::state::EngineState;
use lte_strategy_bridge::abi::ModuleInput;

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
