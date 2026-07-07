# Changelog

## 1.0.15

Timeout close guard release.

- Kept the timeout-close log expansion from `1.0.14`.
- Restricted created-order timeout closes to unfilled positions only (`qty == 0.0`).
- Preserved the live position snapshot in close logs for filled long and short positions.

## 1.0.14

Close-timeout visibility release.

- Expanded timeout-close logs to include the live position snapshot, raw and derived qty, entry price, TP/SL, exit mode, and start time.
- Kept the existing one-position timeout behavior unchanged.

## 1.0.13

Cascade TP release for research and testing.

- Raised the cascade movement TP anchor to the 90th percentile over a 30-sample window.
- Mirrored the percentile TP source in `movement_research` so replay/live testing matches the module.
- Kept the created-order timeout at `10_000 ms`.
- Prepared a fresh module build for bot-group testing.

## 1.0.12

Created-order timeout extension.

- Increased the created-position age timeout to `10_000 ms`.
- Switched cascade movement TP sizing to a 90th percentile over a 30-sample window.
- Kept the current structural scalping and sug-info movement flow unchanged.
- Prepared a new module release for the active bot groups.

## 1.0.11

Module release refresh.

- Bumped the structural scalping module to `1.0.11`.
- Kept the cascade research and sug-info movement pipeline intact for the next deploy.
- Updated the active bot groups to the new module version.

## 1.0.10

Created-order timeout and entry reason persistence.

- Stored the live entry reason on position runtime so created-entry timing can be tracked directly.
- Restored the created-order timeout backstop using the stored `started_at_ms` from the sug-info movement reason.
- Kept the exact-zero `qty` check so unfilled created positions are not confused with short quantities.

## 1.0.9

Movement entry alignment release.

- Folded movement-based entry back under `SugInfoMovement`.
- Kept the research-style movement open logic while preserving the existing regime gate.
- Kept TP/SL source debug annotations for active and fallback exit planning.

## 1.0.8

Fast TP/SL debug and movement release.

- Routed TP/SL planning through the fast movement cascade path first.
- Preserved structural TP/SL fallback values for grid and detection context.
- Added debug source labels so runtime output shows which exit source is active.

## 1.0.7

Module release refresh.

- Bumped the module crate and release metadata to `1.0.7`.
- Aligned the active bot configuration and module manifest template with the new version.
- Kept the current structural scalping runtime unchanged for this release.

## 1.0.6

SugInfo scalping default and movement research refresh.

- Made `sug_info.candle` the default scalping entry / exit path when the projection data is available.
- Kept regime and structure as a higher-level fallback for context.
- Added the movement research note and runnable replay/live script for 1m cascade analysis.
- Recorded the latest multi-symbol replay results for ETH, HYPE, LAB, and SLX.

## 1.0.5

Exit memory cleanup and debug trim.

- Removed the live `ExitTrace` object and replaced it with a compact persisted exit memory.
- Stopped printing trace deltas in debug output.
- Kept structural TP/SL planning and post-exit memory for re-entry gating.

## 1.0.4

Margin-sized entry and min-notional handling.

- Fixed initial budget math so `entry_budget()` stays at margin size instead of multiplying by leverage too early.
- Kept `trading_amount` as a guardrail for normal position sizing, with extra reserve only for specific multi-entry cases.
- Removed the hardcoded min-notional fallback so symbols without a declared minimum can still trade small sizes.

## 1.0.3

Fast-candle entry gating and exit-state tracking.

- Restored `fast_candle_entry_block` as a dedicated confirmation gate for first entry and continuation filtering.
- Kept entry wait policy layered on top of the FC gate with post-SL reclaim checks and TP reliability checks.
- Added stored TP/SL tracking in position runtime so the next tick can reason about the last exit cleanly.
- Preserved the one-position limit-only model and the current `cap_init` / `trading_amount` risk surface.

## 1.0.2

`cap_init` risk-model reset.

- Removed the public multi-variable sizing stack.
- Added a single Start parameter: `cap_init` as a percentage from `0` to `100`.
- Internal deployable budget now starts from `max_amount * cap_init / 100`.
- Profits double the internal budget until that cap is reached, then reinvest with leverage.
- Limit entries and the one-position trading model stay intact.

## 1.0.1

Limit-only entry release.

- Removed the last market-open helper from the active path.
- Initial entries now use `open_positions` with `Limit` order payloads only.
- Manual force-open actions were aligned to the same limit-entry flow.
- Bot-group config should now point at `1.0.1` for the LAB module repo.

## 1.0.0

Fresh baseline reset.

- Removed legacy branch history, retired execution paths, and compatibility config.
- Reduced the public config surface to `seed_pct`, `cap_pct`, `risk_pct`, `stop_pct`, `tp_pullback_pct`, `cooldown_ticks`, `confirm_tf_fallback`, and `debug_info`.
- Rebuilt the Rust module around a detector pipeline:
  - `market_data.rs`
  - `detection/mrc.rs`
  - `detection/cascade.rs`
  - `detection/vlines.rs`
  - `detection/regime.rs`
  - `decision.rs`
  - `execution.rs`
  - `debug.rs`
  - `engine.rs`
- Strategy inputs are now limited to `MRC`, `DPSignal`, `ZigZagTrend`, candle OHLC, and live price.
- Trading behavior is deterministic: exactly one market state, exactly one open position at a time, hard stop first, TP on pullback second, regime invalidation last.
