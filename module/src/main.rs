use long_scalping::{engine, state};
use lte_strategy_bridge::run_loop;

fn main() {
    let mut state = state::EngineState::default();
    let mut restored = false;

    run_loop(|input| {
        if !restored {
            restored = state::restore(&mut state, input.state.as_ref());
        }
        let mut out = engine::run(&input, &mut state);
        out.state = state::save(&state);
        out
    });
}
