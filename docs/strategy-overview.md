# Long Scalping — Strategy Overview

A public-facing summary of the inputs, parameters, and timeframes the strategy
module consumes. Figures are taken directly from the live module source
(`module/src/engine.rs`, `module/src/state.rs`), not estimated.

> **One-line version:** 4 candle indicators (MRC-200 channel, SMC, OHLC, Volume)
> + 2 real-time analysis streams (NTPS, spike-detector),
> across 5 timeframes (1m / 5m / 15m / 30m + live tick), with 16 tunable
> parameters and risk-based position sizing.

---

## ⏱️ Timeframes — 5

| Timeframe | Role |
|-----------|------|
| **1-minute** | Primary execution & entry/exit zones |
| **5-minute** | Trend regime + profit-target structure |
| **15-minute** | Higher-TF context |
| **30-minute** | Higher-TF context |
| **Real-time tick (~1s)** | Sub-candle order-flow analysis stream (Projection feed) |

---

## 📊 Indicators — 8 distinct inputs

### From candle data (4)

1. **MRC — Mean Reverse Channel (200)** — 9 price bands
   (MeanLine ± Inner / Small / Outer / Big on each side) plus a live zone-cross
   level. This is the core structure indicator.
2. **SMC — Smart Money Concept** — directional bias (Long / Short).
3. **Candle OHLC** — High / Low / Close / Direction.
4. **Volume-50** — Gain and Buy/Sell-Gain (BSG), each with a rolling 50-candle
   min/max envelope so "how big is big" adapts to recent volatility.

### From the real-time analysis feed (2)

5. **NTPS** — trade-activity rate (average trades per 3 seconds).
6. **Fast-candle detector** — real-time momentum-spike events.

---

## ⚙️ Parameters — 16 tunable

- **4 feature switches** — grid / stairs / trend / hedge entry modes.
- **9 numeric knobs** surfaced in the launch dialog — imbalance ratio,
  breakout-NTPS, SL-margin %, seed size, cap, cooldown ticks, nearest-orders,
  plus the breakout / imbalance guards.
- **3 risk controls** — `risk_pct`, `stop_pct`, and margin-SL — which drive
  **risk-based position sizing**: size = (risk budget ÷ stop distance), hard-capped
  at `max_amount ÷ leverage`. The bot never risks more than its configured margin.
  range floors, V-pattern freshness window, EMA smoothing α = 0.20, etc.).

---

## 🧠 Derived signals built on top

From those 8 inputs the engine continuously maintains:

- **Per-timeframe trend regime** (Sideways / Trend-Up / Trend-Down) on each of 1m/5m/15m/30m
- **Market-mode classifier** (Unknown / Sideways / Trend-Long / Trend-Short)
- **V-pattern statistics across all 9 MRC lines** (support/resistance memory)
- **Movement-cascade tracker** (absorbs counter-moves, records break levels)
- **Fast-candle event history** (rolling EMA of spike size, duration, momentum)
- **Risk ratchet** (scales entries down in drawdown)
- **Max-Favorable-Excursion (MFE) per side** (protects banked gains on reversal)

So the *visible* input count is deliberately small, but the internal state derived
from it is rich.

---

*Strategy module: `long-scalping`. This document describes inputs and
configuration only — it does not disclose entry/exit decision logic.*
