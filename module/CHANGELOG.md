# Changelog

# 1.0.70

Entry and active-exit protection release.

- Keep configured/cascade TP minimums during active TP recalculation until the virtual SL is crossed.
- Skip opposite-side MRC levels and the MRC mean line when selecting structural stop losses.
- Expose reason, submitted order, and market prices in entry notes for fill tracing.

# 1.0.69

Active TP recalculation release.

- Recalculate the active-position TP once per completed 1-minute bar.
- Reuse the existing TP calculation and order replacement behavior.

# 1.0.67

Exit diagnostics release.

- Track MRC reverts independently for long and short pullbacks.
- Add a configurable minimum structural TP and use the larger of it and the cascade average.
- Move exit helpers into the exit/stop-loss modules and centralize tick-size handling.
- Treat unavailable `BuySellGain` as zero and remove obsolete sticky MRC crossing extremes.

# 1.0.66

Pending-entry protection release.

- Keep unfilled MRC limit entries active when the next signal enters a wait state.
- Reprice pending entries only when a later open decision changes the entry price.
- Remove pending-cancellation debug state and preserve the existing entry reason.

# 1.0.65

Directional entry release.

- Follow an active directional lock when selecting the entry side.
- Expose the selected entry side in debug output.

# 1.0.64

Martingale SL activation release.

- Reserve an additional full martingale range before SL activation.
- Keep the reserved activation price separate from the common level-based SL.
- Expose the martingale SL activation price in debug output.

# 1.0.63

Deduplicated MRC crossing diagnostics release.

- Ignore repeated touches of the current MRC line in the crossing sequence.
- Correctly identify pullback sequences such as A -> B -> A.
- Compact and bracket MRC crossing debug output.

# 1.0.62

Realtime MRC crossing diagnostics release.

- Track realtime MRC crossing direction, repeat counts, sequences, pullback-free crossings, and reverted lines.
- Track favorable movement percentage from each crossed line and expose the metrics in debug output.

# 1.0.61

Cascade debug release.

- Expose the active cascade movement percentage and current position within that movement in debug output.

# 1.0.60

Cascade-minimum TP release.

- Select the next structural TP level only when it reaches the 1-minute cascade average percentage.
- Keep cascade percentage calculation shared between TP selection and debug output.
- Preserve existing TP behavior after the virtual-SL transition.

# 1.0.59

Movement-history entry pricing release.

- Removed Sug cascade tracking and kept a single timeframe cascade.
- Limited cascade direction use to explicit movement-plan direction selection.
- Added bounded completed-movement history for movement-based entry pricing.
- Used previous movement ranges and current movement delta without fallback prices.
- Renamed the MRC pullback wait reason to `mrc-pullback-wait`.

# 1.0.58

Configurable TP-control release.

- Added Normal and Trailing stop TP-control settings.
- Added favorable-only virtual stop trailing with a fixed two-tick recovery TP after an SL cross.
- Removed runtime TP-mark state and use deterministic direction-based TP marks.

# 1.0.57

Informative stop-loss release.

- Represented stop-loss plans as typed structural, sideway, support-break, or martingale values.
- Included the actual stop-loss source and level details in active-position debug output.

# 1.0.56

Direction trace release.

- Record the actual direction decision source and runtime reason in debug output.
- Include direction-change window percentages with tick-accurate prices.

# 1.0.55

Short-position close fix release.

- Use the absolute raw position quantity when creating close orders so short positions are fully closed.

# 1.0.54

Test coverage release.

- Fixed pending-position cancellation test coverage without changing production evaluation logic.

# 1.0.53

Typed take-profit release.

- Represented take-profit plans as typed structural, movement, sideway, or martingale values.
- Included the selected structural level name in TP order notes.
- Centralized TP order-note rendering and removed duplicated structural TP metadata.

# 1.0.52

Execution safety release.

- Disabled the SugInfo exit pattern so exits use movement, sideway, or structural plans.
- Removed the unused cascade argument from execution evaluation.
- Returned missing execution results as errors in the test evaluation helper instead of panicking.

# 1.0.51

Debug and execution fixes release.

- Consolidated entry-wait output into one concise execution line with tick-formatted prices.
- Removed redundant cascade debug fields and wait-note values.
- Reset TP repricing history after flat positions and include the current history in TP replacement order notes.

# 1.0.50

Strategy fixes release.

- Removed obsolete entry timing metadata and contextual entry reasons.
- Fixed forced pullback orders to use the planned MRC entry price.
- Restored direction-change closes for Single mode and fixed indicator/test coverage.

# 1.0.49

Execution note cleanup release.

- Kept blocker reasons explicit for entry refusals while removing the exit-helper constraint transport.
- Moved fast-candle waits to a concrete fast-candle solution and removed the placeholder cooling variant.
- Kept order and position notes on the actual trade objects instead of duplicating execution note text.

# 1.0.48

Debug formatting release.

- Centralized formatting helpers so prices, quantities, and durations render consistently across debug output.
- Removed duplicated position fields from the main debug line and kept only structured wait-gate tracking when the engine is blocked.
- Trimmed timeout noise and kept the direction history summary readable while preserving the underlying execution behavior.

# 1.0.47

Direction history observer release.

- Moved direction-change tracking into a dedicated observer module so runtime state stays simpler.
- Recorded direction-change intervals, rolling averages, and per-change min/max prices for debug tracing.
- Kept the existing entry and exit behavior unchanged while surfacing the new history summary in the debug line.

# 1.0.46

MRC window release.

- Reworked pullback entries to use real MRC line windows instead of a full-band percentage fallback.
- Classified price into 1-line and 2-line MRC zones so pullback windows follow adjacent structural lines.
- Removed the synthetic price-based fallback from the pullback wait path.

# 1.0.45

Init snapshot fix release.

- Built init snapshots from indicators only so timeframe close values no longer depend on live price.
- Kept partial indicator updates and cached realtime snapshot reuse unchanged.
- Preserved the validation and raw inventory tracing added in prior releases.

# 1.0.44

Partial indicator merge release.

- Merged partial indicator updates onto the cached snapshot so `Indicators { timeframes }` no longer requires a full rebuild from raw payloads.
- Kept the bridge history update path intact and reused the last full snapshot for `SugInfo` processing.
- Preserved the indicator validation and raw inventory tracing added in the previous release.

## 1.0.43

Trace context release.

- Added event and timestamp context to snapshot error lines so empty raw payloads can be traced back to `Init`, `Indicators`, or `SugInfo`.
- Kept the raw indicator inventory in the error output so missing bridge fields are visible immediately.
- Preserved the current snapshot validation behavior for valid indicator payloads.

## 1.0.42

Indicator inventory error release.

- Removed silent zero fallbacks from raw indicator loading so missing fields now fail fast.
- Added raw indicator inventory details to debug errors for quick inspection of what data is present.
- Kept the existing snapshot validation and strategy flow unchanged for valid inputs.

## 1.0.41

Indicator validation guard release.

- Added a hard snapshot validator that stops processing when timeframe directions or structural inputs are missing.
- Surfaced validation failures directly in the debug line so bad snapshots are visible before strategy evaluation.
- Kept the probe-aligned entry flow and structural level handling intact.

## 1.0.40

Direction probe parity release.

- Tiered structural levels by timeframe instead of flattening them into a single clustered band.
- Kept nearby low-TF levels visible so low-priced symbols do not inherit a far 1h stop.
- Matched the probe's intended entry / TP / SL references more closely and removed same-price break anchors.

## 1.0.39

Typed order status release.

- Switched martingale rollover handling to the bridge's typed order status enum.
- Replaced the string-based fill heuristic with exact `Filled` matching.
- Kept the martingale replenishment flow limited to the next rung only.

## 1.0.38

Builder split release.

- Split the strategy builder into dedicated initial, shared, single-mode, and martingale files.
- Kept the runtime builder API stable while moving mode-specific logic out of the monolith.
- Updated the release metadata and bot strategy version to the new module version.

## 1.0.37

Martingale recovery TP release.

- Advanced the martingale recovery take-profit to current price plus 2 ticks after virtual SL hit.
- Kept single-mode virtual SL recovery on the breakeven TP reclaim path.
- Updated the module and strategy release metadata to point at the new version.

## 1.0.36

Martingale range and debug fix release.

- Derived the martingale ladder range from candle range instead of MRC range.
- Made the debug line label martingale exits explicitly when martingale profile data is present.
- Updated the release metadata to point at the new module version.

## 1.0.35

Martingale range fix release.

- Switched martingale stop math from MRC range to the martingale ladder width.
- Persisted the martingale profile on the open position so exit recalculation uses the same range.
- Updated the release metadata to point at the new module version.

## 1.0.34

Martingale sizing fix release.

- Kept only the 2 nearest resting martingale orders instead of emitting the full ladder.
- Resized the legs to a 25% / 25% / 50% split so the immediate open is not tiny.
- Updated the release metadata to point at the new module version.

## 1.0.33

Martingale ladder fix release.

- Replaced the single extra martingale order with a full 2x ladder across the calculated levels.
- Kept the immediate market open leg and the sideway block-edge exit logic intact.
- Updated the release metadata to point at the new module version.

## 1.0.32

Martingale stop and debug fix release.

- Collapsed the martingale debug output to a single effective TP/SL pair.
- Tightened sideway martingale exits so the stop stays near the sideway block instead of using the full MRC range buffer.
- Kept the immediate martingale open behavior from the previous release.

## 1.0.31

Immediate martingale open release.

- Made martingale entries create a live `Market` open for the first leg instead of only placing resting ladder orders.
- Kept the follow-up ladder order and structural stop-loss logic intact.
- Updated the strategy metadata to point at the new module version.

## 1.0.30

Martingale debug cleanup release.

- Removed the temporary raw trade-mode debug field.
- Kept numeric `trade_mode` parsing for the martingale switch.
- Bumped the release metadata to a fresh module version.

## 1.0.29

Martingale trade mode release.

- Added a `trade_mode` switch with `single` and `martingale` options.
- Kept the current single-order cascade path intact.
- Added a martingale ladder planner with step, TP, and SL controls derived from live range.
- Exposed the martingale mode in the strategy start action and module debug output.

## 1.0.25

Forced pullback gate bypass release.

- Bypassed the `fast-candle-wait` and `one-way-lock` entry waits when pullback mode is enabled.
- Kept the sug-info cascade pullback logic and 25% pullback budget intact.
- Preserved the normal release path for non-pullback entries.

## 1.0.26

Forced pullback cascade bypass fix.

- Allowed forced pullback entries to bypass the 1m cascade direction/confirmation wait.
- Preserved the existing pullback budget and debug markers so the open path stays visible.
- Kept normal entries unchanged outside pullback mode.

## 1.0.27

Sideway entry reason fix.

- Promoted active sideway signals into a real `SidewayScalp` entry reason even when the run is not coming from stop-loss recovery.
- Allowed bullish `LineCross` sideway signals to open long continuation entries instead of staying stuck in `market_state=Sideways`.
- Kept the forced pullback behavior and the existing long-only runtime path intact.

## 1.0.28

LineCross pullback discipline release.

- Required active `LineCross` sideway longs to wait for a pullback cap derived from 60% of the 1m cascade average.
- Kept sug-info trading as the execution path, but stopped buying far above the sideway block.
- Added a regression test so `LineCross` only opens after the pullback threshold is satisfied.

## 1.0.24

Forced pullback entry release.

- Added a separate `pullback_amount` knob to force sug-info pullback entries at 25% of the normal deployable budget.
- Kept the normal `trading_amount` default at 50% so regular entries remain unchanged.
- Converted the sug-info pullback wait branch into a forced open when the new pullback amount is enabled.

## 1.0.23

Default entry-size reduction release.

- Lowered the compiled default `trading_amount` from 50% to 25% so new bots start with smaller orders.
- Kept the sug-info pullback entry logic unchanged.
- Preserved the WASM-only release split from the native helper binaries.

## 1.0.22

WASM entrypoint safety release.

- Kept the trading module entrypoint isolated from the native-only probe and replay binaries.
- Gated `direction_probe` and `movement_research` behind a native tools feature so the WASM release path only contains `module_entry`.
- Prevented host command usage from leaking into the deployed module artifact.

## 1.0.21

Long direction probe release.

- Added a standalone Rust probe binary to inspect direction, entry readiness, and expected exit levels.
- Reworked the long regime to accept price above frozen `Zee1` with `ZigZag` `BOTH` as valid bullish confirmation.
- Kept the strategy long-only while preserving the 1m cascade and sug-info entry stack.

## 1.0.20

Stale lock release.

- Cleared the frozen direction lock when higher-timeframe confirmation falls back to sideways.
- Restored 1m pullback entry flow after neutral conditions instead of keeping a stale one-way bias alive.
- Kept explicit-entry protection so reasonless regime opens remain blocked.

## 1.0.19

Explicit-entry release.

- Blocked regime opens that do not carry an explicit entry reason.
- Kept `sug_info` scalp, movement, and stop-loss reversal paths as the only valid opens.
- Stopped the worst-case `reason=none` entry path from reaching live orders.

## 1.0.18

Clear entry solution release.

- Made `sug_info` the primary scalp entry path and required 1m confirmation for the live open.
- Kept movement as a fallback and reserved sideways for stop-loss-driven reversal entries.
- Added a dedicated `SugInfoScalp` entry reason so the runtime and debug output show the new primary path.

## 1.0.17

Created-entry amend release.

- Replaced created-position timeout closes with in-place entry-price updates.
- Added `update_positions` plumbing so pending entries amend without cancel+replace.
- Kept direction changes as a hard close trigger for active positions.

## 1.0.16

Sideway scalp release.

- Added a live contrarian sideway entry path that opens on the opposite block edge.
- Routed sideway entries into dedicated entry reasons and debug output.
- Tied sideway TP/SL planning to the detected block bounds instead of generic structural fallback.

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
