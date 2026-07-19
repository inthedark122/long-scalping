mod builder;
mod debug;
mod detection;
mod engine;
mod execution;
mod exit;
mod format;
mod market_data;
mod martingale;
mod model;
mod observer;
mod order_manager;
mod orders;
mod prediction;
mod risk;
mod risk_control;
mod state;
mod stop_loss;

use lte_strategy_bridge::run_loop;

fn main() {
    let mut state = state::EngineState::default();

    run_loop(|input| {
        let mut out = engine::run(&input, &mut state);
        out.state = state::save(&state);
        out
    });
}
