use long_scalping::{engine, market_data, state};
use lte_strategy_bridge::{abi::ModuleEvent, run_loop};

const MODULE_STATE_ALIAS: &str = "structural_scalping";

fn main() {
    let mut state = state::EngineState::default();
    let mut restore_checked = false;
    let mut init_completed = false;
    let mut late_state_reported = false;

    run_loop(|input| {
        let mut diagnostics = Vec::new();
        if !restore_checked && !matches!(&input.event, ModuleEvent::Requirements) {
            restore_checked = true;
            if let Some(saved_state) = input.state.as_ref() {
                let saved_state = saved_state.get(MODULE_STATE_ALIAS).unwrap_or(saved_state);
                match state::restore_with_error(&mut state, Some(saved_state)) {
                    Ok(true) => diagnostics.push("state-restore=loaded".to_string()),
                    Ok(false) => diagnostics.push("state-restore=empty-payload".to_string()),
                    Err(error) => diagnostics.push(format!("state-restore-failed {error}")),
                }
            } else {
                diagnostics.push("state-restore=skipped-no-input-state".to_string());
            }
        } else if restore_checked && init_completed && input.state.is_some() && !late_state_reported
        {
            late_state_reported = true;
            diagnostics.push(format!(
                "state-restore=ignored-late-payload {}",
                market_data::render_kline_history_inventory(&state.klines)
            ));
        }

        let is_init = matches!(&input.event, ModuleEvent::Init);
        let mut out = engine::run(&input, &mut state);
        if is_init {
            init_completed = true;
        }
        if !diagnostics.is_empty() {
            if !out.debug.is_empty() {
                out.debug.push(' ');
            }
            out.debug.push_str(&diagnostics.join(" "));
        }
        out.state = state::save(&state);
        out
    });
}
