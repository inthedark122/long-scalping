# Long Scalping

The current experiment prioritizes capturing repeated moves inside sideways ranges. Existing market analysis detects the regime and Movement Cascade supplies the recent range low and high. Each strategy owns its entry and exit rules under `module/src/strategies/`.

## Strategy routing

`module/src/strategies/mod.rs` routes from the existing candle pattern analysis and confirms a sideways range with five recent movements and four direction changes:

- **Sideways** — active when pattern matching reports `MEAN_RANGE` and Movement Cascade confirms a range. In the lower half of the window, an upward cascade movement places a buy limit at the window low. In the upper half, a downward cascade movement places a sell limit at the window high. Either entry targets the opposite boundary; its stop sits one average Movement Cascade movement beyond the entry boundary, with one symbol tick as the minimum distance. The strategy allows one entry attempt per active cascade movement and cancels a pending order when price leaves the window.
- **Trending** — the existing MRC and staircase entry logic, with its MRC based trailing stop. It handles entries when the market is not routed to Sideways.

When Sideways takes over while a Trending position is open, it replaces that position's TP with the opposite range boundary and puts the stop one average cascade movement beyond the near boundary while price is inside the window. The position then remains managed by Sideways until it closes.

This is an experiment in range-rebound trading, not a claim that the rules are profitable. The backtest should show whether repeated range trades earn enough to offset losses when the market trends or breaks out of the detected window.

## Code ownership

- `module/src/detection/` calculates shared market facts, including the Movement Cascade window. It does not place orders.
- `module/src/pattern_matching/` identifies the existing candle pattern used to distinguish `MEAN_RANGE` from staircase and other patterns.
- `module/src/strategies/trending/` owns the current trending entry and exit rules.
- `module/src/strategies/sideways.rs` owns range-rebound entry selection and range-boundary TP/SL levels.
- `module/src/strategies/execution.rs` applies shared sizing and translates a strategy plan into the module order API.
- `module/src/engine.rs` updates analysis, routes the decision, and coordinates strategy execution.

MRC analysis remains available to shared debug output and to the trending strategy. The sideways strategy uses the detected Movement Cascade window for its entry and exit levels rather than MRC lines.

## Risk sizing

The public sizing parameters are `cap_init`, `trading_amount`, and the optional `risk_target_percent`. `cap_init` seeds the internal budget as a percentage of `max_amount`. `trading_amount` caps normal entry size as a percentage of the deployable budget (`budget × leverage`). A positive `risk_target_percent` reduces entry size when the stop distance exceeds that target; `0` disables that adjustment. Profits grow the internal budget automatically.

## MRC research

Historical pattern classification and per-pattern trade reports can be run with:

```bash
cargo run --manifest-path module/Cargo.toml --locked --features mrc-research,native-tools --bin mrc_crossing_research -- --symbol-key 'OKX#CAP-USDT-SWAP#SWAP' --symbol-key 'OKX#BEAT-USDT-SWAP#SWAP' --symbol-key 'OKX#ETH-USDT-SWAP#SWAP' --history 10000 --output tmp/reports/ --html tmp/reports/
```
