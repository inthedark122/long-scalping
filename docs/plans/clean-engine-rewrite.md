# Clean engine reimplementation and feature-parity plan

Status: implementation in progress. Do not release or deploy from this plan until every release gate is green.

Implementation checkpoint (2026-08-23): the reducer, feature-controller bridge,
position-scoped exits, persistence migration, diagnostics, and core lifecycle
tests are implemented. Differential trace coverage now includes long and short
virtual-stop recovery, fixed-plan idempotency, pending-intent deduplication,
confirmed-flat re-entry gating, dual-side rejection, and invalid-price plan
preservation. Full baseline fixture automation, broader live-path replay
comparison, and release gates remain outstanding.

## 1. Objective

Reimplement the production trading engine with a clear event-driven architecture while preserving the behavior of the previous production engine.

“100% same logic” means the new engine must preserve:

- the same MRC inputs and derived observations;
- the same touch, crossing, movement, cascade, learning, and prediction state transitions;
- the same primary pattern authority and transition semantics;
- the same entry gates, wait reasons, direction locks, budgets, risk sizing, limit prices, expirations, and order idempotency;
- the same open-position context, TP/virtual-SL behavior, repricing, close conditions, lifetimes, and re-entry behavior;
- the same persistence and restart safety;
- the same diagnostic facts, including enough detail to explain every entry, wait, reprice, and close.

Optimization is allowed only after behavior is characterized. It may change data structures, allocation patterns, module boundaries, and computation cost. It may not change thresholds, ordering, route selection, side effects, or fallback behavior without a separately approved strategy change.

The current 1.0.114 live implementation is not a valid behavioral baseline. The previous production implementation at commit `5d46ff9` is the compatibility baseline.

## 2. Safety rules for the work

1. Do not release the current simplified engine.
2. Do not use the simplified engine’s results as evidence that the strategy works.
3. Preserve the current working tree’s unrelated user files.
4. Keep all new work local until parity tests pass.
5. Do not stop, close, or reconfigure live positions automatically as part of refactoring. Any rollback or bot action needs explicit operational approval.
6. Every analysis or code change that affects replay output must produce the required JSON and HTML reports.
7. Every stage must leave the repository compiling and testable.
8. No large generated adapter file may be used as a shortcut. Each feature must have an explicit owner, input contract, state, output, and tests.

## 3. Frozen compatibility baseline

Before writing the replacement engine, freeze the previous production behavior.

### 3.1 Baseline source

Use commit `5d46ff9` as the source of truth for live behavior:

- `module/src/engine.rs`
- `module/src/execution.rs`
- `module/src/exit.rs`
- `module/src/debug.rs`
- `module/src/market_data.rs`
- `module/src/model.rs`
- `module/src/state.rs`
- `module/src/detection/*.rs`
- `module/src/observer/*.rs`
- `module/src/risk.rs`
- `module/src/risk_control.rs`
- `module/src/orders.rs`
- `module/src/order_manager/*.rs`
- `module/src/stop_loss.rs`
- `module/src/guard/*.rs`

The baseline is an oracle for characterization, not a module to copy into the new engine.

### 3.2 Baseline artifacts

Create local, repository-owned fixtures under `tmp/fixtures/` and tests under `module/tests/`:

- indicator initialization inputs containing both 1m and 5m MRC data;
- incremental indicator updates;
- SugInfo ticks crossing every MRC line;
- exact touches and near touches;
- repeated same-line touches;
- reverted lines and no-pullback sequences;
- rising and falling staircases;
- stalled and exhausted staircases;
- potential and confirmed reversal transitions;
- cascade start, continuation, stretch, adverse move, and completion;
- volume/NTPS rising and falling sequences;
- learned route samples and prediction warm-up;
- entry accepted, entry waiting, entry rejected, and entry invalidated;
- market and limit entries;
- partial fills, duplicate fills, cancellations, expirations, TP fills, virtual stops, and confirmed flat events;
- restart with pending intent, open position, missing intent, missing indicators, and invalid state.

Do not hand-author expected output only. Capture normalized output and state from the baseline engine and store the expected facts in readable fixture files.

### 3.3 Normalization

Differential comparisons must normalize only nondeterministic values:

- timestamps when the fixture intentionally uses relative time;
- generated internal IDs when comparing equivalent intent identity;
- floating-point formatting within the baseline tick/precision rules.

Do not normalize away:

- direction;
- pattern;
- candidate selection;
- line sequence;
- route;
- target or stop;
- order type;
- quantity;
- close reason;
- repricing revision;
- diagnostic reason.

## 4. Complete feature inventory

The replacement is incomplete until every item below has a named implementation module and at least one parity test.

### 4.1 Bridge and event inputs

Support these events with explicit domain mappings:

- `Init`;
- `Indicators`;
- `SugInfo`;
- `Signal`;
- `Action`;
- `NewPosition`;
- `OrderUpdate`;
- `FinishPosition`.

The adapter must preserve:

- current price and symbol;
- max amount, auto max amount, leverage, and symbol constraints;
- 1m/5m indicator history;
- long/short server position summaries;
- SugInfo timestamp, symbol, NTPS, and activity fields;
- order role, status, fill price, filled quantity, and stable mark.

Malformed input must produce a diagnostic error and preserve the last valid trading state.

### 4.2 Indicator and MRC frame data

Preserve all fields used by the baseline:

- candle open, high, low, close;
- DownBig, DownOuter, DownSmall, DownInner;
- MeanLine, UpInner, UpSmall, UpOuter, UpBig;
- MRC current cross and slope/slope sign;
- volume gain, minimum gain, maximum gain;
- buy/sell gain;
- NTPS and NTPS fast timing;
- higher-timeframe bullish/bearish structure;
- higher-timeframe slope.

The parser must retain the distinction between:

- missing field;
- malformed field;
- non-finite field;
- valid zero;
- stale cached frame.

The engine must never silently turn a malformed snapshot into a neutral trading signal.

### 4.3 MRC line crossing and movement state

Preserve the full crossing state machine:

- line labels and ordered line indexes;
- initial line seeding from history;
- above/below state for every virtual line;
- first cross versus repeat cross;
- cross direction and cross timestamp;
- current crossing sequence;
- sequence durations;
- no-pullback count;
- reverted long and short lines;
- movement start price;
- market start price;
- target line and target range;
- MRC low/high range;
- movement high, low, and extreme;
- movement direction;
- movement age;
- movement completion;
- next-line progress.

Crossing state must be updated before decision evaluation on every relevant tick, with the same ordering as the baseline.

### 4.4 Touch detection and touch learning

Preserve:

- exact touches;
- crossing touches when the sample price is not exact;
- per-line touch pressure;
- touch EMA and regime;
- distance-to-line information;
- repeated touch count;
- line direction;
- touch intervals and time buckets;
- context by 5m MRC slope and price side;
- broad fallback versus contextual history;
- learned next-line route;
- support/resistance versus price-only movement;
- success/failure statistics;
- favorable and adverse movement statistics;
- expected movement time;
- learning warm-up behavior.

Touch learning must remain analysis state, not be replaced by a hard-coded formula.

### 4.5 Direction prediction and route planning

Preserve:

- current touch-derived direction;
- learned route direction;
- predicted next line;
- predicted line price;
- prediction confidence/state;
- mean distance context;
- higher-timeframe context;
- cascade context;
- movement coverage and expected value;
- route role: target, continuation, barrier, resistance, or reference;
- maximum reachable forward movement;
- route selection when multiple lines are reachable.

The predictor must remain neutral when the baseline is neutral. A refactor must not manufacture a direction from missing evidence.

### 4.6 Staircase classification and transitions

The new domain model must expose only two primary patterns:

- `STAIRS_UP`;
- `STAIRS_DOWN`.

It must also preserve non-primary transition evidence:

- potential reversal;
- confirmed reversal;
- stalled staircase;
- exhausted staircase;
- progressing lifecycle;
- pullback lifecycle;
- invalidated lifecycle;
- protected swing;
- protected line;
- favorable extreme;
- adverse extreme;
- staircase sequence and elapsed time;
- entry state, entry line, and entry reason;
- transition kind and transition reason.

Reversal is never a third primary pattern and never becomes an independent entry authority.

### 4.7 Secondary transition and support evidence

The main strategy list contains only the two primary patterns:

- `STAIRS_UP`;
- `STAIRS_DOWN`.

The following are separate transition/support evidence. They must not be
presented as primary patterns, added to the primary pattern list, or become
independent entry authorities:

- fast-break-return;
- pullback;
- volume;
- favorable-extreme;
- trend.

They may annotate or constrain a staircase decision only through the explicit
transition/support rules.

Preserve:

- component configuration from the start signal;
- runtime add/remove/replace action;
- unknown component diagnostics;
- primary candidate count;
- tradeable candidate count;
- selected candidate;
- selected score;
- direction lock;
- blocked-by-lock state;
- lock reset/rearm behavior;
- lifecycle and transition state;
- ending reason;
- support evidence flags.

Support evidence may influence the same baseline gate, but it must not independently create an entry unless the baseline did so.

### 4.8 Cascade and activity context

Preserve:

- movement phase;
- movement direction;
- movement speed;
- movement strength;
- stretched state;
- potential movement;
- adverse movement;
- average movement duration;
- recent directional size;
- recent movement distance;
- recent total and signed movement;
- activity timing;
- cascade trade state;
- volume gain zone and progress;
- NTPS rising/falling counters.

Cascade remains a context provider according to the baseline. It must not silently become a new direction authority.

### 4.9 Risk and budget

Preserve start-signal parameters:

- `cap_init`;
- `trading_amount`;
- `risk_target_percent`;
- `min_tp_percent`;
- `min_sl_percent`;
- `required_samples`;
- `entry_type`;
- `primary_mode`;
- `decision_components`.

Preserve:

- cap budget seed;
- leverage and minimum-notional rules;
- budget growth after wins;
- budget reduction after losses;
- risk-target adjustment;
- stop-distance percentage;
- aggregate same-side margin accounting;
- below-minimum-notional rejection;
- risk-adjusted-size-too-small rejection.

Every rejected entry must expose the exact constraint.

### 4.10 Entry behavior

Preserve:

- market entry versus limit entry;
- line-based entry price;
- two-tick adjustment when the selected price is already crossed;
- one pending intent per side;
- pending entry bar;
- one-candle limit expiry;
- cancel-before-replacement ordering;
- no replacement before cancellation is confirmed;
- direction BOTH switching;
- LONG/SHORT directional restriction;
- directional lock and same-direction rearm;
- recyclable staircase cycle limits;
- no entry while an open position exists;
- no duplicate entry for the same market version;
- intent persistence before order emission;
- complete order notes containing the selected route and risk facts.

### 4.11 Position lifecycle

Preserve separate contexts:

- market context for future entries;
- position context for the open side’s exit decision.

Preserve:

- fill matching to pending intent;
- average price and quantity on additional same-side fills;
- partial fills;
- duplicate fill idempotency;
- recovery of server-visible positions;
- fallback plan when intent is missing;
- no emergency close solely because analysis is missing;
- one-shot lifetime;
- recyclable staircase lifetime;
- cycle direction, sequence, age, and entry count;
- re-entry permission and reason;
- confirmed-flat requirement before new entry;
- one-position invariant and explicit handling of invalid dual positions.

An opposite market signal must never erase the open position’s exit context.

### 4.12 Exit behavior

Preserve:

- entry-anchored initial target and virtual stop;
- same-side dynamic route;
- target source and target reliability;
- stop source and stop reason;
- MRC line target versus movement target;
- target reachability and reward/risk validation;
- TP repricing rules;
- favorable extreme updates;
- stalled/exhausted behavior;
- cascade/activity timing effects;
- TP order marks and reduce-only semantics;
- quantity allocation and rounding;
- cancellation of stale TP orders;
- virtual stop close request;
- retry/idempotency after close request;
- confirmed flat before clearing state;
- missing or invalid indicators preserving the last valid plan.

The exchange-facing stop-loss field remains empty if that is still the intentional virtual-stop contract. The diagnostic must explicitly state that the stop is virtual.

### 4.13 Diagnostics

Replace the old string renderer with a structured diagnostic model, then render a compatible concise string.

At minimum, every SugInfo diagnostic must be able to show:

- sequence/tick and current price;
- pattern, selector, score, age, plan key, and direction lock;
- primary decision state and support evidence;
- primary trace and blocking reason;
- market direction, source, and reason;
- cascade state;
- volume/activity state;
- MRC risk;
- MRC crossing sequence and movement;
- touch signal and touch learning;
- prediction;
- entry gate, wait reason, route, target, stop, and constraint;
- intent ID and order details;
- position side, entry, quantity, TP, virtual SL, source, revision, lifetime, and re-entry;
- exit action, repricing, close reason, and last lifecycle event;
- indicator/persistence errors.

Diagnostics must distinguish:

- no candidate;
- candidate rejected;
- candidate accepted;
- intent submitted;
- fill pending;
- position open;
- exit plan held;
- TP repriced;
- virtual stop requested;
- waiting for flat;
- confirmed flat.

A state summary may be appended, but it must never overwrite the meaningful event diagnostic.

## 5. Target architecture

Use a small adapter plus explicit domain services.

```text
src/core/
  mod.rs                 public engine API
  bridge.rs              LTE input/output mapping only
  types.rs               domain events, state, decisions, orders
  indicators.rs          typed 1m/5m indicator snapshots
  mrc/
    lines.rs             line ordering and common line operations
    crossing.rs          virtual-line crossing state
    touch.rs             touch pressure and regime
    learning.rs          route learning and time buckets
    prediction.rs        next-line direction and movement forecast
    movement.rs          movement/extreme/progress state
  cascade.rs             movement/activity context
  strategy/
    pattern.rs           stairs classification and transitions
    decision.rs          components, gates, locks, constraints
    entry.rs              intent and sizing decisions
  position/
    context.rs           lifetime and re-entry state
    reducer.rs            fill/recovery/flat transitions
  exit/
    plan.rs               target/virtual-stop plan
    controller.rs         repricing and close decisions
  orders.rs               deterministic order construction
  persistence.rs          versioned snapshot and migration
  diagnostics.rs          structured diagnostics and rendering
  reducer.rs              event ordering and side-effect output
```

Rules:

- Feature modules consume typed domain snapshots, not `ModuleInput`.
- Feature modules do not emit bridge orders.
- Strategy decision modules return decisions, not side effects.
- The reducer is the only place that mutates durable state and constructs output actions.
- Position state is not stored inside market feature state.
- The live engine imports the new core only. Research tools may use a separate research adapter.
- No module may hide a second decision engine or silently call the old engine.
- Each module gets focused tests in the separate `module/tests/` tree.

## 6. Step-by-step implementation sequence

### Step 0 — Operational containment

Deliverables:

- written rollback decision and owner approval;
- no new release while the current engine is under reconstruction;
- current active bot/group/module versions recorded;
- current server position/open-order state recorded before any migration.

Exit condition:

- the team knows which version is allowed to trade during development.

### Step 1 — Build the baseline oracle

Tasks:

1. Add a test-only baseline runner that invokes the previous engine from a fixture-compatible copy or an isolated baseline crate.
2. Define a normalized `TraceEvent` containing input event, state facts, output actions, and debug facts.
3. Capture baseline traces for every fixture category in section 3.2.
4. Store expected decision facts, not giant opaque strings.
5. Add a command that prints the first divergence between baseline and candidate.

Exit condition:

- candidate tests can say exactly which feature, field, or order differs.

### Step 2 — Define domain contracts

Tasks:

1. Define typed events for all eight bridge events.
2. Define `IndicatorFrame1m` and `IndicatorFrame5m` with every field in section 4.2.
3. Define `MrcObservation`, `TouchObservation`, `MovementObservation`, `CascadeObservation`, `Prediction`, and `DecisionTrace`.
4. Define explicit `WaitReason`, `EntryConstraint`, `ExitReason`, and `PersistenceState` enums.
5. Define output intents separately from bridge output.
6. Add conversions at the adapter boundary and tests for malformed/missing fields.

Exit condition:

- no strategy module needs to inspect raw JSON or bridge-specific indicator types.

### Step 3 — Port indicator parsing exactly

Tasks:

1. Port the baseline 1m/5m parser.
2. Preserve optional versus required fields.
3. Preserve history merge behavior on partial timeframe updates.
4. Preserve slope calculation when only one candle is delivered.
5. Add tests for every indicator field and every failure mode.
6. Compare parsed domain frames field-by-field against baseline snapshots.

Exit condition:

- parsed frames are identical for all baseline fixtures.

### Step 4 — Port MRC primitives

Tasks:

1. Implement line labels, ordering, ranges, and line lookup.
2. Implement virtual-line state and cross events.
3. Implement sequence storage, timing, reverts, and movement snapshots.
4. Implement exact and crossing touch detection.
5. Compare every emitted cross/touch event to the baseline.
6. Optimize only after event order and values match.

Exit condition:

- all crossing/touch fixtures produce identical event sequences.

### Step 5 — Port learning, prediction, movement, and cascade

Tasks:

1. Port touch EMA/regime state.
2. Port contextual and fallback learning.
3. Port transition statistics and expected movement values.
4. Port direction prediction state.
5. Port movement/extreme/progress tracking.
6. Port cascade phase and activity updates.
7. Port MRC risk observation.
8. Add trace assertions for counters, time buckets, predicted line, movement percentages, and cascade values.

Exit condition:

- baseline and candidate feature traces match before strategy decisions are evaluated.

### Step 6 — Port pattern and decision selection

Tasks:

1. Implement staircase classification from the new crossing history.
2. Keep only stairs as primary patterns.
3. Represent reversal as transition evidence.
4. Port lifecycle, protected swing, extreme, and barrier behavior.
5. Port decision components and runtime actions.
6. Port candidate ranking and selected-score behavior.
7. Port direction locks and rearm rules.
8. Produce a full `DecisionTrace` for every tick, including waits.

Exit condition:

- selected candidate, market direction, gate, wait reason, and constraint match baseline.

### Step 7 — Port risk and entry decisions

Tasks:

1. Port cap/budget state and close-PnL updates.
2. Port risk-target sizing.
3. Port minimum-notional and leverage checks.
4. Port market/limit order selection.
5. Port line price and two-tick adjustment.
6. Port pending intent lifecycle and one-candle expiry.
7. Port cancellation-before-replacement ordering.
8. Port full intent/order notes from structured diagnostics.
9. Compare order direction, type, amount, price, and note facts.

Exit condition:

- every entry fixture produces the same open/update/cancel/constraint behavior.

### Step 8 — Port position context and lifecycle

Tasks:

1. Add durable intent state before order output.
2. Attach fills by side and intent identity.
3. Handle partial and duplicate fills.
4. Separate market signal context from position context.
5. Port one-shot/recyclable lifetime rules.
6. Port cycle limits, age, direction, and re-entry reasons.
7. Add server-position reconciliation.
8. Add confirmed-flat gating.
9. Add recovery tests for every restart state.

Exit condition:

- position lifecycle traces match baseline and no opposite signal mutates the position context.

### Step 9 — Port exit controller

Tasks:

1. Port initial entry-anchored exit plan.
2. Port same-side route selection and movement targets.
3. Port TP repricing and monotonic improvement rules.
4. Port virtual stop and explicit close request.
5. Port TP marks, reduce-only orders, quantity allocation, and rounding.
6. Port stale-order cancellation and duplicate suppression.
7. Port missing/invalid-indicator behavior.
8. Compare every exit order and close reason to baseline.

Exit condition:

- all TP/virtual-stop/reprice/flat fixtures match baseline.

### Step 10 — Implement structured diagnostics

Tasks:

1. Add a `DiagnosticSnapshot` containing the full feature and lifecycle facts.
2. Generate it from state, decision, and output—not by re-parsing strings.
3. Render a compact compatibility string with stable keys.
4. Keep the last meaningful event diagnostic separately from the last state summary.
5. Emit diagnostics for Init, Indicators, SugInfo, Signal, Action, fills, order updates, and FinishPosition.
6. Add assertions that every required section is present when its feature is active.
7. Add a diagnostic regression test that rejects accidental field loss.

Exit condition:

- the new log contains all baseline feature facts and never degrades to only `market=none position=flat` after a meaningful decision.

### Step 11 — Persistence and migration

Tasks:

1. Define a new explicit snapshot version and engine identifier.
2. Persist feature state, learning state, crossing state, pending intents, position context, exit plan, and diagnostics metadata.
3. Migrate valid new-engine snapshots.
4. Detect the simplified 1.0.114 snapshot and recover conservatively.
5. Detect old baseline snapshots and reconcile with server positions.
6. Never fabricate an exit plan from missing data without recording the fallback.
7. Never force-close only because a snapshot is unknown.
8. Add restart tests at every event boundary.

Exit condition:

- restart traces match uninterrupted traces and all unknown-state paths are observable.

### Step 12 — Differential and property testing

Tasks:

1. Run every fixture through baseline and candidate.
2. Compare normalized state after every event.
3. Compare output actions after every event.
4. Compare diagnostic fact sets.
5. Add property tests for:
   - no opposite-side position overwrite;
   - no duplicate intent;
   - no duplicate TP revision;
   - no new entry before confirmed flat;
   - virtual stop only closes on the correct side;
   - missing indicators preserve the last valid plan;
   - reversal never becomes a primary pattern;
   - support evidence cannot create an entry without primary stairs.
6. Fail CI on any unclassified divergence.

Exit condition:

- no unexplained divergence remains.

### Step 13 — Performance optimization

Only after parity:

1. Profile parsing, crossing updates, learning queries, and diagnostic rendering.
2. Bound histories with explicit constants matching baseline windows.
3. Avoid cloning large snapshots where ownership can be clear.
4. Use compact arrays for fixed MRC lines and counters.
5. Cache derived facts only when invalidation is explicit.
6. Re-run differential tests after every optimization.
7. Record runtime and memory comparison against baseline.

Exit condition:

- candidate is faster or simpler without any behavioral divergence.

### Step 14 — Replay validation

Run the new engine—not only the research tool—on:

- CAP;
- BEAT;
- ETH;
- ENA;
- BOME.

Use the requested history, refresh short caches, and generate JSON plus HTML reports. Compare:

- crossings;
- touch events;
- predictions/forecasts;
- cascade states;
- entries;
- completed positions;
- wins/losses;
- win rate;
- gross PnL;
- fees;
- net PnL;
- max drawdown;
- active/pending/open counts;
- emergency-close count;
- diagnostic coverage.

A replay difference is not accepted as “optimization” until it is explained at the event level.

### Step 15 — Canary and release

Before deployment:

1. Run formatting, full tests, locked checks, research checks, and release build.
2. Inspect the complete staged diff manually.
3. Verify the package excludes target, cache, and temporary files.
4. Build a release candidate without changing bot groups.
5. Deploy only to an explicitly approved simulation canary.
6. Verify:
   - module version;
   - active module state schema;
   - debug output;
   - MRC crossing/touch/cascade fields;
   - entries and pending intents;
   - TP/virtual-SL plans;
   - repricing and close events.
7. Compare canary traces to local candidate traces.
8. Release to production only after explicit approval.
9. Keep a tested rollback artifact and migration procedure.

## 7. Required test matrix

The candidate must have tests for:

| Area | Required cases |
| --- | --- |
| Indicators | full init, partial 1m update, partial 5m update, missing field, malformed field, stale frame |
| Crossings | every line, repeat line, direction change, reversion, sequence timing, no pullback |
| Touches | exact, near, repeated, regime change, distance EMA |
| Learning | contextual match, broad fallback, warm-up, successful route, failed route |
| Prediction | neutral, long, short, predicted line, invalid target |
| Cascade | start, build, stretch, adverse, finish, speed, average duration |
| Stairs | up, down, pullback, stalled, exhausted, protected swing, reversal transition |
| Decision | components, add/remove, direction lock, rearm, support-only rejection |
| Risk | cap seed, win, loss, risk target, min notional, zero budget |
| Entry | market, limit, two-tick adjustment, expiry, cancel-before-replace, duplicate version |
| Position | intent, fill, partial fill, duplicate fill, recovery, missing intent, opposite signal |
| Exit | initial plan, same-side reprice, invalid candidate, virtual stop, duplicate close, flat |
| Persistence | save/restore every phase, unknown schema, simplified-state recovery |
| Diagnostics | every event type, full MRC fields, entry facts, exit facts, error facts |

## 8. Release gates

The new engine cannot be released if any condition is true:

- any feature in section 4 is missing from the live path;
- a feature exists only in the research binary;
- the candidate uses a simplified substitute without a documented baseline equivalence;
- a golden trace diverges without an approved explanation;
- debug output omits a required active feature;
- a missing indicator can erase a valid position plan;
- a fill can be attached to the wrong intent;
- an opposite signal can mutate the open position exit context;
- a duplicate event creates a second order or close;
- a position can be re-entered before confirmed flat;
- a replay report is missing JSON or HTML;
- configured bots are updated before the canary is verified;
- rollback and migration are not tested.

## 9. Definition of done

The work is done only when:

1. The new engine has explicit feature modules and no copied mega-pipeline.
2. The baseline and candidate match on all characterization traces.
3. All MRC, touch, cascade, movement, prediction, entry, position, exit, risk, persistence, and diagnostic tests pass.
4. The full live path uses the same feature pipeline as the tests.
5. The new diagnostics explain every active decision.
6. CAP, BEAT, ETH, ENA, and BOME replays complete with all required reports.
7. A simulation canary confirms the same behavior and visible debug facts.
8. Only then is a release considered safe.
