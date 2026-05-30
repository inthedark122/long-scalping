# Trend-DCA Bounce Placement (sell the bounce / buy the dip)

## Purpose

The trend-following DCA adds (`dca-s` / `dca-l`) used to rest **1 tick ahead of price**
(`price − tick` / `price + tick`) — the comment literally said *"fills on the very next
tick move."* During a pullback that means the bot re-enters at the **dip** instantly,
stacking size at the worst price right before a bounce. Live on 2026-05-30 this turned a
winning short into a loss: after banking the initial dip, **six `dca-s` fills at ~2004
(each `price − tick`)** dragged the average to 2005.36, then price rallied to 2014 →
**−$0.12** on the rebuilt short. The fix makes a trend add **wait for a bounce** and fill
into strength, never at the extreme.

## Core Concept

- **`dca-s` (add short):** rest the sell a FastCandle-distance **under the running candle
  high**; the anchor **trails up** as the candle is green / makes new highs. Place it only
  once price has pulled back from the high by **≥ that distance** (so `sell_px` is strictly
  *above* market) — it therefore can never fill immediately at the dip; it fills only when
  price **bounces back up** toward the high (selling into strength, a distance below the top).
- **`dca-l` (add long):** mirror — rest the buy a distance **over the running candle low**,
  trailing down; fills only on a **dip back** toward the low.
- **Distance from FastCandle.** `dist = price × fc.avg_pct/100` (the average fast-candle move
  in $), floored to `5%` and capped to `50%` of the 1m MRC range so it's sane when the
  FastCandle average is ~0 (quiet) or huge (spiky). NATR is **not** wired yet (deferred —
  it will be a *confirmation gate only*, never the distance; user 2026-05-30).
- The movement-cascade gate (`mkt_mode == ∓2 && mv_cas_dir == ∓1`) still decides *whether*
  to add; this only changes *where* the add rests.

## State

```rust
// Running high/low of the CURRENT 1m candle (reset on candle close, extended each sug tick).
pub cur_hi: f64,
pub cur_lo: f64,
```

## State Transitions

- **Each sug tick:** `cur_hi = max(cur_hi, price)`, `cur_lo = min(cur_lo, price)`
  (with a `<= 0.0` guard to seed on the first tick).
- **Each 1m candle close** (`handle_indicators`, TF block): `cur_hi = cur_lo = price`
  (the boundary price) so the anchor tracks the new forming candle.

## Signals Derived

| Order | Rest price | Arms when | Fills on |
|-------|-----------|-----------|----------|
| `dca-s` | `max(cur_hi, price) − dist` | `sell_px > price + tick` (pullback ≥ dist) | a bounce **up** toward the high |
| `dca-l` | `min(cur_lo, price) + dist` | `buy_px < price − tick` (bounce ≥ dist) | a dip **down** toward the low |

`dist = clamp(price × fc.avg_pct/100, mrc_range×0.05, mrc_range×0.50)`.
The order uses the create-or-update mark (`"dca-s"`/`"dca-l"`): when the arm condition
fails (price too close to the extreme) the order is simply not emitted → it rests only when
a real bounce setup exists, and trails as the extreme moves.

## History of Changes

### v0.56.0 — 2026-05-30 (built, not deployed)
- **Why:** user, watching live — *"you just place it immediately on the same price while we
  have pullback while expecting price is growing… use at least small distance under each
  candle, or when the candle is green / updates its high… use FastCandle averaging to check
  the possible move; use NATR to confirm but not for distance."*
- **Built:** `cur_hi`/`cur_lo` running-candle extremes; `dca-s`/`dca-l` rewritten to anchor a
  FastCandle-distance under the high / over the low, armed only after a ≥dist pullback, trailing
  with new highs/lows. Verified against the live 2004 case: the add **skips** (0.2 pullback <
  0.75 dist) instead of re-shorting the dip.
- **Deferred (user):** NATR (`natr_14`, available in the feed) as a *confirmation gate* only —
  add the distanced order only when NATR shows enough volatility for a real bounce; never use
  NATR for the distance. Calibrate the floor from the live `natr_14` scale first.
