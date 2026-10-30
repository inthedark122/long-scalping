# Open-position decision controller and lifecycle plan

## Status

Proposed. This document describes the implementation plan only. It does not
change strategy code, configuration, live bots, or orders.

## Purpose

Separate the decision for an already-open position from the decision for the
next position.

The market decision must continue to track the current Long/Short direction,
`STAIRS_UP`/`STAIRS_DOWN`, transition state, and future-entry information. An
open position must have its own position-specific decision context that keeps
calculating its TP and virtual SL even when the market selector moves to the
opposite side.

The position controller may use the future market context to decide that the
current position should be closed, but an opposite direction by itself must
not erase the current position's exit plan or immediately reverse the trade.

## Current problem

The current behavior is split in the wrong place:

- `module/src/engine.rs` stores `active_mrc_analysis` only when its direction
  matches the held position. A new opposite-side analysis is therefore not
  retained as future context, while the previous position analysis can become
  stale or unavailable.
- `module/src/execution.rs` enters the existing-position branch before the
  flat-entry branch. This correctly prevents a new entry while a position is
  open, but it leaves no separate decision object for the current position.
- `module/src/exit.rs` derives the active plan from the current runtime and
  `active_mrc_analysis`. If no valid plan can be built, it returns no TP order
  and no virtual-SL update. A live position can consequently reach
  `tp=none` and `no-valid-exit-plan`.
- `module/src/state.rs` persists only part of the position state. A restart can
  rebuild the server position without enough immutable entry context to
  reconstruct the exact original plan.

The target behavior is:

```text
market context:    latest route for the next position
position context:  exact route and protection for the current position
main executor:     position controller while open, market decision while flat
```

## Goals

1. Every live position has an authoritative position-specific decision.
2. A valid TP and virtual SL are always preserved or rebuilt from a known
   position anchor.
3. Opposite market analysis is retained as future-entry/transition context and
   cannot overwrite the current position's exit context.
4. The position controller can explicitly request `Hold`, `UpdateExit`, or
   `Close`.
5. A close must be confirmed before a replacement position is opened.
6. Position lifetime is explicit:
   - `OneShot`: manage and close this trade, then do not reopen from its old
     context.
   - `RecyclableStairs`: after closing, allow a fresh entry while the same
     staircase cycle remains eligible.
7. Recyclable trading has finite safety limits and stops on invalidation,
   confirmed transition, risk limits, or cycle expiry.
8. The strategy continues to have only two execution patterns:
   `STAIRS_UP` and `STAIRS_DOWN`. Reversal and transition remain contextual
   state, not a new `MrcPatternKind`.
9. Keep the existing virtual-SL plus TP-order model. Do not add exchange SL
   orders as part of this change.

## Non-goals

- Do not close a position merely because the selector changed direction.
- Do not implement a raw Long-to-Short flip in one evaluation.
- Do not compare speculative profitability without accounting for fees,
  slippage, and the current position's remaining route.
- Do not make one position technically live forever. Recyclable mode means
  multiple independently protected trades in one valid staircase cycle.
- Do not change pattern classification or add `Reversal` as a primary pattern.

## Proposed data model

### Market decision context

Retain the latest selected market analysis independently of position side. This
context is for future entries and transition reporting.

It should contain, directly or through the existing analysis types:

- selected direction and score;
- selected pattern (`STAIRS_UP` or `STAIRS_DOWN`);
- staircase lifecycle and transition;
- candidate entry, target, and stop levels;
- MRC route, barrier, cascade, and prediction information;
- whether the candidate is currently entry-eligible;
- whether it can be used for a recyclable re-entry.

The existing `TradeDecision` remains the entry-facing decision. The new
market context must not be restricted to the held position's side.

### Position context

Add an explicit runtime context for the open position. It should be created
from the actual filled position and remain tied to that position until the
host reports flat.

Recommended fields:

- position side;
- actual filled entry price and entry time;
- immutable `EntryReason`/entry analysis snapshot;
- original target and virtual-stop anchor;
- whether the target was movement-based or MRC-line-based;
- original source/target/stop line metadata;
- last valid same-side MRC analysis and its non-indicator context;
- last valid calculated `ExitPlan` and its source;
- current exit mode, TP, virtual SL, and exit progress;
- position lifetime mode;
- staircase cycle identifier or entry cycle key;
- consecutive recyclable-entry count;
- close/re-entry eligibility and reason.

The current `PositionRuntime` can continue to hold live execution values, but
the immutable entry anchor and last-valid plan must not be represented only by
the latest market analysis.

Recommended lifetime enum:

```text
PositionLifetime::OneShot
PositionLifetime::RecyclableStairs
```

Recommended exit-plan source enum:

```text
DynamicSameSide
LastValidPositionPlan
EntryAnchor
EmergencyClose
```

The exact names may change during implementation, but the source must be
observable in debug output.

### Position decision result

Introduce a position-specific result returned to the main execution path:

```text
PositionDecision {
    action: Hold | UpdateExit | Close,
    side,
    exit_plan,
    exit_plan_source,
    close_reason,
    future_route,
    reentry_allowed,
    lifetime,
}
```

`future_route` is informational until the position is confirmed closed. It may
contain the latest opposite-side candidate without changing the current
position's TP or virtual SL.

## Position plan resolution

For every open position, the position controller resolves protection in this
order:

```text
valid same-side dynamic plan
    -> last valid plan saved for this position
    -> immutable entry TP/virtual-SL anchor
    -> emergency close with re-entry blocked
```

Rules:

1. A dynamic plan is eligible only when its analysis direction matches the
   position side and its target/stop pass the existing price-level and minimum
   distance validation.
2. An opposite-side market analysis may be supplied to the position
   controller as transition information, but it cannot be used to calculate a
   Long exit or overwrite the Long position context.
3. A newly calculated stop may only improve protection according to the
   existing monotonic stop rules. A temporary loss of same-side analysis must
   not widen or clear the current virtual SL.
4. A valid plan must be cached before it is applied. If a later dynamic plan
   is invalid, the cached plan remains available.
5. The original entry anchor must be reconstructed from the filled entry
   context, not from the current market price.
6. If no valid anchor or cached plan exists, the controller must request a
   controlled close and block re-entry. It must never silently return an
   unmanaged live position with no TP and no virtual SL.
7. The existing TP order and virtual-SL behavior remains unchanged after the
   plan is selected.

## Separate market and position evaluation

### While a position is open

Each evaluation should perform these steps:

1. Build or refresh the market decision context, including an opposite-side
   candidate if present.
2. Update only the position context's same-side analysis when it is valid for
   the open position.
3. Run the position controller with:
   - the open position context;
   - current indicators and price;
   - same-side position analysis;
   - future market/transition context;
   - lifecycle and risk state.
4. Apply the returned `Hold`, `UpdateExit`, or `Close` action.
5. Return the future candidate and lifecycle information in the main decision
   state/debug output.
6. Do not submit a new entry while the host still reports the old position.

### While flat

1. Use the latest market decision context for entry evaluation.
2. If the previous position was `OneShot`, reject the stale position context.
3. If the previous position was `RecyclableStairs`, allow re-entry only when
   the same staircase cycle is still valid and all risk/cooldown limits pass.
4. Create a new position context only after an actual fill or the existing
   position-creation event, using the actual filled price.

### Close and replacement sequencing

The controller may report that a current position should close because its
position-specific route is exhausted, invalidated, or no longer protected.
The executor must sequence this as:

```text
position decision = Close
    -> submit close
    -> wait for host-confirmed flat state
    -> re-evaluate latest market context
    -> open a new position only if the candidate is still valid
```

No same-cycle close-and-open should be emitted. If the close fails or the
position remains open, its previous protection continues to control it.

An opposite direction alone is not a close reason. A close reason must be
explicit, such as:

- position route invalidated;
- protected staircase line broken;
- confirmed transition according to the position policy;
- position lifetime completed;
- emergency protection unavailable;
- configured risk or time limit reached.

## Position lifetime behavior

### OneShot

Use for a trade that should complete once and then stop using its entry
context. After the host confirms flat:

- clear the position controller's active context;
- block re-entry from the old candidate;
- let a later fresh market cycle create a new context.

### RecyclableStairs

Use when the open trade belongs to an active, same-direction staircase that is
eligible for repeated legs. After the host confirms flat:

- keep only the bounded staircase-cycle/re-entry context;
- require a fresh candidate evaluation;
- open a new independently protected position if allowed;
- increment the cycle-entry counter.

Recycling must stop when any of these occurs:

- staircase lifecycle is invalidated or exhausted;
- confirmed transition/reversal invalidates the same-direction route;
- maximum cycle age is reached;
- maximum consecutive entries is reached;
- cooldown or risk-control rule blocks re-entry;
- the candidate no longer has valid target and virtual-stop levels.

The first implementation should use conservative bounded defaults and expose
the lifetime/re-entry reason in diagnostics. A truly unlimited mode should not
be introduced without a separate operational review.

## Persistence and runtime recovery

Extend the persisted state to retain enough position context to reconstruct the
same plan after a module restart:

- position lifetime;
- entry side and actual entry price;
- immutable entry reason/anchor;
- last valid exit plan and source;
- re-entry/cycle counters and cycle validity;
- any close-pending state that is safe to restore.

Use serde defaults for backward compatibility with existing state payloads.
When a live server position is detected but the persisted position anchor is
missing or invalid, apply the emergency-close policy and block new entries;
do not attempt to invent a target or stop from the current opposite-side
analysis.

## Implementation phases

### Phase 1: introduce state and decision types

- Add position lifetime, exit-plan source, position action, and position
  context types in the appropriate state/model module.
- Add separate market-context and position-context fields to `EngineState`.
- Preserve existing `PositionRuntime` fields during the transition.
- Add serde defaults and persistence/restore handling.

### Phase 2: split analysis ownership

- Store the latest selected `STAIRS_UP`/`STAIRS_DOWN` analysis as market
  context regardless of held side.
- Store/update a separate same-side analysis snapshot for the open position.
- Keep the position snapshot unchanged when the selected market direction is
  opposite.
- Keep transition/reversal information in the market context; do not add a
  pattern kind.

### Phase 3: implement the position controller

- Move or refactor active-position plan resolution so it can consume the
  position context explicitly.
- Implement dynamic → cached → entry-anchor fallback resolution.
- Cache a plan before applying it.
- Preserve the existing monotonic virtual-SL behavior and TP repricing rules.
- Return an explicit emergency close when no valid protection can be built.

### Phase 4: integrate execution sequencing

- Make the position controller authoritative whenever a live position exists.
- Keep flat-entry evaluation separate from open-position management.
- Prevent same-evaluation close-and-open behavior.
- Re-evaluate the newest market context after confirmed flat state.
- Return future-route and lifecycle information through the main decision/debug
  output.

### Phase 5: add lifetime and recyclable stairs policy

- Assign `OneShot` or `RecyclableStairs` at entry from explicit strategy
  context.
- Add bounded cycle age, consecutive-entry, cooldown, and invalidation checks.
- Permit re-entry only after confirmed flat state and a fresh valid candidate.
- Verify that re-entry uses a new entry anchor and does not reuse the prior
  position's TP/virtual SL.

### Phase 6: diagnostics and operational validation

Use compact, non-duplicated fields such as:

```text
market=Short/STAIRS_DOWN/transition=...
position=Long
position_action=Hold|UpdateExit|Close
position_exit_source=dynamic|cached|entry-anchor|emergency-close
position_tp=...
position_vsl=...
future_route=Short
lifetime=one-shot|recyclable-stairs
reentry=allowed|blocked reason=...
```

The critical invariant must be visible:

```text
open position => valid TP and virtual SL, or explicit emergency close
```

Do not emit several synonymous debug fields for the same state. Each field
should describe one distinct decision or source.

## Tests

Add behavior tests in the crate's separate `module/tests/` directory.

### Position decision tests

1. Long position plus selected Short market analysis preserves the Long exit
   context and produces TP/virtual-SL output.
2. Short position plus selected Long market analysis has the symmetric
   behavior.
3. Opposite analysis is stored as future context and does not overwrite the
   same-side position analysis.
4. Invalid current same-side analysis falls back to the last valid position
   plan.
5. Missing current analysis falls back to the immutable entry anchor.
6. Missing entry anchor and missing cached plan request emergency close and
   never produce an unmanaged live position.
7. Dynamic TP repricing still works for valid same-side movement entries.
8. Virtual-SL crossing still transitions to the existing recovery TP behavior.

### Lifecycle and sequencing tests

1. `OneShot` does not reopen after its confirmed close.
2. `RecyclableStairs` can reopen only after confirmed flat state.
3. Recyclable mode stops after staircase invalidation, cycle expiry, or risk
   limits.
4. A close result does not contain a new entry in the same evaluation.
5. A failed/unconfirmed close keeps the old position controller active.
6. A replacement entry receives a new entry anchor and new exit plan.

### Recovery tests

1. Persist and restore a live position context without changing its anchor.
2. Restore older state payloads using serde defaults.
3. Detect a live position with missing context and request emergency close.

Run the focused position/execution/state tests first, then the full
`cargo test --manifest-path module/Cargo.toml --locked` suite.

## Operational acceptance criteria

Before release, verify with runtime/debug data that:

- no open position reports `tp=none` while its virtual SL is also absent;
- opposite direction is visible as future context rather than being confused
  with the current position's exit direction;
- every close has an explicit position-controller reason;
- every replacement entry follows a confirmed flat state;
- one-shot and recyclable lifecycle decisions are distinguishable;
- position-side TP repricing and virtual-SL behavior remain intact;
- no `MrcPatternKind::Reversal` or other third execution pattern is added.

After implementation, generate the required operational-correctness report
and compare the loaded history with the requested `--history`. If report
results are affected by the change, refresh any short cache before accepting
the report and record remaining data shortfalls.

