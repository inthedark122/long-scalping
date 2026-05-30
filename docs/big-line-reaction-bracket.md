# Big-Line First-Reaction Bracket (3-order TP/reaction ladder)

## Purpose

Today, when a short is open in a 5m-Sideways regime, the engine places **one** TP — 90%
of the position at DownBig (`engine.rs:1228-1233`, the `mode==1` branch). The user's
directive (2026-05-30): that single order is wrong for the **first** approach to the Big
line, because we *don't yet know how price will react* to Big. Until Big has been crossed
once, we should bracket it with **at least 3 orders** so we capture profit whether price
just touches Big and reverts, or fast-candles straight through it.

## Core Concept

The Big line has **two characteristic first reactions**:

1. **Just touch** — price reaches Big and mean-reverts (the band holds). Profit should be
   captured *at / just past* Big before it bounces.
2. **Fast candle** — price punches through Big by **1–2× the Outer→Big distance**
   (continuation / breakout). Profit (or a continuation order) should sit *below* Big to
   catch the overshoot.

A single order at exactly Big serves neither well: if price stalls just above Big it never
fills; if price blasts through, only the small qty at Big fills and the extension is missed.

### Geometry (SHORT, down-side — mirror for LONG at Up-side)

Let `O = DownOuter`, `B = DownBig`, `d = O − B` (Outer→Big gap, > 0):

| Rung | Price | Reaction it catches | Proposed qty |
|------|-------|---------------------|--------------|
| 1 — pre-touch | `(O+B)/2 = B + d/2` | early bounce *before* Big | 25% |
| 2 — at/just-below Big | `B − 0.15·d` | the **just-touch** case | 35% |
| 3 — overshoot | `B − (1…2)·d` | the **fast-candle** case | 40% |

(`B + d/2` is *above* Big since price falls Outer→Big; rungs 2–3 are *below* Big.)
The "1–2×" spec for rung 3 can be **one order at ~1.5·d**, or **split into two** orders at
`1·d` and `2·d` below Big to fully cover the range (→ 4 orders total). Total qty stays ≤
the current 90% so a runner remains.

### When it applies

- **First Big interaction only** — while `!side_big_tested`. Once Big is crossed/tested and
  the reaction is known, collapse back to normal logic: a touch-reversion → finish the TP;
  a confirmed break → hand off to continuation / extended-TP (`mrc_ub ± half_range`) and the
  "don't TP the fly" rule (`docs/trend-target-5m-ml.md`, impl 0.B).

## State

```rust
// State additions (proposed):
pub short_big_tested: bool,   // has price crossed/closed beyond DownBig since the short opened?
pub long_big_tested: bool,    // mirror for UpBig
```

## State Transitions

- `*_big_tested` = false when the side opens (flat → open).
- Set true the first time price trades through Big (e.g., `low ≤ DownBig` for a short).
- While false → emit the 3-(or-4-)rung bracket instead of the single `mode==1` TP.
- After true → normal single/laddered TP + continuation handling resumes.

## Signals Derived

| Output | Meaning |
|--------|---------|
| `bracket[3]` | the rung prices + qtys around Big for the first reaction |
| `*_big_tested` | whether the Big reaction is now known (collapse the bracket) |

## History of Changes

### v0.56.0 (2026-05-30) — ✅ IMPLEMENTED
- **Why:** user — "currently you place single limit order just on the down big. this is
  wrong. I expect at least 3 orders: middle between big and outer / slightly below big /
  below big. but this needed only for a first time when we don't cross big line yet … we
  expect 2 type of reaction: just touch / fast candle with 1-2x from the distance of
  Outer→Big."
- **Built:** `short_big_tested` / `long_big_tested` flags (reset flat, set when price trades
  through Big). While false, the `mode==1` single TP is replaced by a **4-order bracket**:
  pre-touch `(O+B)/2` 20% · just-touch `B∓0.15d` 30% · fast-candle **1× `B∓1d`** 25% ·
  fast-candle **2× `B∓2d`** 15% (user chose the split 1×+2× to cover the full 1–2× range).
  Mirror for longs at UpBig. Marks `s-bk1..4` / `l-bk1..4` track each rung independently.
- **STRICT close protection (user-requested, 2026-05-30):** every rung is gated profit-only
  two ways — `tp_is_profitable(entry, tp, side, fee_pad≈0.08%)` (close strictly past avg entry)
  **AND** `exp_pnl > 0` — and is `reduce_only` (can only close, never open). So when stairs
  trade **around** Big and move the average entry, below-entry rungs are never placed → the
  multi-close can **never open a negative TP**. Also fixed the pre-existing gap where the old
  single `mode==1` TP had no profit gate at all.
- Debug: `bk=` shows `short_big_tested``long_big_tested`. Open refinement: tune the 2d flag-flip
  handoff to the fly/extended-TP logic (impl 0.B) from live observation.
