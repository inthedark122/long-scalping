# Changelog

# Unreleased

# 1.0.115

ta-rs kline data pipeline release.

- Request 1m and 5m kline history through ABI v2 `Requirements` and calculate
  MRC-200 and Volume-50 locally from `KlineUpdate` data with ta-rs.
- Ignore platform indicator events; keep kline history and calculated values
  transient instead of serializing them into module output state.

# 1.0.114

Market-decision ABI compatibility release.

- Migrate the strategy bridge from `SugInfo` to the public `MarketDecision`
  contract, mapping `ntp_5s` and `tdt` into the existing activity context.
- Persist the latest market-decision timestamp so position and order events
  retain timing context when the ABI omits the snapshot from those events.

# 1.0.113

Open-position decision controller release.

- Keep the current market route for future entries separate from the same-side
  route and exit plan owned by an open position.
- Add one-shot and recyclable-stairs position lifetimes with bounded re-entry
  state and persisted position context.
- Reprice exits from the active position context, retain the last valid plan as
  fallback, and close explicitly when the same-side staircase is invalidated or
  no valid exit plan remains.
- Expose compact position-control action, exit-source, future-route, lifetime,
  re-entry, and reason diagnostics.

# 1.0.112

Exit diagnostics release.

- Add concise active-position exit events for runtime rebuilds, missing or
  invalid exit plans, TP submission, virtual-SL crossings, repricing, and
  stalled/no-change decisions.
- Label the internal protective level as `vsl` in runtime debug output and
  include the server-position synchronization context when the in-memory
  position runtime is rebuilt.
- Keep strategy behavior unchanged; this release improves operational
  observability only.

# 1.0.111

Staircase transition identity release.

- Remove reversal as a primary MRC pattern and carry it as a potential or
  confirmed transition on the underlying `STAIRS_UP` or `STAIRS_DOWN` route.
- Keep reversal routes additive without filtering valid staircase continuation
  entries, while preserving their transition state and protected-swing barrier.
- Attribute transition trades to the underlying staircase performance ledger
  and expose the separation in research JSON/HTML reports.

Imperative active MRC route TP.

- Build route-level favorable/adverse movement fields for every active 1m MRC
  line plan.
- Scan all supported forward lines and select the maximum currently reachable
  movement using historical route evidence and live cascade interest.
- Recalculate the selected movement from the latest live staircase analysis and
  amend the existing reduce-only TP order while keeping the filled entry as
  the price anchor.

# 1.0.110

Imperative active MRC route TP release.

- Build route-level favorable/adverse movement fields for every active 1m MRC
  line plan.
- Scan all supported forward lines and select the maximum currently reachable
  movement using historical route evidence and live cascade interest.
- Recalculate the selected movement from the latest live staircase analysis and
  amend the existing reduce-only TP order while keeping the filled entry as
  the price anchor.

# 1.0.109

Entry-relative movement TP release.

- Calculate the primary TP from the selected entry price and learned favorable movement instead of targeting an MRC line.
- Keep MRC target lines as route/reference analysis and keep the protective stop tied to the MRC structure.
- Anchor live TP recalculation to the filled entry price so the movement target does not drift with the market.
- Remove the automatic far structural MRC TP for movement-target entries.
- Expose the movement target source in runtime debug output and research JSON/HTML reports.

# 1.0.108

Stair entry-quality release.

- Add a dedicated staircase entry-state layer for pullback, retest, continuation, top, and risk-rejected states.
- Keep STAIRS_UP/STAIRS_DOWN as the structural direction authority while using current MRC touches, debounce, movement state, 5m context, and fresh-extreme timing to select safer limit entries.
- Prevent top-of-spike entries and reject stalled, adverse, or reversal states instead of chasing an extreme.
- Expose the selected stair entry state, reason, and line in analysis and runtime debug output.
- Local 10,000-sample replay: CAP 128 positions, BEAT 138, ETH 137; all three replay nets improved versus the previous local reports.

# 1.0.107

Structural continuation exit release.

- Remove cascade-staged entry orders and dynamic cascade-driven multi-TP switching.
- Keep one primary TP with an optional fixed-size far structural TP selected from MRC lines and staircase extremes.
- Use MRC movement time, cascade duration averages, staircase extreme-update timing, NTPS growth, volume growth, and no-pullback activity to detect stalled continuation.
- Keep cascade as timing/activity context only and safely handle exchange quantity steps and small final remainders.

# 1.0.106

Empirical MRC touch-learning release.

- Learn next-line direction from exact and crossing MRC touch episodes instead of weighted direction formulas.
- Tag touch history by 5m MRC slope/price-side context and fast/normal/slow touch intervals.
- Persist touch-learning state and expose learned line, samples, scope, timeframe, and reliability in debug output.
- Replay note: CAP net -25.923%, BEAT net -47.194%, ETH net -28.937% on cached 10k windows; ENA replay unavailable. Released at user request for live observation.

# 1.0.105

Incremental MRC-touch direction prediction release.

- Use price position, touch/debounce, EMA speed, angle, and MRC band-width movement as a continuous up/down/sideways prediction signal.
- Combine current MRC-touch prediction with learned route outcomes through expected-value entry scoring.
- Keep cascade as movement context instead of using it as the primary direction authority.
- Expose the prediction state, probabilities, confidence, and candidate edge in debug output.

# 1.0.104

Incremental MRC touch analysis and pattern cleanup.

- Add EMA-based MRC touch, debounce, speed, angle, interest, and band-width pressure signals for staircase scoring.
- Warm the touch analyzer from historical MRC samples and expose its diagnostics in realtime debug output.
- Remove the standalone inner-pullback pattern from classification, state, reports, history, and documentation.

# 1.0.103

Take-profit allocation fix.

- Keep partial TP quantities anchored to the calculated entry allocation instead of re-splitting the shrinking live position.
- Track cumulative fills by TP mark and cap regenerated orders safely when a fill-stage update is missing.
- Allocate the final remainder after quantity rounding and merge dust below the exchange minimum.
- Persist TP allocation state across module restarts.

# 1.0.102

Stairs-only pattern engine cleanup.

- Keep pullback classification available for analysis and learning only.
- Prevent pullback patterns from opening live positions.
- Remove obsolete primary-rule and pattern state from runtime, reports, and configuration.
- Keep valid staircase legs through cascade mismatch, stalled lifecycle, and
  barrier context; only invalidated staircase structure blocks entry.
- Force valid staircase entries to market execution so authorized continuation
  signals are not lost as unfilled one-candle limit orders.

# 1.0.101

Cascade limit-entry execution fix and stale default cleanup.

- Keep `cap_init` default at 50% in the module, strategy metadata, and tracked bot-group configuration.
- Emit the first cascade entry as a real limit position open so staged orders can be attached after the first fill.
- Keep additional cascade entry legs as standing orders and preserve the one-budget, up-to-three-leg design.
- Remote long-only stairs/limit backtest on the common 10k window: 197 positions, 109 wins, 88 losses, gross -15.3439, fees 10.5769, self-calculated net -25.9208; not profitable yet.

# 1.0.100

Cascade-driven stairs execution release.

- Use cascade direction, speed, continuation potential, adverse movement, and phase to gate stairs entries.
- Preserve the one-direction lock and reject stair/cascade direction mismatches.
- Support up to three staged limit entries and three reduce-only partial take-profit orders.
- Reconcile staged orders when cascade readiness changes.

# 1.0.99

Selected-candidate entry-note fix.

- Use the selected candidate for the actual opening note, not only the direction summary.
- Add staircase lifecycle transitions with protected-swing invalidation; stalled and invalidated staircases no longer create entries.
- Use cascade as the execution gate for stairs: confirm direction and speed, compare continuation potential with TP distance, and reject adverse movement larger than the planned SL.
- Support up to three marked cascade entry orders and three reduce-only partial TP orders while preserving one aggregate risk budget.

# 1.0.98

Selected-candidate direction diagnostics release.

- Render the selected candidate's prediction in entry notes instead of the last evaluated candidate.
- Keep selected pattern, predicted direction, trade direction, and order side aligned in diagnostics.

# 1.0.97

Current prediction and long-only evaluation release.

- Separate the current prediction outcome from the previous completed movement trace.
- Attach prediction side, fill state, target/SL result, result movement, and resolution time to each crossing.
- Keep realized movement direction correct and expose current target/SL movement separately from filled-position MFE/MAE.
- Improve long-only research output with per-line route information, cascade speed, NTPS, readable durations, and clearer prediction results.

# 1.0.96

State-machine primary pattern selection release.

- Keep support evidence available for the selected primary rule without allowing support patterns to open trades alone.
- Expose the selected pattern and all-pattern PnL/reason views in the research report.
- Preserve stair sequences and price-movement metrics in live and research decision traces.

# 1.0.95

Adaptive MRC prediction and pattern-selection release.

- Preserve long-lived crossing evidence while keeping local pattern classification responsive.
- Weight weak target evidence continuously so one FAST_BREAK_RETURN sample cannot dominate selection.
- Align research pattern feedback with live behavior by learning from selected trades only.
- Add an all-pattern PnL chart and detailed parallel-pattern trade reasons to the HTML report.

# 1.0.94

Pattern-driven MRC entry and exit pricing.

- Use the crossing market price as the MRC entry anchor instead of historical favorable movement.
- Keep favorable movement as projection evidence and select TP/SL from current MRC levels around the actual entry.
- Add stable MRC pattern classification and pattern-grouped research reporting.

# 1.0.93

Safe one-candle limit-entry cancellation.

- Cancel an expired limit entry without opening a replacement in the same evaluation.
- Wait for the host to confirm cancellation before processing a new entry signal.

# 1.0.92

One-candle limit-entry expiry release.

- Cancel unfilled limit entries when their one-candle lifetime ends, including same-side signals.
- Allow a fresh current-candle signal to create a new limit entry after the stale order is canceled.

# 1.0.91

Simplified fixed MRC exit handling.

- Represent strategy-provided TP/SL values as fixed variants of the existing exit types.
- Keep runtime exit mode only for active fixed, structural, and virtual-SL behavior.
- Remove the separate ExitLevels plumbing and ExitPlan mode field.

# 1.0.90

Strategy-provided MRC exit release.

- Use the MRC strategy's exact target and stop levels through the shared exit flow.
- Preserve strategy-provided TP/SL for pending, initial, and active positions.
- Keep the global minimum TP/SL guard applied to all strategy exits.

# 1.0.89

MRC opposite-direction core release.

- Make the learned opposite-direction trade decision the core MRC entry behavior.
- Keep the learned entry trigger while swapping TP and SL for the selected trade side.
- Remove the separate research-only mode so the default analysis follows live core behavior.

# 1.0.88

Global exit-distance guard release.

- Enforce configured minimum TP and SL distances through the shared guard layer.
- Keep exit-plan construction focused on selecting the strategy's raw TP and SL levels.
- Make used structural stop-loss exclusions explicit in the single exit-plan builder.

# 1.0.87

MRC Crossing model release.

- Replace the previous MRC reclaim transition logic with exact three-line crossing samples.
- Use the configured minimum samples to learn next-line probability, expected timing, and movement.
- Expire unfilled limit entries after one candle through the common execution path while continuing to evaluate fresh signals.

# 1.0.86

MRC Reclaim fixed-exit release.

- Keep the entry-derived MRC target and adjacent-line stop fixed during normal realtime evaluations.
- Preserve virtual-stop recovery behavior after the initial stop is crossed.
- Add regression coverage for MRC exit stability.

# 1.0.85

Trailing recovery TP repricing release.

- Reprice the reduce-only recovery TP from the current market price after the virtual stop is crossed.
- Keep the recovery order anchored to the latest price on successive realtime evaluations.
- Add integration coverage for long and short trailing repricing.

# 1.0.84

MRC sample warmup progress release.

- Restore the existing \`samples:current/required\` progress while MRC transition history is warming up.
- Keep the explicit MRC crossing wait once the configured sample requirement is ready.

# 1.0.83

Virtual trailing stop recalculation release.

- Recalculate the `VirtualTrailingExit` stop plan on every realtime evaluation instead of retaining the initial stop level.
- Keep trailing stops favorable-only while preserving the existing once-per-1m-bar TP repricing cadence.

# 1.0.82

Volatility-adjusted risk sizing release.

- Add optional `risk_target_percent` sizing based on projected stop-loss distance.
- Cap aggregate exposure when the projected stop is wider than the configured target.
- Reject protected entries that cannot satisfy a valid stop-loss or exchange minimum notional.

# 1.0.81

Common required-samples configuration release.

- Use the configured `required_samples` value for MRC startup seeding and live observation.
- Apply the common sample requirement across MRC, movement, SUG, and exit warmup logic.
- Remove the hidden MRC sample fallback and expose the requirement in the research tool.

# 1.0.80

MRC Reclaim stale-entry protection release.

- Consume MRC crossing entry reasons once instead of reusing unfilled signals on later ticks.
- Derive entry direction from the stored source-to-target transition so stale signals cannot reverse TP/SL orientation.
- Expose the internal position entry price in realtime debug output and keep final TP/SL validation at execution.

# 1.0.78

MRC Reclaim exit validation release.

- Migrate the MRC crossing research target/stop direction guard into the live strategy.
- Reject MRC Reclaim entries whose TP or SL collapses to the entry after exchange tick normalization.
- Keep invalid exit-level setups waiting instead of opening unprotected or zero-distance trades.

# 1.0.77

Event-time and volume resilience release.

- Use market event time for strategy timing instead of processing/system time.
- Treat missing `volume_50` fields as neutral values so incomplete volume data does not block indicator snapshots.

# 1.0.76

Indicator event recovery release.

- Decode string and typed direction values consistently for ZigZag and SMC indicators.
- Keep incremental timeframe updates from triggering extraction of unavailable timeframes when the cached snapshot is missing.
- Preserve full initialization errors while preventing misleading higher-timeframe cascades.

# 1.0.75

Indicator extraction diagnostics release.

- Collect all required indicator errors across fields and timeframes before returning the extraction failure.
- Preserve valid `Direction::Both` as zero without confusing it with missing data.
- Require SMC direction and swing direction while preserving genuinely optional fields as absent values.

# 1.0.73

Universal virtual-trading and research release.

- Consolidate MRC and movement research simulations on the shared virtual-trade simulator.
- Add configurable limit/market fees, net PnL, drawdown, and position-level reporting.
- Add MRC crossing analysis, volume analysis, and HTML research reports.

# 1.0.72

Zigzag data handling and crossed-entry pricing release.

- Preserve missing zigzagtrend `BuyPrice` and `SellPrice` values as optional extrema instead of treating them as zero or failing indicator processing.
- Submit crossed long/short limit entries two ticks beyond the current market while preserving the original entry reason and price.

# 1.0.71

Dynamic-exit release.

- Remove sideway-specific TP/SL exit planning.
- Use the normal movement/structural exit calculation for sideway entries and virtual-SL repricing.
- Remove the unused entry-reason parameter from exit planning APIs and callers.

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
- Reworked the long regime to accept price above the frozen structural anchor with `ZigZag` `BOTH` as valid bullish confirmation.
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
- Strategy inputs are now limited to `MRC`, `SMC`, `ZigZagTrend`, candle OHLC, and live price.
- Trading behavior is deterministic: exactly one market state, exactly one open position at a time, hard stop first, TP on pullback second, regime invalidation last.
