# Trend Target = 5m Mean Line + Scale-In on 1m Pullbacks

## Purpose
A trend position (e.g. a `tm-s` trandm short) should ride to the **higher-timeframe target**, not
be banked at a tiny 1m level. The take-profit reference is the **5m MRC mean line**, and 1m
pullbacks against the position are **add opportunities**, not exit signals. This corrects the
2026-05-30 18:23–18:32 mistake where the monitor recommended banking a +$0.015 short at the 2021
1m support — when the 5m had not even reached its mean line, i.e. the real target was far below.

## Core Concept (user, 2026-05-30)
> "To close short is wrong. Generally we can close but [only] for the ability to re-enter at the
> best price. Currently 1m MRC has very small distance between lines, NATR 1m is very small, so we
> can very easily break the 1m MRC because 5m still has not even touched [its] mean line. So 5m
> shows we have a very big distance for the short to take profit. I would even like to increase
> short size on the pullback."

Decomposed:
1. **TP reference = 5m mean line.** While price is on the far side of the **5m ML** (for a short:
   price still *above* the 5m ML), the trend has room to run. The 5m ML is the profit target —
   not the 1m DownBig/support, not a fixed few-cent gain.
2. **1m compression ⇒ 1m levels are weak.** When the **1m MRC band spacing is small** and **1m
   NATR (volatility) is low**, price breaks the 1m range easily. So a 1m bounce off a 1m support
   (e.g. 2021) is **noise / a pullback within the larger move**, not a reversal — do not exit on it.
   (This is *why* the v0.53 `trandm_hold_short` is correct to hold through such bounces.)
3. **Scale-in on pullbacks.** A counter-move (pullback) toward the 1m upper band while the 5m
   target is still far is a chance to **add to the position at a better average price**, not to
   close. Closing is only justified to **re-enter at a better price** (a tactical re-entry), never
   to bank a small gain and abandon the move.

## State (proposed for v0.55)
```rust
// 5m mean line (already available as the tf5 MRC MeanLine / read_mrc on the 5m timeframe).
// Need: per-side TP at the 5m ML, and a pullback-add ladder for the trend side.
//   short_tp_5m_ml: f64   // buy-to-close target = 5m ML (the big-distance TP)
//   add levels: when has_short && trandm<=-50 && 5m far above ML, add on pullbacks toward
//               the 1m upper band (better short entries), capped by free-margin.
```

## Signals Derived (proposed v0.55)
| Condition | Action |
|-----------|--------|
| trend short open, price still **above 5m ML** | hold — target not reached; ignore 1m bounces |
| price **reaches 5m ML** | take profit (the real TP) — `tm-tp` buy-to-close |
| **pullback** up toward 1m upper band, trandm ≤ −50, 5m far above ML | **add** to the short (scale-in at a better price), free-margin capped |
| 1m MRC compressed + low 1m NATR | treat 1m levels as breakable — do NOT exit on a 1m bounce |
| close *only* to re-enter better | allowed; never close just to bank a small 1m gain |

## Interactions
- **Replaces the v0.55 "short TP at support" idea** (18:23 note) — the TP is the **5m ML**, not the
  1m support band. (The 1m support is where you might *add*, not exit.)
- **`trandm_hold_short` (v0.53)** already holds the short through 1m bounces — this doc explains the
  *why* (1m compression) and adds the *target* (5m ML) + *scale-in*.
- **Sizing:** pullback-adds raise size at better prices; the user explicitly wants to *increase*
  short size into pullbacks while the 5m target is far (opposite of de-risking).

## MeanLine: touch (reversion) vs hard-break-and-hold (continuation)
User (2026-05-30): "MeanLines work good, but only when we **touch** [them] without hard breaking.
Currently 5m is holding candles UNDER the MeanLine (max price under the 5m ML) → we **hard broke**
this 5m line and price is just waiting for the next reaction."

So a MeanLine has **two regimes**, and using it as a level depends which:
- **Soft touch → reversion:** price tags the ML and turns. Here the ML is a valid mean-reversion
  target / re-entry price (the 0.C "re-enter at the 1m ML" rule applies) and a fade level.
- **Hard break + hold below (for a short) → continuation:** when candles *close and stay* under the
  ML — even the candle **highs** are under it — the ML is **broken/invalidated as support**. Do NOT
  expect a bounce back to it; price is *coiling for the next leg* (down to the next MRC line). This is
  a HOLD/continuation signal, not a reversion/TP signal.

Detection: track whether recent candle **highs** stay below the ML (hard break) vs price oscillating
across it (touch). A hard-broken ML ⇒ skip ML-reversion logic, expect continuation to the next line.
This refines 0.B (don't TP at a *broken* line) and 0.C (the 1m-ML re-entry only fires on a *touch*,
not when the ML is hard-broken and price is trending away from it).

### ML **angle** = the price-path expectation on a soft touch
User (2026-05-30, critical): "When we soft-touch the MRC mean line, usually the **angle** of the mean
line shows the potential expectation of price if we just stay under the mean line. Breaking big is
possible — in that case use any of the **fly** solution."

- On a **soft touch / price riding just under the ML**, the **slope of the ML is the expected drift**:
  a **down-angled ML** under which price sits ⇒ expect continued decline *along that slope* (bearish —
  favor holding/adding the short, and a re-entry short at the ML has the slope behind it). A flat/up
  ML ⇒ weaker/▲ expectation (a counter-trend short there fights the slope).
- The module already exposes `state.angle` (used in `compute_bias`/`compute_allocation`) — derive it
  from the ML slope (successive ML values) per timeframe. v0.55: **gate the 1m-ML re-entry on a
  confirming (down) angle**, and use the angle to set the continuation expectation.
- If instead price **breaks big** out of the band ⇒ abandon the angle/reversion read and switch to the
  **fly solution** (0.B): ride the breakout, don't TP at lines.
- So three states for the ML: **soft touch** (use angle to project drift) · **hard break + hold below**
  (continuation to next line) · **big breakout/fly** (ride, 0.B).

## ZigZag confirmation (add to the read)
The **ZigZagTrend** indicator confirms exhaustion of the up-leg: on the **5m**, `EndDirection=SHORT`
with **Fibo ≈ 0.91** means the bounce retraced ~91% of the prior range (swept up through the lines)
and has turned down — "no power to pump, just waiting" (user, 2026-05-30). Use zigzag
`EndDirection` + a high `Fibo` (deep retrace) as a confirmation that a counter-pump is spent and the
trend (here: down) is resuming. ZigZag `BreakPrice`/`FollowPrice` mark the structural levels.

## History of Changes
### v0 (concept captured) — 2026-05-30
Quantified live (18:35): 5m **ML=2014.07** vs price ~2024 → short has ~10pts/0.53% to the first
target, then 5m **DownInner=1994.96**, **DownBig=1955.99** (big runway). 1m **NATR=0.08%** (vs 5m
0.24%), 1m inner-band width **11.83** (vs 5m 38.21), price already **< 1m DownInner** (cross −2) —
1m levels are weak/breakable, so the 2021 bounce is a pullback not a reversal. 5m **ZigZag
EndDirection=SHORT, Fibo=0.913** → up-leg exhausted, turning down. All three of the user's reads
verified.

Captured after the monitor wrongly recommended banking a +$0.015 `tm-s` short at the 2021 1m
support. User corrected: hold to the **5m mean line** (far target), 1m MRC is compressed + 1m NATR
tiny so the 1m level breaks easily (the bounce is a pullback, not a reversal), and pullbacks are
**add** opportunities. To be built as v0.55: 5m-ML take-profit for the trend short + pullback
scale-in ladder. Until built, manage manually: do not close winners at 1m levels; add on pullbacks.
