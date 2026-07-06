# Structural scalping handoff

Last updated 2026-07-06.

## Baseline

This repo now tracks the current release only.

- Rust crate: `module/`
- Runtime model: one position at a time
- Market states: `Long`, `Short`, `Sideways`
- Primary regime: `5m`
- Execution: `1m`
- Fallback confirmation: `15m`
- Context blocker: `1h`

## Config surface

The Start signal exposes two variables: `cap_init` and `trading_amount`.

- Type: percentage
- Range: `0` to `100`
- Meaning: seed the internal budget from `max_amount * cap_init / 100`
- `trading_amount`
- Default: `50`
- Range: `10` to `100`
- Meaning: normal entry size as a percent of the deployable budget (`budget × leverage`)
- Growth model: double the budget on profit until that cap is reached, then reinvest profit with leverage

## Rust layout

The module has been reset into a concern-based pipeline:

- `model.rs`: shared enums and structs
- `market_data.rs`: indicator and candle extraction
- `detection/mrc.rs`: band zone and slope
- `detection/cascade.rs`: candle-path movement tracking
- `detection/vlines.rs`: MRC crossing memory
- `detection/sharp_return.rs`: sharp move and fast return risk
- `detection/regime.rs`: `5m` classification plus `15m` fallback and `1h` blocker
- `decision.rs`: final market decision
- `execution.rs`: entry, stop, TP, cooldown, post-SL sizing
- `debug.rs`: compact runtime summary
- `engine.rs`: event routing

## Execution rules

- Flat state can open either a long or a short, never both.
- `Sideways` opens nothing.
- Existing positions are not reversed in the same tick.
- Exit priority is hard stop, then TP on pullback, then regime invalidation.
- After stop loss, the engine applies cooldown and reduced re-entry sizing.

## Validation

- `cargo check --target wasm32-wasip1`
- `cargo test --target x86_64-unknown-linux-gnu`

Both are expected to pass before build or deploy work.
