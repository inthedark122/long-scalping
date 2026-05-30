# Opposite-Direction Exit + Net-Aware Hedge

## Purpose
The session of 2026-05-29 exposed a gap: a counter-trend position (a SWY dip-buy long) bled −1.3% through a sustained downtrend with **no exit** — the sudden-move panic needs a single >0.5% no-rollback run (a staircase decline never trips it), the band TPs only fire in profit, and `mkt_mode` mislabeled the downtrend as Sideways. The position just waited for a reversion the trend kept pushing away. This feature handles a confirmed *opposite* trend two ways at once: exit the stuck side at the **best reachable (least-negative) price**, and **open a hedge** in the trend direction so the waiting period is *traded*, not idled.

## Core Concept
When the confirmed trend (`mkt_mode`, now promoted correctly by the multi-TF override — see [[../implementation|#3]]) flips **against** an open side:
1. **Hedge** (`compute_auto_open`): open a position in the trend direction. Net-aware size = *neutralize* the stuck side + a *conviction excess* (scales with 5m momentum, 0–0.5×), capped by free-margin room. OKX **hedge mode** allows the simultaneous counter-position.
   - `H = S` → net-flat: the stuck side's loss is *frozen*; we wait calmly for its best-negative exit.
   - `H > S` → net-directional: the excess `(H−S)` profits from continuation while we wait.
2. **Best-negative exit** (`compute_limit_orders`): a reduce-only limit on the stuck side at a mean-reversion bounce target (ML), **trailed toward break-even only — never chased toward market**. The hedge freezes the loss, so we can afford to wait for this good fill instead of dumping at market.

The two sides exit **independently but with shared awareness** (net exposure is visible via `input.positions` each tick). When the stuck side's best-negative fills, the hedge remains as the clean net-directional trade.

## State
`Config`: `enable_hedge: bool` (default `true`; also gates skipping the old market-dump transition-close).
`State`:
```rust
pub neg_exit_l: f64,  // sell-to-close target for a stuck long (trailed up only; 0 = inactive)
pub neg_exit_s: f64,  // buy-to-close target for a stuck short (trailed down only; 0 = inactive)
```

## State Transitions
- **Hedge open** (`compute_auto_open`, gated `enable_hedge`): `has_long && !has_short && mkt_mode==-2 && !short_on_cd` → open short `hedge-s` sized `min(stuck·(1+excess), hedge_room)`. Mirror: `has_short && !has_long && mkt_mode==2` → long `hedge-l`. `hedge_room = (free_margin_est − 10% buffer)·leverage`. The `has_other` guard fires it once (no double-hedge, max 2 sides).
- **neg-exit maintenance** (`handle_sug_info`, per tick): if `has_long && mkt_mode==-2` → `neg_exit_l = max(neg_exit_l, ML.max(price+tick))` (trail up); else reset to 0. Mirror for short with `min`/`price−tick`.
- **neg-exit placement** (`compute_limit_orders`): reduce-only `neg-l` SELL at `neg_exit_l` (when `> price`), full `long_n`; `neg-s` BUY at `neg_exit_s` (when `< price`), full `short_n`.

## Signals Derived
| Signal | Action |
|--------|--------|
| stuck-long + `mkt_mode==-2` | open short hedge `hedge-s`; place `neg-l` best-negative sell |
| stuck-short + `mkt_mode==2` | open long hedge `hedge-l`; place `neg-s` best-negative buy |
| stuck side's neg-exit fills | hedge remains = net directional trade, normal management |

Debug (`sug` line): `negL=<price> negS=<price>` show the active best-negative targets.

## Interactions
- **Replaces** the old market-dump transition-close (`tc-cl`/`tc-cs`): those are skipped when `enable_hedge` is on (they dumped the wrong side at market — exactly what the best-negative exit avoids).
- **Margin/staged SL**: with a hedge, `total_pnl` (now true unrealized, see [[unrealized-pnl]]) nets out → SL won't false-fire on the hedged pair.
- **Sudden-move panic** still applies as the hard stop on a genuine runaway.
- **Prerequisite**: OKX account/symbol in **hedge (long-short) mode** (confirmed by user 2026-05-29), else a hedge order nets/reduces instead of opening a second side.

## History of Changes
### v0.49.0 — 2026-05-29
Initial implementation from the user's spec: "close opposite side if we detect opposite direction, but use TP in the best negative place" + "open hedge while waiting … sometimes bigger to make small profit on the excess" + "both sides independent but aware of each other." v1 uses `mkt_mode` (with the new multi-TF override) as the opposite-direction trigger, ML as the best-negative target, and 5m-momentum-scaled excess sizing under the free-margin gate. Open tuning: excess curve, neg-exit target refinement (currently ML), and per-side coordination when one leg closes.
