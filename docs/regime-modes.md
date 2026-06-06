# Regime Modes — long-only, three-state entry/exit

> Status: **DESIGN (not yet implemented)** · target branch `v0.61-long-regime` off `v0.61-long` · authored 2026-06-06

## Purpose
The long-only build (`v0.61-long`) currently fires a *soup* of entry types (`auto-l`, `trend-l`,
`tm-l`, `re-l`, `stair-l`, plus the `enable_grid` band-ladder) that each carry their own gates.
This redesign replaces that with **three explicit market regimes**, each with one clear, purpose-built
behaviour. Long-only means we only ever buy — so the goal is to **buy dips well** in a downtrend,
**range-trade** when there's no direction, and **scalp micro-pumps** in an uptrend.

## Core concept — regime detection
The regime is owned by the **movement-cascade** (the existing `mkt_mode`, derived from the `tf5`
cascade in `advance_market_mode`/`classify_mode`) **plus `trandm`** (the buy/sell control gauge,
−100..+100). The **1h `ZigZagTrend.Direction` indicator is confirmation only** — it agrees with or
**vetoes** the cascade's *trend* call (on disagreement, fall back to Sideway / wait); it never drives
the regime by itself.

| Regime | cascade gate | trandm | 1h ZigZag (confirm) | behaviour |
|---|---|---|---|---|
| **Downtrend** | `mkt_mode == -2` | `< -50` | `down` (else wait) | buy **only at `DownBig`**, **only on a red candle**, at the candle **`Low`** |
| **Sideway** | `mkt_mode == 1` | `[-50, +50]` | `both`/neutral | buy bottom / sell top on 1m & 5m MRC bands + risk control |
| **Uptrend** | `mkt_mode == 2` | `> +50` | `up` (else wait) | **stairs**: fast enter/exit, scalp each micro-pump |

"Strong move without retracement" (= *always updating bottom/top*) is exactly what ZigZag encodes —
that's why it's the confirmation, distinguishing a clean trend from chop the cascade may mislabel.

## Per-mode behaviour (recommended defaults — tune these)

### Downtrend — disciplined dip-buy at the extreme
- **Only** entry: a resting **buy LIMIT at the current red candle's `Low`** while `price <= DownBig`
  (`Mrc.DownBig`, `state.mrc_db`). Best-price fill on the continued dip; never chase.
- **Red candle only** — gate on `Candle.Direction == down` (or `Close < Open`) on **both 5m and 1m**
  (`analysis.klines`). A green candle = an attempted bounce; do **not** buy into it (the falling-knife
  rule: only add while it's still falling, at the deepest band).
- Once per leg: arm a single entry per DownBig touch; re-arm only when ZigZag prints a **new lower low**.
- Exit: ride to the bounce — TP toward `MeanLine`/`UpInner`; margin-SL backstops.

### Sideway — range trade the bands (this absorbs the old `enable_grid` ladder)
- Buy near the **lower** bands (`DownSmall`/`DownInner`), sell (reduce) near the **upper** bands
  (`UpInner`/`UpSmall`) using the existing band-ladder (`l-e1/l-e2` in, `l-t1/l-t2` out) — but **gated
  to Sideway**, not a standalone flag.
- Risk control: cap notional at `R_side` (existing `risk_sized_notional`), staged `psl-l` partial SL,
  full margin-SL backstop. Nearest 1–2 bands only (`cfg.nearest`).

### Uptrend — fast stairs, scalp the micro-pumps
- Use the stair-add ladder (`stair-l`) but **faster/tighter**: a "micro-pump" = a ~**+0.15%** up-leg
  (fast-candle), enter on the small dip/stair, exit at a **tight TP (~+0.10–0.15%)** so each pump is a
  quick cycle. Re-arm immediately for the next leg while the uptrend + trandm hold.

## Indicators (all confirmed available via `list_indicators`)
- **`ZigZagTrend`** (period via 1h timeframe) — `Direction`, `PrevDirection`, `EndDirection`,
  `StartPrice`/`EndPrice` for the confirmation layer.
- **`Candle`** (5m + 1m) — `Direction` (red/green), `Open`/`Close`/`High`/`Low` for the red-candle-Low entry.
- **`Mrc`** (`mrc_200`, already read) — `DownBig`..`UpBig` bands.
- **`mkt_mode`** + **`trandm`** — already computed in-module (cascade + sug analysis).
- (Optional) **`Smc`** `DirectionSwing` (30m) as a second trend confirmation.

## Open decisions to confirm before building
1. Downtrend: limit-at-`Low` (recommended) vs market-on-red-close at DownBig.
2. Sideway: exact band pair for bottom/top + the stop.
3. Uptrend: the micro-pump leg size and TP %.
4. ZigZag veto strictness: hard veto (no trend entries unless 1h agrees) vs soft (size-down).

## Implementation notes
- `compute_auto_open` becomes a **`match regime { Downtrend => …, Sideway => …, Uptrend => … }`** —
  each arm emits only its mode's entries. Same for the exit/TP logic in `compute_limit_orders`.
- **Remove the `enable_grid` config flag** — its band-ladder logic moves wholesale into the Sideway arm.
- New indicator reads: add `ZigZagTrend` (1h) and `Candle` (1m/5m red/green) to the indicator pulls.
- Keep `enable_stairs`/`enable_trend`? Likely fold into the regime arms too (Uptrend = stairs,
  Downtrend/Sideway have their own) — decide during the spec.

## Constraint
**Cannot be backtest-validated until the sim PnL bug is fixed** (`issues/001`). Build via compile +
review + fill-reconstruction spot-checks, and lean on **live** for the real signal until then.

## History of Changes
### 2026-06-06 — designed
Regime model agreed: movement-cascade + trandm own the regime; 1h ZigZag confirms/vetoes; `enable_grid`
folds into Sideway. Per-mode behaviours + recommended entry/exit defaults captured above. Not yet implemented.
