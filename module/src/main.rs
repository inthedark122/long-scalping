mod debug;
mod decision;
mod detection;
mod engine;
mod execution;
mod market_data;
mod model;
mod risk;
mod orders;
mod state;

use lte_strategy_bridge::run_loop;

fn main() {
    let mut state = state::EngineState::default();

    run_loop(|input| {
        let mut out = engine::run(&input, &mut state);
        out.state = state::save(&state);
        out
    });
}
