# Zone-Change TP Hold (don't nibble a winner)

## Purpose
When price drifts from one side of the MRC channel toward the other (e.g. above ML → below ML while we hold a short), that is **zone-change trading**: the position is winning and we expect a staircase of zone crossings in our favor. The old logic placed a reduce-only band TP at every favorable step, nibbling the position and partially closing right as the move worked — "we just partially close on top stair short signal that is totally wrong." This feature holds the winner during the favorable run and only takes profit once price **reverses** back toward ML.

## Core Concept
Track the most-favorable 1m cross zone the position has reached. While price keeps making new favorable extremes (or holds its best zone), the band-TP else-branch is suppressed — HOLD. Once price retraces ≥ 1 cross zone back toward ML, the position is "reversed" and band TPs are re-enabled to capture profit near the turn. All band TPs remain profit-only (`exp_pnl > 0`) — this feature only changes *when* they are placed, never closes at a loss.

## State
In `State` (`state.rs`):
```rust
pub short_min_cross: i8,   // lowest cross since short opened; 127 = no short
pub long_max_cross: i8,    // highest cross since long opened; -127 = no long
```

## State Transitions (per SugInfo tick, in `handle_sug_info`)
- **short open**: `short_min_cross = min(short_min_cross, tf1.cross)` (init to current on first tick).
- **no short**: reset to `127`.
- **long open**: `long_max_cross = max(long_max_cross, tf1.cross)`.
- **no long**: reset to `-127`.

## Signals Derived
| Signal | Definition | Effect in `compute_limit_orders` |
|--------|-----------|----------------------------------|
| `short_reversed` | `short_min_cross==127 \|\| tf1.cross >= short_min_cross + 1` | Band-TP else-branch placed only when true |
| `long_reversed` | `long_max_cross==-127 \|\| tf1.cross <= long_max_cross - 1` | Band-TP else-branch placed only when true |

Debug (`sug` line): `smn=-2 lmx=-127` — short_min_cross / long_max_cross.

Note: the trend-mode TPs (`trend_*_sl_cross` ladders), mode-1 full-exit TP, and extended-range TPs are unaffected — only the sideways/`else` band-nibble branch is gated.

## Known Limitation / Next Step
The user's full intent for zone-change is two-sided: **hold (don't nibble)** AND **add a stair (DCA) at each favorable boundary**. This version implements the hold half only. The "add stairs during sideways zone-change" half requires extending the DCA path (currently `enable_grid=false`, and stair DCA is gated on trend `mkt_mode==±2`) and is deferred to a later version for isolated testing — see implementation.md 0.1.

## History of Changes
### v0.48.0 — 2026-05-29
Initial implementation (implementation.md 0.1 / 0.2). Added `short_min_cross`/`long_max_cross`; gated the band-TP else-branches on a reversal of ≥ 1 zone. Also stripped the noise triggers from the `s-ts` spike-exit (see [[sudden-move-exit]]) so a winning short is no longer partially closed on a single cross-bounce above ML.
