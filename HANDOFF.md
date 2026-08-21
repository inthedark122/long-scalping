# Adaptive pattern-engine handoff

Last updated 2026-08-17.

## Baseline

This repo tracks the current adaptive MRC pattern release only.

- Rust crate: `module/`
- Runtime model: one position at a time
- Input: 1m/5m MRC lines, volume, candles, and live price
- Classification: six active MRC patterns are evaluated for every observation
- Selection: the highest-scoring tradable staircase remains active until its protected structure ends
- Learning: each pattern stores observations, trades, wins, net PnL, and an EMA of realized net PnL
- Continuation: successful staircase legs advance to the next MRC line instead of stopping after one leg

## Config surface

The Start signal exposes sizing, exit, and execution controls. There is no manual
strategy-mode selector: the pattern engine is always active.

- Type: percentage
- Range: `0` to `100`
- Meaning: seed the internal budget from `max_amount * cap_init / 100`
- `trading_amount`
- Default: `50`
- Range: `10` to `100`
- Meaning: normal entry size as a percent of the deployable budget (`budget × leverage`)
- Growth model: double the budget on profit until that cap is reached, then reinvest profit with leverage

## Rust layout

- `detection/mrc_pattern_classification.rs`: stable pattern vocabulary and independent matchers
- `detection/mrc_crossing.rs`: candidate prediction, per-pattern learning, sticky selector, and entry plans
- `detection/mrc_cross_analysis.rs`: MRC line movement, target progression, and directional levels
- `state.rs`: event history, active selection, and realized performance ledger
- `engine.rs`: live event routing and execution integration
- `bin/research/mrc_crossing.rs`: parallel virtual-account research for every pattern
- `bin/research/mrc_crossing_html.rs`: adaptive selector and per-pattern report output
- `execution.rs` / `exit.rs`: exchange-safe one-position and exit mechanics

## Execution rules

- Flat state can open either a long or a short, never both.
- Only a selected staircase pattern can open a position.
- Pullback patterns remain visible to analysis and learning but cannot open positions.
- Existing positions are not reversed in the same tick.
- Staircase patterns can re-plan the next adjacent MRC line while the active movement remains valid.
- A realized trade result is attributed to the pattern that opened it.

## Validation

- `cargo test`
- `cargo test --features mrc-research,native-tools`

Both are expected to pass before build or deploy work.
