# Window and Stairs Detection

## Purpose

Uses the dual-side V-movement history (see [[v-movement]]) to classify price behavior around an MRC line as either:

- **Window**: price oscillating on both sides of the line with sustained energy — no directional commitment, grid/range mode.
- **Stair Up**: each upward V penetrates deeper than the prior — buyers building momentum, potential genuine breakout.
- **Stair Down**: each downward V penetrates deeper — sellers building momentum, potential genuine breakdown.

The transition **Stair → Exhaustion** is the highest-value signal: momentum that fails to make a new high/low on the next attempt has run out. This is the entry window for the opposing position or aggressive TP.

---

## Core Concept

Both long Vs (above line) and short Vs (below line) are tracked by the [[v-movement]] `VTracker` struct. Window/Stairs reads those histories and classifies what the combined pattern means.

**Window**: price has made at least one V on each side of the active line, neither exhausted yet. Price is cycling through the line in both directions — neither buyers nor sellers have decisively won.

**Stair Up**: successive long Vs get deeper (`long_side.stair == true`). Each upward push breaks the prior peak. This is the opposite of exhaustion — momentum is building on the buy side.

**Stair Down**: successive short Vs get deeper (`short_side.stair == true`). Each downward push breaks the prior trough.

**Stair → Exhaustion flip**: the sequence was `stair=true` on V(n), then `exhausted=true` on V(n+1). The momentum run failed to continue. This is the turning point.

---

## State

No additional state fields beyond [[v-movement]]'s `VTracker`. Window/Stairs are derived in real-time from `long_side` and `short_side` each tick.

Derived booleans (computed in engine, not stored):

```rust
let window = vt.long_side.count >= 1
          && vt.short_side.count >= 1
          && !vt.long_side.exhausted
          && !vt.short_side.exhausted
          && !vt.v_active;

let stair_up   = vt.long_side.count >= 2 && vt.long_side.stair;
let stair_down = vt.short_side.count >= 2 && vt.short_side.stair;

// Stair→Exhaustion flip (most valuable signal):
let stair_up_exhausted   = vt.long_side.exhausted;   // was stair, now failed
let stair_down_exhausted = vt.short_side.exhausted;
```

---

## Signals Derived

| Signal | Condition | Meaning | Action |
|--------|-----------|---------|--------|
| **Window** | `long_count≥1 && short_count≥1 && !long_ex && !short_ex && !v_active` | Price oscillating both sides of MRC line | Hold position; no aggressive DCA; tight TP stairs at line edges |
| **Stair Up** | `long_side.stair == true` | Each upward V deeper — buying building | Reduce short DCA allowance; bias toward short TP; watch for flip to exhaustion |
| **Stair Down** | `short_side.stair == true` | Each downward V deeper — selling building | Reduce long DCA allowance; bias toward long TP; watch for flip to exhaustion |
| **Stair Up → Exhaustion** | `long_side.exhausted == true` after stair | Buy momentum failed — return move likely | Resume short TP aggressively; s-ts reverts to standard formula |
| **Stair Down → Exhaustion** | `short_side.exhausted == true` after stair | Sell momentum failed — return move likely | Resume long TP aggressively |

---

## DCA/TP Application

**Window detected**:
- Block aggressive DCA on either side (adding into a range oscillation wastes margin)
- Place TP stairs at both MRC line edges — let the range pay
- Wait for window to collapse into exhaustion on one side → direction confirmed

**Stair Up detected**:
- Do not add short DCA at or above the active line — each push goes deeper, adding into it increases risk
- If holding shorts, consider trimming the closest TP stair to book partial profit before the next V

**Stair Down detected**:
- Symmetric mirror of Stair Up for long positions

**Stair → Exhaustion (the entry window)**:
- The move that started as staircase has now failed to exceed the prior penetration
- If position is short and long_side just exhausted: short TP now fills favorably, resume DCA below line
- If position is long and short_side just exhausted: long TP fills, resume DCA above line
- This is the highest-conviction entry point — the prior move is confirmed as a stop hunt, not a breakout

---

## History of Changes

### v0.32.0 — Concept defined (2026-05-28, not yet implemented)

Extracted from [[v-movement]] dual-side tracking. Motivated by live session: after V exhaustion above UpBig (V1=20.4%, V2=8.1%, ratio=39.7%), price bounced both above and below UpBig in what would be classified as a Window phase. Tracking only one V direction missed the window pattern and the secondary short V sequence.

Staircase detection added to distinguish between: (a) exhaustion — stop hunt fading, and (b) stair — momentum building. These require opposite responses: exhaustion → resume TP/DCA; stair → hold back DCA, let the move extend.

Key insight: the Stair → Exhaustion flip is the most actionable signal. A sequence V1=5%, V2=8%, V3=6% says: V2 was a stair (momentum), V3 is exhaustion (failed to continue). The best entry is right after V3 closes, when price has returned through the line and the stair has visibly failed.
