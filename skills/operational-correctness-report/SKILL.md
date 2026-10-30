---
name: operational-correctness-report
description: Write operational-correctness reports for trading strategies, focusing on position duration, signal behavior, and entry/exit quality rather than profitability.
---

# Operational Correctness Report

Use this skill when the user wants to understand whether a strategy is functioning correctly in production or backtesting—for example, why a bot keeps a position open long after a large move. The deliverable is an evidence-based operational report, not a profit report.

## Scope and safety

- Inspect only the requested bot, backtest run, symbols, and time window. If the scope is missing, infer it from the supplied bot/run ID and the latest comparable run, and state the assumption.
- A server backtest may be started only when the user explicitly asks to run one. Do not start, stop, reconfigure, or release a live bot as part of reporting.
- Use the repository root and repository-local `tmp/reports/` for reports, caches, generated HTML, JSON, and command output. Never use `/tmp`, `module/tmp`, or module-local temporary directories.
- Preserve raw evidence. Do not replace missing timestamps, order events, or position data with invented values; label them `unknown` and explain how that limits the conclusion.
- Do not treat profit, win rate, or a profitable position as proof that a signal or execution was correct. Profit may be included only as context when it explains an operational event.

## Evidence to collect

Prefer server backtest run data and execution/order records. Supplement them with strategy debug lines and the local research JSON/HTML reports when available. Separate these stages:

`prediction -> candidate signal -> decision gate -> order submitted -> order filled -> position managed -> exit requested -> exit filled`

For every stage, count transitions and missing transitions. Keep signal generation, order acceptance, fill quality, position management, and exit quality as separate measurements; otherwise a low fill rate can be mistaken for a bad predictor.

For this repository, run analysis commands from the repository root. When the standard research workflow is relevant, generate both JSON and HTML directly in `tmp/reports/`, include all requested symbols, and verify that the loaded history matches `--history`. If a cache is short, refresh it before accepting the result or explicitly record the shortfall. Common research statistics may be recorded as scope/context, but the report must remain operational rather than PnL-focused.

## Required analysis

### 1. Strategy and position duration

Measure, where timestamps exist:

- signal-to-order, order-to-fill, fill-to-first-favorable-move, fill-to-maximum-favorable-excursion, and fill-to-exit durations;
- time spent after the best favorable price before exit, including giveback after the move;
- time in adverse movement before exit, time at a stale target, and positions still open at the end of the window;
- duration distributions and outliers by symbol, direction, pattern, exit reason, and position state.

Explicitly flag the “long after big move” defect when a position reaches a material favorable extreme, fails to update its target/stop or exit state, and remains open for an unusually long interval or gives back a large part of that move. Show the bot ID, position ID, timestamps, price path, MFE time/price, exit time/price, and the relevant debug lines.

### 2. Signal-generation accuracy

Evaluate whether signals describe the next intended movement, independently of whether the trade made money:

- candidate signals, tradeable signals, gated/waiting signals, rejected signals, submitted orders, filled orders, and duplicate/repeated signals;
- direction correctness at short operational horizons (for example, whether the intended direction moved first over 1m/5m/15m or the available equivalent);
- favorable and adverse excursion after the signal, time to each, route/target-line reach, and whether the signal was generated after the meaningful move had already happened;
- stale signals, signals on a fresh extreme/spike, signals during reversal or invalidated staircase states, direction flips, and signals that disagree with current MRC/touch/timeframe context;
- results by pattern (`STAIRS_UP`, `STAIRS_DOWN`, and any configured alternatives), symbol, direction, MRC line, movement phase, and entry gate reason.

Report separate rates for “generated”, “approved”, “submitted”, and “filled”. A high generated count with few approved or filled orders is a gating/execution issue, not evidence that the predictor is accurate.

### 3. Entry and exit price-point quality

Report both quantity and quality.

Entry quantity and integrity:

- number of candidate, approved, submitted, filled, cancelled, expired, replaced, and duplicate entry orders;
- fill ratio, fill delay, limit distance from the signal price, price movement while waiting, and slippage/crossing behavior;
- entries by MRC line and structural state, including pullback, continuation, retest, fresh extreme, top/spike, invalidated, and unknown;
- whether the entry was near the intended best price or arrived after the favorable move, at a local extreme, or on the wrong side of a reversal.

Exit quantity and integrity:

- number of TP, SL, timeout, forced, cancelled, replaced, partial, and final exit orders;
- whether each opened position had one valid protective exit and whether exit quantities sum to the opened quantity, including exchange step rounding and small final remainders;
- TP/SL replacement churn, duplicate reduce-only orders, partial exits that use the wrong base quantity, and positions left with residual size;
- exit price relative to target, stop, MFE, MAE, and the best available price after the signal; measure giveback and exit lag instead of only labeling the result TP or SL.

For entries and exits, distinguish strategy price selection from exchange execution. A good selected price with a late fill is an execution problem; a bad selected price filled immediately is a strategy-point problem.

## Report format

Produce a concise report with:

1. **Operational verdict** — what works, what is broken, and confidence/data limitations.
2. **Scope and data quality** — bot/run IDs, symbols, time range, module/config version, rows/events loaded, and missing fields.
3. **Duration behavior** — distributions, stale/open positions, post-MFE holding, and the clearest examples.
4. **Signal behavior** — stage counts, direction/path accuracy, stale or contradictory signals, and breakdowns by pattern/state.
5. **Entry quality** — order counts, fill behavior, price-point quality, and spike/chase cases.
6. **Exit quality** — exit counts, protection/quantity integrity, replacement churn, exit lag, giveback, and residual positions.
7. **Prioritized improvements** — concrete code or instrumentation changes tied to observed evidence. Do not recommend a rule merely because it would reduce the number of trades.
8. **Evidence appendix** — position/order IDs and short debug excerpts for representative normal, best, and worst operational cases.

Use tables for stage counts and duration/price-quality distributions. Use a timeline for individual long-hold cases. Keep PnL out of the main verdict; if the source exposes it, place it in a small optional context field only.

## Interpretation rules

- `position=...` describes the current/existing position; it is not proof of a new entry.
- `selector=active:...` or a positive pattern score means a pattern was recognized, not that an order was approved.
- `selected=None`, `entry-wait`, `pattern-wait-*`, or `primary-awaiting-*` must be counted as gated/waiting behavior and explained.
- `na` is unavailable evidence, not zero and not success. Do not use it to satisfy a duration or quality threshold.
- A signal can be directionally correct but operationally bad if it arrives after the move, fills at a poor location, or leaves exits stale.
- A position can be profitable but operationally wrong if it entered late, held past its favorable extreme, used incorrect quantity, or relied on an invalid/missing protective exit.
