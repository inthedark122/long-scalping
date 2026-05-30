# Per-Trade-Type Auto-Scaling

## Purpose

v0.55 sizes most entries through one shared `risk_sized_notional()` call. The user's
directive (2026-05-30): in an *unpredictable* market we should be able to **lower the
overall risk**, but every entry *type* — seed, trend, stairs, tm-driven, re-entry,
spike — should compute its size with **its own auto-scaling calculation**, not a single
flat multiplier. Each trading style reacts to different evidence, so each deserves its
own size curve, all drawing from one pre-budgeted risk envelope so a full scale-in can
never exceed the risk budget.

## Core Concept

One **position risk budget per side**:

```
R_side = risk_sized_notional(max_amount, leverage, risk_pct, stop_pct)   // ≤ max_amount
```

Every entry type draws from the *remaining* room `room = R_side − current_side_notional`
with its **own fraction curve** `f_type ∈ [0,1]`, then a global modifier is applied:

```
new_amt = clamp( room × f_type × uncertainty_damp × risk_drawdown , 0 , room )
invariant: current_side_notional + new_amt ≤ min(R_side, max_amount)
```

Because every add is a fraction of *remaining* room, the sum of all adds converges to
`R_side` and **never overshoots** — "scale-in then stop ≤ risk budget" (memory
`feedback_risk_based_sizing`). `R_side` shrinks when `risk_pct` is lowered → the whole
ladder scales down together for an unpredictable regime.

### Per-type fraction curves (each its own calc)

| Type | `f_type` driver | Proposed curve |
|------|-----------------|----------------|
| **seed / auto-open** (flat→open) | conviction = bias × ntps_mult × zone-freshness | `0.40 × conviction` (leaves room for adds) |
| **trend** (`trend-s/l`) | confirmed multi-candle trend strength | `0.30 + 0.40 × clamp(\|tf5.mov\|/100 · tf5.conf/100)` |
| **stairs** (`stair-s/l`) | depth (which 5m line) + remaining room | rung-n gets `1/(n+1)` of room → 0.50, 0.33, 0.25… (diminishing) |
| **tm-driven** (`tm-s/l`) | `\|trandm\|` beyond STRONG | `0.30 + 0.30 × clamp((\|trandm\|−50)/50)` |
| **re-entry** (`re-s/l`) | 1m-ML angle confirmation | `0.25 + 0.25 × clamp(\|angle\|/angle_ref)` |
| **spike** (`s-spike`) | spike size vs avg | `0.15 × clamp(fc.peak_gain / fc.avg_gain)` |

### Global modifiers (apply to all types)

- **`risk_drawdown`** — existing risk-ratchet (scales down after drawdown).
- **`uncertainty_damp` (NEW)** — the "make risk less when unpredictable" knob:
  `0.6–0.8×` when `mkt_mode ∈ {UNK, Sideways}`, or `asset_01` is thin (`< $5M` → a thin
  book can whip), or regime confidence is low. `1.0×` in a clean, confirmed trend.

## State

```rust
// Config (existing): risk_pct, stop_pct  — lower risk_pct to de-risk the whole envelope.
// State additions (proposed):
pub uncertainty_damp: f64,   // recomputed each tick from mkt_mode + asset_01 + conf
pub stair_rung: u8,          // how many stair adds taken this position (drives 1/(n+1))
// R_side is computed inline; no new persistent field needed beyond stair_rung.
```

## State Transitions

- `uncertainty_damp` recomputed every SugInfo tick from `mkt_mode`, `asset_01`, `tf5.conf`.
- `stair_rung` increments on each `stair-s/l` fill; resets to 0 when the side goes flat.
- `R_side` recomputed each entry from live `max_amount`/`leverage`/`risk_pct`/`stop_pct`.

## Signals Derived

| Output | Meaning |
|--------|---------|
| `R_side` | hard ceiling of total notional allowed on the side |
| `room` | budget left to deploy (`R_side − current`) |
| `f_type` | this entry type's fraction of `room` |
| `new_amt` | final risk-bounded order size after damp + ratchet |

## History of Changes

### v0.56.0 (2026-05-30) — ✅ IMPLEMENTED
- **Why:** user — "we still in the unpredictable situation, so maybe we can make the
  risk less. but we should be able to use auto-scale for all types of trading even stairs
  or trend trading but using own calculation of auto-scaling. so each type of trading
  should have own auto-scaling solution."
- **Built:** `budget_size(room, f, tilt, state)` + `uncertainty_damp(state)`; `R_side`
  computed once per side in both `compute_auto_open` and `compute_limit_orders`; every entry
  type rewired to a fraction of remaining room with its own `f` curve; every DCA `remaining`
  re-based to `R_side − notional` so the cumulative invariant holds on all add paths.
- **Decisions (user):** `risk_pct` kept at **0.40** — de-risking is dynamic via
  `uncertainty_damp` (×0.75 UNK · ×0.80 thin book <$5M · ×0.85 weak 5m conf · ×0.85 1m/5m
  direction conflict; clamp [0.40, 1.0]), not a lower base. Thin-book threshold = $5M.
- Debug: `bgt=` (R_side), `dmp=` (damp). Open refinement: per-type `f` curve tuning from live.
