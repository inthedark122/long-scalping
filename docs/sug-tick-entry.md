# Sug-Tick Organic Entry

## Purpose
Organic market entries (`compute_auto_open`) previously fired only inside `handle_indicators`, which runs on **1m candle closes**. A valid entry signal that formed mid-candle had to wait up to ~60s for the next close, so the bot sat idle through the exact moment it should have acted (observed: 500+ idle ticks while a clean setup passed). This feature lets entries fire on the SugInfo tick the moment conditions align, and feeds the real-time per-line crossing data into the freshness gate so the signal is actually available mid-candle.

## Core Concept
Two changes work together:
1. **Sug-tick call (Timing 0.1):** `compute_auto_open` is now also called from `handle_sug_info` every tick. Its internal `has_long`/`has_short` guards make it a no-op once a position exists, so it cannot double-open; the post-close cooldown (`*_close_tick` + `cooldown_ticks`) prevents immediate re-entry. The candle-close call is retained as a backstop.
2. **Real-time freshness (Timing 0.2):** `v_bull_fresh`/`v_bear_fresh` previously read only `level_stats`, which updates on 1m zone changes. They now also accept a recent crossing from `v_lines`, which updates on every sug tick — so the fine-grained per-line crossings already tracked for the V display now drive entry freshness.

## State
Added to `VLineTracker` (`state.rs`):
```rust
pub last_cross_tick: u64,   // state.ticks at last real-time crossing (0 = never)
pub last_cross_dir: i8,     // +1 crossed up (support test), -1 crossed down (resistance test)
```
Set in `update_v_lines` whenever `was_above` flips. Window constant in `compute_auto_open`:
```rust
const V_CROSS_WINDOW: u64 = 90; // sug ticks (~90s) of real-time crossing freshness
```

## State Transitions
- `update_v_lines` (every sug tick): on a line crossing, set `last_cross_tick = state.ticks`, `last_cross_dir = +1/−1`.
- `compute_auto_open`: `v_bull_fresh` true if any lower line (DB..DI) crossed **up** within `V_CROSS_WINDOW`, OR the legacy `level_stats` 5-candle test; symmetric for `v_bear_fresh` on upper lines crossing **down**.

## Signals Derived
| Signal | Source | Used by |
|--------|--------|---------|
| `v_bull_fresh` | `level_stats` (≤5 candles) OR `v_lines` up-cross (≤90 ticks) | SWY-mode long trigger |
| `v_bear_fresh` | `level_stats` (≤5 candles) OR `v_lines` down-cross (≤90 ticks) | SWY-mode short trigger |

## Safety
- No double-open: position guards inside `compute_auto_open`.
- No re-entry churn: existing `long_on_cd`/`short_on_cd` cooldown.
- Trend SL-cross recording (`trend_short_sl_cross`/`trend_long_sl_cross`) is replicated in the sug-tick path so trend opens still arm their zone SL.

## History of Changes
### v0.48.0 — 2026-05-29
Initial implementation (implementation.md Timing 0.1 / 0.2). Added `last_cross_tick`/`last_cross_dir` to `VLineTracker`; broadened freshness; called `compute_auto_open` from `handle_sug_info`. Related: [[v-movement]] (the V-line trackers this reads).
