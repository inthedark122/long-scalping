# Strategy ownership & regime arbitration

> Design spec for letting multiple entry strategies (base trend/stair "pump" logic +
> the fast-candle scalp "FC") coexist in one bot without corrupting each other.
> Status: **design + simple first cut.** Live-validate; the order-book pieces can't be backtested.

## 1. The problem we hit

Running FC alongside the base strategy, a single position ended up managed by **three**
strategies at once — e.g. position 24306 (ALLO) carried `fcs-tp` (FC's +0.4% TP) **and** the
base `l-ext-t1/t2` band TPs **and** was finally closed by the base `sudden-down` panic. Result:
a **tiny TP capped by FC** but a **big SL allowed by the base** → ~1:6 risk/reward, churn, and
entries on tops. FC wasn't *adding* a strategy; it was *corrupting* the base one.

Root cause: **there is no notion of which strategy owns a position.** Every entry/exit block
acts on "the long," whoever opened it.

## 2. Core fix — per-strategy ownership

Each position is **owned by the exact entry strategy that opened it**, and **only that owner's
exit rules apply to it.** Ownership is **fine-grained — one owner per entry strategy, not a single
"base" bucket** — so even strategies that *currently* share an exit can be tuned separately later.

- **Owner tag** (`state.pos_owner: u8`), set on open from the opening order's `No:0` mark:

  | Owner | Opening mark | Default exit (today) |
  |---|---|---|
  | `OWN_AUTO` (1)   | `auto-l`            | band TPs + margin/sudden-down |
  | `OWN_STAIR` (2)  | `stair-l` / `dca-l` | band TPs + trend-sl |
  | `OWN_TREND` (3)  | `trend-l`           | progressive `t-l*` TPs + trend-sl |
  | `OWN_TRANDM` (4) | `tm-l`              | `tm-exit` (trandm neutralize) |
  | `OWN_REENTRY` (5)| `re-l`              | band TPs |
  | `OWN_FC` (6)     | `fcs-l`             | order-book resistance TP + deep DownBig stop |
  | `OWN_NONE` (0)   | — (flat)            | — |

- **Shared but adjustable exits**: each owner points at an **exit profile**. Several owners may
  reference the *same* profile today (e.g. AUTO/STAIR/REENTRY → "band-TP profile"), but because the
  routing is keyed per owner, we can later give STAIR its own profile without touching TREND, etc.
  Implement as a `match pos_owner { ... }` that selects which exit blocks run — a single edit point
  to re-route any one strategy.
- **Exit routing**: every close/TP block runs **only for the owners whose profile includes it**
  (e.g. `sudden-down` only for AUTO; `tm-exit` only for TRANDM; `fcs-tp`/deep-stop only for FC).
- **Entry routing**: entry blocks already carry distinct marks; we additionally gate them by regime
  (§3) so two strategies don't open competing positions in the same regime.

This removes the corruption (a `tm-l` position is no longer band-TP'd by stair logic, an `fcs-l`
position is no longer `sudden-down`-closed by AUTO logic), and keeps a per-strategy handle for
tuning each exit independently.

## 3. Regime arbitration — who is allowed to enter

Each strategy has a regime it is good in. The bot runs **one** at a time, chosen by regime:

| Regime (5m `mkt_mode`) | Active strategy | Rationale |
|---|---|---|
| **Sideways** (`SWY`, mode 1) | **FC scalp** | range chop → buy support walls / dips, sell resistance |
| **Trend up** (`TUP`, mode 2) | **Base** (stair/trend/pump) | ride the trend, don't fade it |
| **Trend down** (`TDN`, mode -2) | neither opens (long-only) | don't catch the knife |

Implementation: the entry blocks gate on regime —
- base entries require `mkt_mode == 2` (trend) and are skipped in sideways,
- FC entry requires `mkt_mode == 1` (sideways) and is skipped in trend.

So in a given regime only the **owning** strategy opens, and (per §2) only it manages the result.

## 4. Handoff — "try to exit", not force-close

When the regime flips and now favors the *other* strategy, the currently-open position is **not
force-closed**. Instead its owner is **commanded to try to exit gracefully**:

- Set `state.handoff = true` for the open position's owner.
- That owner's exit logic, when `handoff`, **tightens its target toward breakeven / its nearest
  profitable level** (e.g. base drops its far band TPs and takes the next small profit; FC pulls
  its `fcs-tp` in to the touch) — it exits at the *first reasonable* price rather than holding for
  the full thesis.
- Only once the position is flat does the newly-favored strategy start entering.
- A genuinely catastrophic level (deep DownBig / margin floor) still force-exits regardless.

This is the "more interesting strategy can run, so politely ask the current one to step aside"
behavior — no selling the bottom, no abrupt flip.

## 5. Weight / risk-profit arbitration (optional refinement)

Instead of (or on top of) the regime table, score each strategy's **potential** and let the
higher one take/keep control. Reuse the existing risk machinery (`risk_drawdown`, the per-side
risk budget `r_side`, `compute_allocation`/`compute_bias`):

```
weight(strategy) = expected_move% / risk%       // potential reward per unit risk
  FC:   expected_move = (resistance_wall - entry)        risk = entry - support_wall (or DownBig)
  base: expected_move = (UpBig - entry) * trend_conf     risk = entry - trend_sl
```

If `weight(FC) > weight(base)` in the current book/regime, FC may issue the handoff command to
base (and vice-versa). Keep it **simple first** (regime table in §3) and add the weight later.

## 6. FC entry/exit specifics (already built into `fcscalp9`)

- **Entry price**: nearest **order-book SUPPORT wall** below price (`short_levels`); maker LIMIT
  create-with-price. **Fallback** (no order book, i.e. backtest): MARKET open at the MRC
  DownInner dip — limit *opens* don't fill in backtest, but the TP (a reduce-only limit) does.
- **TP price**: nearest **order-book RESISTANCE wall** above price (`long_levels`); maker
  reduce-only limit. Fallback: `entry × (1 + fcscalp_tp_pct)`.
- **Stop**: deep **DownBig only** — hold through the flush, never sell the bottom on a Supertrend
  flip (that's usually the worst price). Catastrophic floor only.
- **Entry timing refinement (future)**: use **Parabolic SAR (1m)** to buy *into* the pullback for
  a better/lower price (not to wait for confirmation, which chases). Volatility-size the
  fallback distance with **NATR** if it's fed; else MRC range.

## 7. Simple first-cut implementation checklist

1. **State**: add `pos_owner: u8` (+ optional `handoff: bool`).
   - Set `pos_owner` when a position opens, mapping the `No:0` mark → owner id (table in §2).
     Latch it in `handle_order_update` (fill carries the mark) or on the flat→open transition.
   - Clear to `OWN_NONE` in `handle_finish` / when flat.
2. **Entries** (`compute_auto_open`): regime-gate so only the regime's strategy opens —
   base/trend/stair require `mkt_mode == 2`; FC requires `mkt_mode == 1` (no early-return — both
   strategies coexist, just never in the same regime).
3. **Exits** (`compute_limit_orders` + `handle_sug_info`): one `match pos_owner { ... }` per exit
   block (or a per-owner exit-profile selector) so each block runs **only for its owner(s)**.
   Same-profile owners share code today but stay independently re-routable.
4. **(Later)** handoff flag + risk/profit weight scoring (§4–5).

Steps 1–3 are the "simple implementation" — they stop the cross-contamination and give each entry
strategy its own lane with an independently tunable exit. Handoff/weighting (§4–5) layer on once
that's proven live. The order-book entry/TP (§6) is live-only; the ownership/regime routing itself
**is backtestable** (it's about which exits run, not the order book).
