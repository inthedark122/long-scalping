# Current entry-policy expectations and checks

This document tracks the current-code diagnosis. Run 238 is behavioral evidence; the task is to improve the current refactored code, not restore a source snapshot.

## Expected strategy behavior

- Ride sustained directional movement. A slow, continuously rising candle sequence should be able to produce a LONG entry; a falling sequence should be able to produce a SHORT entry.
- Use history to recognize the actual MRC movement: stair/mean line sequence, direction, extension, reversal evidence, and the movement that followed similar sequences.
- Treat continuation and countertrend as different cases. For countertrend entries, require matching prior history and the specified 2–3% extension or a spike, followed by reversal/price confirmation. Do not apply a blanket rule that blocks every countertrend or allows every one.
- Preserve independent MRC and candle-trend entry paths. Pattern forecasts may refine a target when reliable; an unrelated or unavailable forecast should not silently veto a continuation signal.
- Keep decision logic in focused domain modules. `engine.rs` routes events; it should not own pattern rules. Keep state limited to values needed across events.

## What Run 238 demonstrated

Run 238 finished the APR five-day window with 74 filled entries and 74 exits: 53 MRC/pattern-engine entries and 21 candle-trend entries. All 53 MRC entry notes show a 1x 5-minute ATR stop buffer. The run was able to take an MRC LONG after a selected `STAIRS_DOWN` structure when MRC analysis chose `trade_direction=1`, reported an upward cascade, and set an entry-movement target. The first such position, `417483`, exited by maximum-route TP 21 minutes later.

The outcomes were asymmetric by MRC-selected structure: 17 LONG positions against `STAIRS_DOWN` had 11 wins and 6 losses; 21 SHORT positions against `STAIRS_UP` had 11 wins and 10 losses. These counts are evidence that history needs to distinguish setup direction and context. They do not prove that every trade in either group was correct.

The candle-trend path was active, but its quality needs separate review: 21 positions had 4 wins, 13 losses, and 4 flat results. Preserve the path so it can be improved; do not treat its entry count as proof of quality.

Run 243 used the same symbol, window, strategy version 21, margin, and leverage, but stopped at 25.89%. It had only 2 MRC entries, no candle-trend entries, and 2 completed positions. At the previously captured market-time cutoff `1792775580000`, Run 238 had 10 entries and 10 exits, including one candle-trend entry. Run 243 is partial and cannot be compared as a full-window result.

## Diagnosis and latest test

The entry-policy bug was a second, generic OHLC-pattern gate on top of MRC's own candidate selection. It could veto an MRC continuation because an unrelated 30-bar forecast or breakout was missing. Candle-trend entries also had a generic continuation veto even though `candle_trend` already validates its own movement and MRC exit levels.

The current code removes that continuation veto. MRC staircase countertrades use the selected MRC analysis: they require a non-empty MRC sequence and historical samples, at least 2% favorable extension or an OHLC spike, and reversal/price confirmation. A rejected MRC candidate clears its plan key so a later valid candidate can be considered; a simultaneous candle-trend candidate remains independent.

Run 248 tested an overbroad intermediate change that removed the countertrend checks; it was stopped at 54% after its orders showed `reversal=potential` countertrades. It is not an accepted result.

Run 249 tested the narrowed code in the exact Run 238 five-day APR window. It finished with 59 entries and 59 exits: 39 MRC/pattern-engine and 20 candle-trend, 25 buys and 34 sells. Gross PnL was -$1.3804, fees $3.8257, and net PnL -$5.2062. Exit notes included 24 stop losses; there were no emergency no-exit-plan or module-plan-handoff-failure closes.

Over the first three market days, Run 249 had 31 completed positions (19 MRC and 12 candle-trend), with gross PnL $15.8525, fees $1.9939, and net PnL $13.8586. Run 238 had 43 completed positions and net PnL $25.3523 over the same dates. These PnL figures are not directly comparable: Run 249 order notes show `cap_init=20%`, while Run 238 notes show `cap_init=25%`. The backtest MCP exposes filled orders and positions, but not every rejected candidate, so per-gate rejection totals remain unavailable.

The policy bug is addressed in code, but Run 249's full-window result is not accepted as a working strategy result. Do not commit it yet.

## Files to inspect

- [`entry_policy/mod.rs`](../../module/src/entry_policy/mod.rs): joins the MRC and candle-trend decisions and applies the additional regime filters.
- [`entry_policy/gate.rs`](../../module/src/entry_policy/gate.rs): MRC-specific countertrend criteria and entry-level validation.
- [`pattern_matching/decision.rs`](../../module/src/pattern_matching/decision.rs): forecast-based target extension; it no longer vetoes entry candidates.
- [`pattern_matching/history.rs`](../../module/src/pattern_matching/history.rs): historical match key is broad `PatternKind`.
- [`pattern_matching/matcher.rs`](../../module/src/pattern_matching/matcher.rs): current classification comes from one-minute OHLC windows.
- [`detection/mrc_crossing.rs`](../../module/src/detection/mrc_crossing.rs) and [`detection/mrc_pattern_classification.rs`](../../module/src/detection/mrc_pattern_classification.rs): existing MRC sequence classifier, movement history, and MRC forecasts that may already satisfy the needed history ownership.
- [`detection/candle_trend.rs`](../../module/src/detection/candle_trend.rs): direct sustained-candle signal and structural target/stop selection.

## Remaining checks

- [ ] Compare Run 249 with Run 238 under the same `cap_init` before attributing PnL differences to the policy change.
- [ ] Inspect Run 249's days 4–5 orders and stop-loss notes to explain why net PnL changed from +$13.86 at three days to -$5.21 at five days.
- [ ] Check whether `candle_pattern=waiting bars=200` means the last 30 OHLC bars were invalid/non-contiguous or simply did not classify. Do not interpret 200 loaded rows as 200 matching patterns.
- [ ] Measure candle-trend entry timing and exit outcomes separately; Run 249 had 20 candle-trend entries and 24 stop-loss exits overall.
- [ ] Keep the Run 238 emergency no-exit-plan closes and staircase-invalidation exits as a separate exit-control investigation.

## Verification conditions

- Keep the current refactor and all staged/unstaged changes. Do not restore, reset, or remove files as part of this task.
- Keep strategy version 21 and module label `1.0.129`; the tested WASI build was `e9d5c570-87d6-463d-880d-68c65a5171ef`, pinned to existing leg 15.
- Inspect actual positions and orders. Compare entry-family counts and exit notes; do not accept a favorable partial-window PnL as proof of full-window behavior.
- Commit only after the same-window result is accepted.
