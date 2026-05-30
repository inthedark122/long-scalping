# Volume Normalization Floor

## Purpose
The volume "spike" signals (`bearish_spike`, `bullish_spike`, `gain_pct`, `bsg_pct`, stair volume multipliers, fast-candle prefire) all normalize the current candle's `Gain`/`BuySellGain` against the 50-candle rolling **max−min** range. After a long quiet stretch that range collapses toward zero, so a tiny absolute move normalizes to ~1.0 — a "full-scale" signal manufactured from noise. This produced premature stair TPs and false exhaustion detections in sideways markets. The floor keeps the normalization denominator at a meaningful minimum so small moves stay small.

## Core Concept
`pct = (value − min) / max(max − min, FLOOR)`. When the real spread `(max − min)` shrinks below `FLOOR`, the floor takes over as the denominator. Because the numerator `(value − min)` is also tiny in a quiet period, the resulting `pct` stays near 0 instead of jumping to 1.0. When the market is active and the real spread exceeds the floor, behavior is unchanged.

## State
Two module constants in `engine.rs` (no runtime state):
```rust
const GAIN_RANGE_FLOOR: f64 = 0.5; // min Gain spread (Gain is a ratio: +1.0 = 100% above baseline)
const BSG_RANGE_FLOOR: f64 = 1.0;  // min BuySellGain spread
```

## State Transitions
None — pure stateless clamp applied at three call sites:
- `compute_auto_open` — `gain_range_st` / `bsg_range_st` (stair entry size multipliers)
- `update_volume_spike` — `gain_range` / `bsg_range` (`bearish_spike` / `bullish_spike`)
- `update_fast_candles` — `gain_range` / `bsg_range` (prefire detection)

## Signals Derived
| Signal | Effect of the floor |
|--------|---------------------|
| `bearish_spike` / `bullish_spike` | No longer trip on micro candles after quiet periods |
| `gain_pct_st` / `bsg_pct_st` (stair size) | Stay realistic; size multipliers don't inflate on noise |
| `prefire` (fast candle) | No false pre-spike flags during calm |

## History of Changes
### v0.48.0 — 2026-05-29
Replaced all `.max(f64::EPSILON)` denominators for `gain_range`/`bsg_range` with `GAIN_RANGE_FLOOR` (0.5) / `BSG_RANGE_FLOOR` (1.0). Motivated by the live observation (implementation.md 0.0): after ~50 quiet candles, `gain=0.17 → 0.55` read as a full-scale spike and fired stair TP logic prematurely.
