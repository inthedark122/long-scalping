# Sudden-Move Panic Exit

## Purpose
The bot should not close a position at a loss or breakeven on noise — small spikes and slow zone-change oscillation reverse. It should only force an adverse exit when a **genuine sustained move** is underway: price running one direction without rollback, with rising market participation, of meaningful size. This is the live-price-driven loss-exit that sits between holding for a profitable TP and the deep margin SL (liquidation protection).

## Core Concept
Track a "run": the number of consecutive SugInfo ticks price has moved in one direction without a meaningful rollback. The run extends while price makes new extremes (and tolerates shallow noise within `tol`); a retracement beyond `tol` resets the run to the opposite direction. A run qualifies as **sudden** when all three hold:
1. `run_ticks >= 5` (no rollback for ~5 ticks)
2. NTPS rising (`ntps > ntps_avg`) — real participation, not a single print
3. `move_pct > 0.5%` from the run's origin

A sudden move closes **only the adverse side**: a sharp rise (`run_dir > 0`) closes a short; a sharp drop (`run_dir < 0`) closes a long. A favorable run never closes the winning side.

## State
In `State` (`state.rs`):
```rust
pub run_dir: i8,            // +1 rising, -1 falling, 0 uninit
pub run_ticks: u32,        // consecutive ticks in run without rollback
pub run_start_price: f64,  // price at run origin (pivot)
pub run_extreme: f64,      // furthest price reached in run_dir
```
`tol = max(mrc_range * 0.10, price_tick_size)`.

## State Transitions (per SugInfo tick, in `handle_sug_info`)
- **uninit / first tick**: `run_dir` set from sign of `price − prev`, `run_ticks = 0`, origin/extreme = price.
- **same direction, new extreme**: extend extreme, `run_ticks += 1`.
- **same direction, shallow (within `tol` of extreme)**: `run_ticks += 1` (tolerate noise).
- **retrace beyond `tol`**: flip `run_dir`, `run_ticks = 1`, reset origin/extreme to current price.

## Signals Derived
| Signal | Definition | Action |
|--------|-----------|--------|
| `sudden_move` | `run_ticks>=5 && ntps>ntps_avg && move_pct>0.5` | Market-close adverse side |
| `sudden-up` | `sudden_move && run_dir>0 && has_short` | Close short |
| `sudden-down` | `sudden_move && run_dir<0 && has_long` | Close long |

Debug (`sug` line): `run=^12t 0.63% SUDDEN` — arrow=dir, ticks, move %, `SUDDEN` flag when armed.

## Relationship to other exits
- **Profitable band TPs** — unchanged, profit-only, never close at a loss.
- **`s-ts` spike-exit** — now fires only on an SMC trend flip (structural reversal). Its old noise triggers (`bullish_spike`, single cross-bounce above ML) were removed; the sudden-move detector replaces them.
- **margin SL / staged partial SL** — retained as deep-loss / liquidation protection (mandatory at 100x).

## History of Changes
### v0.50.0 — 2026-05-29
**Stop-hunt sweep-hold.** Live on 2026-05-29 15:31 the panic (`sudden-up`) plus the `s-ts` SMC-flip and the NTPS>600 close *covered a short at the top of a liquidity-grab pump into UpBig* — selling the top right before the expected dump (−$0.069). Added a gate: when price spikes INTO the upper extreme (`tf1.cross >= 3`) while `tf5.regime == TDN`, it's a stop hunt, not a reversal → suppress all short-covers there (`short_sweep_hold`); mirror `long_sweep_hold` (`cross <= -3` + 5m TUP) for longs. The deep margin/staged SL is unaffected (still the real backstop). Rationale matches the user's NTPS/stop-hunt rule and the V-movement docs (liquidity grab above UpBig before reversal to ML).

### v0.48.0 — 2026-05-29
Initial implementation (implementation.md 0.3). Replaces the noise-driven partial closes the user flagged: "I don't want to fire close for negative or non-profit … we should only [be] afraid if the price suddenly [goes] any way … single spike or small movement < 0.5% is almost nothing." Thresholds (5 ticks / NTPS>avg / 0.5%) are first-pass and expected to be tuned from live observation.
