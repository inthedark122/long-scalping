mod consts;
mod debug;
mod engine;
mod indicators;
mod orders;
mod state;

use lte_strategy_bridge::run_loop;

fn main() {
    eprintln!("[module] started");

    let mut st = state::State::default();

    run_loop(|input| {
        eprintln!("[module] event: {:?}, price: {}", input.event, input.price);
        let mut out = engine::run(&input, &mut st);
        out.state = state::save(&st);
        eprintln!("[module] debug: {}", out.debug);
        out
    });
}
