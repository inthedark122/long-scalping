# Long Scalping

> **📋 Start with [`HANDOFF.md`](./HANDOFF.md)** — the authoritative project state, architecture,
> build/deploy/backtest workflow, and critical gotchas (notably the backtest PnL bug). The next-feature
> design is [`docs/regime-modes.md`](./docs/regime-modes.md).
>
> **⚠️ The deployed version is now `v0.61-long` (LONG-ONLY)** — short + hedge code removed. The
> delta-neutral both-sided description below is the *legacy* design (preserved on branch `v0.61-hedge`).

Delta-neutral grid strategy on ETH-USDT perpetual swap. Holds simultaneous LONG and SHORT positions, scaling into each side via limit orders placed at MRC (Mean Reversion Channel) band levels. Trend bias dynamically adjusts order sizes to favor the side aligned with market direction.

## Key Settings

| Parameter | Value |
|-----------|-------|
| Exchange | OKX |
| Symbol | ETH-USDT-SWAP |
| Margin Mode | Cross, 100x leverage |
| Margin | $5 |
| Max Open Amount | $500 |
| Max Active Bots | 1 |
| Direction | BOTH |
| Signal Timeframe | 1m (60s) |
| Timeframe Line | 4h |
| Timeframe Movement | 5m |

## Indicators

| Indicator | Period | Timeframe | Purpose |
|-----------|--------|-----------|---------|
| MRC | 200 | 1m | Band levels for grid placement (DownBig/DownOuter/DownSmall/DownInner/MeanLine/UpInner/UpSmall/UpOuter/UpBig) |
| NATR | 14 | 1m | Volatility — extends L2/S2 grid beyond outer bands |
| SMC | - | 1m | Direction signal for momentum-based position closing |
| Analysis.Ntps | - | - | Net trade pressure score — controls TP width and momentum closes |
| Analysis.Trandm | - | - | Trend momentum (0-100) — amplifies/dampens bias multipliers |

## Variables

| Key | Name | Default | Purpose |
|-----|------|---------|---------|
| `init` | Init | 0 | Initialization flag — 0 = first run, 1 = running |
| `half` | Side Alloc | MaxAmount * 0.5 | Capital allocated per side ($250 of $500) |
| `la` | Long Amt | 0 | Tracked LONG position amount (for fill detection) |
| `sa` | Short Amt | 0 | Tracked SHORT position amount (for fill detection) |
| `lf` | Long Fills | 0 | Count of LONG fills since last reset |
| `sf` | Short Fills | 0 | Count of SHORT fills since last reset |
| `angle` | MRC Angle | 0 | MRC MeanLine slope (candle[1] vs candle[0]) |
| `mrc_range` | MRC Range | abs(DownBig - UpBig) | Channel width in price % terms |
| `pzone` | Price Zone | PercentageBetween(DownBig, UpBig, Price) | Where price sits in the channel (0.0 = bottom, 1.0 = top) |
| `tp_pct` | TP Pct | mrc_range * 0.3 | Dynamic take-profit % |
| `lbias` | Long Bias | 1.0 | Multiplier applied to all LONG order amounts |
| `sbias` | Short Bias | 1.0 | Multiplier applied to all SHORT order amounts |

## Strategy Logic

### 1. Initialization (init == 0)

Opens seed positions on both sides at 5% of `half` allocation, sets `init = 1`, and snapshots position amounts into `la`/`sa`.

### 2. Indicator Computation (every analysis tick, init == 1)

- **angle** = PriceMeasure(MRC MeanLine[1], MRC MeanLine[0]) — positive = rising, negative = falling
- **mrc_range** = abs(MRC DownBig - MRC UpBig) — full channel width as % of price
- **pzone** = PercentageBetween(DownBig, UpBig, Price) — normalized price position
- **tp_pct** = adaptive:
  - Default: `mrc_range * 0.3`
  - Ntps > 500 (high pressure): `mrc_range * 0.5` (wider TP, let winners run)
  - Ntps < 100 (low pressure): `mrc_range * 0.15` (tighter TP, take what you can)

### 3. Bias Adjustment

Starts at 1.0/1.0. Two layers of adjustment are applied sequentially:

**Layer 1 — MRC position:**
| Condition | lbias | sbias |
|-----------|-------|-------|
| angle > 0.005 AND pzone > 0.6 (uptrend, price high) | 0.7 | 1.5 |
| angle < -0.005 AND pzone < 0.4 (downtrend, price low) | 1.5 | 0.7 |

**Layer 2 — Trandm momentum (multiplicative on top of Layer 1):**
| Condition | lbias | sbias |
|-----------|-------|-------|
| Trandm > 70 (strong up) | *= 0.7 | *= 1.3 |
| Trandm < 30 (strong down) | *= 1.3 | *= 0.7 |

The bias makes the strategy accumulate more on the side that will profit when price reverts.

### 4. Grid Orders — Two Layers per Side

Each side has two independent order layers (L1/L2 for LONG, S1/S2 for SHORT). Orders are LIMIT type, placed at MRC band levels. Amount = `half * weight * bias`.

**LONG grid (L1 + L2) — buy orders below price:**

| Band | L1 Weight | L1 Cap | L2 Weight | L2 Cap |
|------|-----------|--------|-----------|--------|
| Below DownBig (by NATR) | — | — | 15% | 85% |
| DownBig | 30% | 85% | 30% | 55% |
| DownOuter | 25% | 55% | 25% | 30% |
| DownSmall | 15% | 30% | 15% | 15% |
| DownInner | 10% | 15% | 10% | 5% |
| MeanLine | 5% | 5% | — | — |

**SHORT grid (S1 + S2) — sell orders above price:**

| Band | S1 Weight | S1 Cap | S2 Weight | S2 Cap |
|------|-----------|--------|-----------|--------|
| Above UpBig (by NATR) | — | — | 15% | 85% |
| UpBig | 30% | 85% | 30% | 55% |
| UpOuter | 25% | 55% | 25% | 30% |
| UpSmall | 15% | 30% | 15% | 15% |
| UpInner | 10% | 15% | 10% | 5% |
| MeanLine | 5% | 5% | — | — |

Orders are only placed when:
- Price is on the correct side of the band (above for LONG, below for SHORT)
- Current position amount is below the cap for that level

When position amount exceeds 84% of `half`, all pending orders (L1+L2 or S1+S2) for that side are removed.

### 5. Fill Tracking

When position amount increases beyond the tracked `la`/`sa`, the fill counter (`lf`/`sf`) increments and the tracker updates.

### 6. Take Profit

Closes the entire side when: `PnL > Amount * (tp_pct / 100)`

Requires Amount > $1 to avoid closing seed positions. Resets amount tracker and fill counter on TP.

### 7. Stop Loss

Closes the entire side when: `PnL < -Amount * (mrc_range / 100)`

The stop loss is the full MRC range width — a complete channel traverse against the position.

### 8. NTPS Momentum Close

When Ntps > 500 (extreme trade pressure):
- If SMC Direction == LONG: close the SHORT side (momentum strongly favors longs)
- If SMC Direction == SHORT: close the LONG side (momentum strongly favors shorts)

### 9. Re-entry

When a side has been closed (la == 0 or sa == 0) and Ntps drops below 400 (pressure subsiding), re-open a seed position at 5% of `half`.

### 10. On Finished (bot restart)

Resets all trackers, opens fresh seed positions on both sides at 5% of `half`, and sets `init = 1`.
