# TrandM & Asset_01 — Pump/Dump Potential Gauges

> **Source (resolved 2026-05-29):** NOT in `/api/indicators` / `list_indicators`. They live in
> the **Projection (sug_info) feed** — `indicator.trandm` and `indicator.asset_01` — which the
> module receives every tick via `handle_sug_info`. Live read path:
> `wss://okx.tg.trading.sinvoid.me/ws` → `{"event":"Subscribe","param":{"event":"Projection","symbol":"ETH-USDT-SWAP"}}`
> (subscriber: `eth-dynamic-grid-hedge/sug_ws.py`).
>
> **Module status:**
> - `trandm` IS already deserialized (`SuggestionInfoIndicator.trandm`, read in `read_analysis`,
>   stored as `state.trandm`) — **but used BACKWARDS** in `compute_bias` (see below).
> - `asset_01` IS in the bridge struct (`SuggestionInfoIndicator.asset_01: f64`) but the module
>   does **not read it yet** (`read_analysis` only pulls `ntps`+`trandm`). One-line add.
>
> **Scale (resolved 2026-05-29):** `asset_01_usd = raw_field × 100` (user: "I multiply asset_01
> * 100 to see 9m"). Raw 91.6k → **$9.1M**. ETH reference in **raw** units: **very calm/loaded
> raw < 50k ($5M)**, **floor ~40k ($4M)**, **peak ~200k ($20M)** (spent → expect normal vol).
> Current ~91k ($9.1M) = moderate.

## ✅ FIXED v0.53.0 — `compute_bias` misused `trandm` (was engine.rs ~2136)
Current code treats trandm as a 0–100 oscillator with mean-reversion logic:
```rust
if trandm > 70.0 { sbias *= 1.2; lbias *= 0.8; }  // overbought -> favor short  (OK-ish)
if trandm < 30.0 { lbias *= 1.2; sbias *= 0.8; }  // "oversold" -> favor LONG    (WRONG)
```
trandm is **−100…+100**, so `trandm < 30` fires across the entire oversell region. At
trandm = −68.96 it **boosted long bias and cut short bias** — i.e. "buy the dip" mean-reversion,
the *opposite* of the user's model (oversell ETH = down-spike-prone, no pump expected). This
likely contributed to the engine auto-opening a long on the relief bounce. v0.53 must (a) use a
symmetric ±50/±60 scale and (b) flip the direction so deep-oversell favors **short** (continuation
into the loaded down-spike), deep-overbought favors the mirror.

## Purpose
Standard indicators describe the *current* trend/momentum. These two gauges describe the
**potential energy** for a large move — whether the market is stretched (overbought/oversold)
and whether it is *coiled and calm* (thin) or *spent* (after a big move). Together they answer:
"is a real pump/dump and a possible huge candle likely, and in which direction?"

## Core Concept

### TrandM — side-in-control gauge (−100% selling … +100% buying)
- A normalized −100…+100 metric showing **which side is in control**. **Negative = selling side
  → RIDE SHORT. Positive = buying side → ride long.** This is **continuation, NOT mean-reversion**
  — we **never "buy the dip"** on a negative trandm (user, 2026-05-29: "I don't want to buy the dip.
  TrandM −100% just show that we on the selling side and we should ride selling").
- The magnitude toward ±100 = how strongly that side controls.
- **Oversell side (negative):** do **NOT** expect a big pump. At most a brief poke above the
  **5m UpBig** for a couple of hours. Critically, the oversell side is **spike-DOWN prone**:
  more potential for **short-direction spike candles of >2% within a single 1m candle** (dump spikes).
- (Mirror, by symmetry: overbought side = pump-spike prone, sustained dump capped.)
- So TrandM is **contrarian for sustained continuation but pro-spike in its own direction's
  stretch**: deeply oversold → the violent move to fear/expect is a *down* spike, not a rally.

### Asset_01 — power-to-move gauge ($ to shift TrandM by 1%)
- The **$ amount that must be bought or sold to move TrandM by 1%.** A proxy for the *power of
  trades* / effective depth: how much capital it takes to shift the trend.
- **For ETH: peak ≈ $20M, floor ≈ $4M.**
  - **Peak (~$20M):** a huge movement was just absorbed → expect a **return to normal
    volatility** (the move is largely spent; capital is heavily committed).
  - **Low (<$5M):** the token is **very calm / thin** → a **sudden pump or dump** is likely
    (little capital is needed to move price; the book is coiled).

### Combined read (the dump thesis, worked example — 2026-05-29 ~16:40)
> "When I say I expect a dump: TrandM is **−67%** (oversell side, so I don't expect a big pump),
> and on 4h TF it **wasn't a big dump recently**, so I don't expect a fast pullback."

1. TrandM −67% → oversold → **no big pump expected**, and **down-spike-prone**.
2. asset_01 low/calm → **a sudden move is loaded** (and given the oversold stretch, the
   asymmetric risk is a *down* spike).
3. 4h TF shows **no recent big dump** → there is **nothing to mean-revert up from** → **no fast
   bullish pullback expected**. The dump hasn't happened *yet*.
4. ∴ Expect a **dump / down spike**, not a sustained rally → **hold the short**.

This is exactly the regime in which the engine's current behavior is **wrong**: it covered the
short on a relief bounce (`sudden-up`) and auto-opened a long at the top — fighting an
oversold, down-spike-prone, calm-and-coiled market.

## State (proposed, once data source is wired)
```rust
// NOT yet in ModuleInput — pending data-source identification
pub trandm: f64,    // -100.0 .. +100.0  (oversell .. overbuy)
pub asset_01: f64,  // $ to move trandm 1%  (ETH: ~4M floor .. ~20M peak)
```

## State Transitions
N/A yet — external, read-only gauges. Would be refreshed per tick from the analysis source.

## Signals Derived (proposed for v0.53+)
| Condition | Read | Proposed engine effect |
|-----------|------|------------------------|
| `trandm <= -50` (oversell) | down-spike prone, pump capped | **Suppress auto-open LONG**; **do not `sudden-up`-cover a short** on a relief bounce |
| `trandm >= +50` (overbought) | up-spike prone, dump capped | Suppress auto-open SHORT; don't `sudden-down`-cover a long on a dip |
| `asset_01 < 5M` (calm/thin) | sudden move loaded | Raise conviction for a breakout/spike entry in the stretched direction |
| `asset_01 ≈ 20M` (peak) | move spent | Expect normal volatility → favor mean-reversion / fade, not breakout |
| oversell + calm + no recent 4h dump | **dump loaded, no pullback** | **Hold short as primary; block long flips** |

## History of Changes
### v0.54.0 — 2026-05-30
Added the **trandm-driven short** (`tm-s`) in `compute_auto_open`, independent of tf5 regime /
mkt_mode — built from a live miss (17:49–17:55): v0.53 sat flat through a sustained ~0.7% drop
(2040→2025) with trandm −70 because every short entry was gated on tf5.regime (TUP) / mkt_mode
(Sideways), both of which lag. Trigger: `trandm ≤ −50 && tf1.regime == −2 (1m TDN) && cross1m < 0
&& pzone > 0.15`. **Anti-fakeout:** requires a *confirmed 1m downtrend* (the 17:21 single-candle
flush to 2037 reverted and never built one → would not have fired). Robust but somewhat late
(1m TDN confirmed ~2029 in the live case). Thresholds first-pass/tunable. Symmetric trandm-long
deferred.

### v0.53.0 — 2026-05-30
Implemented the first behavioral use of trandm: (1) **fixed the inverted `compute_bias`** → ride-the-side, symmetric `TRANDM_MILD=25` / `TRANDM_STRONG=50`; (2) **block counter-trend auto-opens** when `|trandm| ≥ 50` (all 4 open sites); (3) **trandm side-hold** gates the `sudden-up`/`sudden-down` panic covers (`|trandm| ≥ 25`) so a relief bounce/dip no longer covers an aligned position; (4) **wired `asset_01`** into state + `read_analysis` + debug (`tm=`,`a01=`), behavioral gate deferred until calibrated. Built clean on wasm32 (zero warnings); awaiting deploy.

### v0 (concept captured) — 2026-05-29
User introduced `trandm` + `asset_01` as analysis values (separate from the indicator feed)
after the engine twice flipped a short into a wrong-way long on a relief bounce. Live read:
**TrandM −67% (oversell), 4h shows no recent big dump → dump expected, no fast pullback.**
Documented the interpretation; **flagged the open question of the data source** so we can wire
these into the module input for v0.53 (suppress auto-long + don't auto-cover shorts when
oversold/down-spike-prone). Until then, used only as monitoring context.
