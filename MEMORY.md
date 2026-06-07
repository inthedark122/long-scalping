# Long Scalping — AI Memory

> NOTE: Sections below "Module Architecture" are PRE-REFACTOR (v0.11 grid/hedge era) and
> largely stale. The current strategy is the long-only window-scalp on branch
> `test/fastcandle-scalp`. The section directly below is the current source of truth for
> the platform / bot groups.

## Sinvoid Platform & Bot Groups (current — 2026-06-07)

Trading runs on the **sinvoid** platform (`trading.sinvoid.me`) via the `mcp__claude_ai_LTE__*`
MCP tools. OKX live indicator/price API:
`https://okx.tg.trading.sinvoid.me/api/indicators?symbol_key=OKX%23<SYM>%23<TYPE>&timeframe=<sec>`.

- **Module**: `20dd415e-a179-4cc1-a7eb-a42cbcfd9f1e` (DynamicModule). Latest version:
  **`0.70-linebreak`**. Branch `test/fastcandle-scalp`. Deploy = build server-side (get_module_upload_url
  → curl PUT → submit_module_build → poll Success) then update_bot_group BOTH
  `strategy.module_version` and `strategy.professional.dynamic_module.module_version`.
- **API keys** (`list_api_keys`): **8 = OKX "inthedark1"** (used by both groups), 3 = BingX, 1 = Binance.
- **User manages bot STATUS + API_KEY MANUALLY** — do NOT change a group's status/api_key; create/configure structure only.

### Bot groups

| ID | Name | Status | Money | Margin/Lev | Symbols | Notes |
|----|------|--------|-------|-----------|---------|-------|
| **29** | Long Scalping | Simulation | **sim** | $10 / 20× per start | ETH, HOME, HYPE, ALLO, OPN | FC dev/test group. Bots 694 ETH, 919 ALLO, 921 OPN running on **0.70-linebreak**; 918 HYPE, 917 HOME stopped. |
| **39** | [PROD] Long Scalping | Active | **REAL** | OPN 50× / others 10× | ETH, HOME, ALLO, OPN | Production on **0.70-linebreak**. **OPN LIVE** (bot 923, $0.75 / 50×, budget ~$37.5) since 2026-06-07. ETH/HOME/ALLO 10× NOT started. REAL money. |

- **Start a bot**: `run_bot_group_action(<gid>, "start", <symbol_key>, <price>, leverage, margin, values)`.
  `values` = the start signal_action params (margin_sl, sl_margin_pct, seed, cap, cooldown_ticks,
  nearest, enable_stairs, enable_trend). FC params (enable_fcscalp default ON, fcscalp_*) are NOT in
  the group's start UI params, so they apply via the module defaults on param-absent starts.
- **Stop**: `run_bot_action(<bot_id>, "lte:stop")`. **Force close**: `run_bot_action(<bot_id>, "force_close_long")`.
- Config loads ONCE per start (cfg_loaded guard) → to re-apply config, lte:stop then start.
- **Backtest**: `run_bot_group_backtest` — any symbol works (not just the group list). FC opens
  with MARKET in backtest (no order book) so it fills; live it's maker LIMIT. FC backtests are
  SLOW (high per-tick order volume + first-time per-token/period data load).

### FC (window scalp) — current design (arb21)
Window/cascade-only (order book = disabled `fcscalp_use_ob` extension). **1m cascade = entry
window; 5m cascade = confirmation** (safe gate + bias). Per-RUNG TPs (each rung exits at its own
entry × (1+tp_dist)). Rung spacing = fraction (`fcscalp_rung_step`) of last-fill→floor distance
(clusters near the bottom, widens for a new lower window). Entry capped at the buy-zone line
(lower window) so it never buys the top. Confirmed downtrend (`mkt_mode == -2`) → tight TP + no
averaging + bag-exit on pump/wide-window; up/sideways → full accumulate.
**Two entry paths:** (1) dip-buyer — opens in the LOWER half of the window (`pos_frac < buy_zone`),
maker limit; (2) ZigZag LINE-BREAK (`fcs-bk`, fixed 0.70-linebreak) — opens at the START of an up-move
when a 1m candle CLOSES above the PREVIOUS candle's SellPrice line (`zz_break_up`, detected on_indicator
in update_zigzag). MUST use the *previous* SellPrice: the zigzag bumps SellPrice up the instant the line
breaks, so the current value never shows a cross (this was the bug — old code compared LIVE sug price to
the CURRENT/static-within-candle value and missed every pump). Direction is the "strong move" (HH+HL)
state and only filters (excludes SHORT) — the line-break LEADS Direction=LONG, so NOT gated on dir==LONG.
MARKET entry, 1.5× rung, upper window, `mkt_mode!=-2`. Both route to OWN_FC per-rung TPs. Cfg:
`fcscalp_zz_breakout`(on), `fcscalp_zz_seed_mult`(1.5). Debug WIN: `zz[sell.. buy.. d±.. k.. BRK]`.
**Orphan/freeze fix (0.69-rearm):** a resting `fcs-l` entry stranded below a relocated (up) window floor
can't fill AND (fc_open_px>0 gates every FC entry) BLOCKS the dip-buyer + breakout = frozen symbol. Now
cancelled (`fcs-rearm`) when `fc_open_px < win.floor` → re-arms in the new window. Also `dca-l` (base
resting buy) now cancelled when flat (was leaking stale entries). Review TODO (not yet done): breakout
should preempt a resting dip (drop fc_open_px gate on the breakout).
**Deep-dump hold-for-bounce (0.66-deep):** a SUDDEN dump into the 30m deep bands (`tf30.cross <= -3`)
is a potential FLIP zone — don't force-cut at the low. Arms on (sudden_dump & 30m-deep & underwater);
while held it SUPPRESSES the FC cuts (`fcs-sl`/`fcs-dn-exit`), stops averaging, and sizing rests ONE
reachable reduce-only TP at `avg×(1-deep_neg_pct)` — ALLOWED below avg entry (breakeven / small loss)
so the quick bounce closes us near flat. Bounded: holds `deep_hold_candles`(~6); if no recovery →
cut (`fcs-deep-stop`). Margin SL still backstops. Cfg: `fcscalp_deep_hold`(on), `_deep_cross`(-3),
`_deep_neg_pct`(0.005), `_deep_hold_candles`(6). Debug WIN line: `dh{0/1} t30c{tf30.cross}`.
**Static grid fix (0.67-grid / 0.68-grid2):** the `fcs-g` rung price is now computed ONCE in the
engine (anchored to the actual rung level + window floor) and FROZEN, emitted exactly once
(`fc_grid_placed` guard); recomputed only on a fill (advance) or a partial-TP-with-window-shift
(restart). Fixes the old per-tick live-price recompute that produced same-price / rising rungs and
duplicate stacked orders. 2wk backtest (ETH+HOME+ALLO+OPN, $100/20×, May22–Jun5): total **−3.4 → +13.9**
(HOME −1.3→+15.1, OPN −2.7→−1.9, ALLO +0.66→+0.94, ETH −0.08→−0.24). HOME +15 is a BACKTEST number
(compounding + issue-001 under-reports losses + taker fees) — directional, not a promise. Minor: ~2
same-price pairs / 50 positions remain from the restart edge case (refinable). Sectioned debug via
`debug_info` bitmask: 1 CORE · 2 CAS · 4 WIN · 8 RUN · 16 FC · 32 VT · 64 MRC · 128 DMP (0 = default CORE|CAS|WIN).
Live (maker fees) is meaningfully better than backtest (taker market opens). Live sim: OPN +,
ALLO green on bounces; base stairs/trend (auto-l) is the main loss source.

## Module Architecture

- **Type**: DynamicModule (WASM via `lte_strategy_bridge::run_loop`)
- **Module ID**: `20dd415e-a179-4cc1-a7eb-a42cbcfd9f1e`
- **Bridge**: `lte_strategy_bridge` (persistent stdin/stdout loop)
- **Events**: SugInfo (~1s), Indicators (candle close), NewPosition, FinishPosition, OrderUpdate, Action

## Movement Detection System (v0.11.0+)

Debug-only analysis to detect slow up/down price movement. NOT used in trading logic — purely for observation and future development.

### Signals Used

| Signal | Indicator | Field | What it tells us |
|--------|-----------|-------|------------------|
| **MRC Angle** | `mrc_200` | computed from MeanLine[1] vs MeanLine[0] | Channel slope direction and speed. Positive = rising, negative = falling. Expressed as % change. |
| **Supertrend Direction** | `supertrend` | `Direction` | Long/Short trend on the 1m timeframe |
| **Supertrend Distance** | `supertrend` | `Supertrend` (line price) | How far price is from the supertrend line. Computed as `(price - st_line) / price * 100`. Positive = above line, negative = below. |
| **Supertrend KLines** | `supertrend` | `KLines` | How many candles since the last supertrend direction change. Higher = more established trend. |
| **Volume BuySellGain** | `volume_50` | `BuySellGain` | Buy/sell volume spike relative to last 50 candles. `+2.0` = buy volume 200% above normal. `-2.0` = sell volume 200% above normal. |

### Movement Score (`mov`)

Composite score from **-100** (strong down) to **+100** (strong up). Measures **sustained directional movement** — price trending in one direction without breaking opposite MRC bands.

**Key principle:** Movement means price is moving directionally in a controlled way. Volume spikes (high |BuySellGain|) signal the **end** of movement (liquidation cascades), not movement itself.

#### Score Components

| Component | Weight | What it measures |
|-----------|--------|------------------|
| **Band depth** | ±40 max | How deep into one side of the MRC channel price sits. `(pzone - 0.5) * 80`. Price above ML in uptrend = positive, below ML in downtrend = negative. |
| **Angle agreement** | 1.0–1.5× multiplier | MRC angle confirms the band direction. Same direction → boost up to 1.5×. Opposing → halve to 0.5×. |
| **Supertrend confirmation** | ±30 max | Supertrend direction agrees + duration (KLines). Up to ±30 when confirmed for 30+ candles. No contribution if direction disagrees. |
| **NATR tightness** | 0.5–1.5× multiplier | Small NATR = tight channel = controlled move = more significant. `(0.2 / natr)` clamped 0.5–1.5. |
| **Volume spike penalty** | 0.3–1.0× multiplier | High |BuySellGain| > 1.5 dampens score. Signals potential end of movement, not continuation. |

#### Formula

```
band_score = (pzone - 0.5) * 80                          # ±40 max
angle_factor = if angle agrees: 1 + min(|angle|*100, 0.5) # 1.0–1.5
               if opposes: 0.5
st_score = st_dir * min(klines/30, 1) * 30               # ±30 if confirmed
natr_factor = clamp(0.2 / natr, 0.5, 1.5)
bsg_penalty = if |bsg| > 1.5: max(0.3, 1 - (|bsg|-1.5)*0.3)

movement = (band_score * angle_factor + st_score) * natr_factor * bsg_penalty
           clamped to [-100, +100]
```

#### Interpretation

| Score | Meaning |
|-------|---------|
| +70 to +100 | Strong sustained uptrend — price high in channel, angle up, supertrend Long, low volatility |
| +30 to +70 | Moderate uptrend — signals mostly agree |
| -30 to +30 | Sideways / mixed signals |
| -70 to -30 | Moderate downtrend |
| -100 to -70 | Strong sustained downtrend |

**Examples:**
- `mov=+65`: price at UpSmall, angle positive, supertrend Long 20 candles, small NATR, no volume spike
- `mov=-20`: price slightly below ML, angle slightly negative, supertrend recently flipped Short
- `mov=+45` dropping to `mov=+15`: BuySellGain spiked to 2.0, dampening the score — possible end of uptrend

### Debug Output Format

**Indicators event:**
```
ind t=42 rng=0.85 ang=0.0023 smc=1 st=L(0.12% k=15) bsg=0.45 natr=0.082 mov=55 filled=3
```

**SugInfo event:**
```
sug t=43 p=2520.1234 ntps=85 rng=0.85 pz=0.62 db=2480.12 ml=2520.12 ub=2560.88 ang=0.0023 st=L(0.12% k=15) bsg=0.45 natr=0.082 mov=55 ind_age=1 ln=12.5 sn=11.2 ...
```

### Key debug fields:
- `st=L/S/-` — Supertrend direction (Long/Short/unknown)
- `(0.12%)` — distance from supertrend line as % of price
- `k=15` — candles in current supertrend direction
- `bsg=0.45` — BuySellGain (positive=buy pressure, negative=sell pressure)
- `natr=0.082` — Normalized Average True Range (volatility)
- `mov=55` — Movement score (-100..+100)
- `ang=0.0023` — MRC angle (channel slope)
- `ind_age=1` — ticks since last indicator update (staleness)
- `pz=0.62` — Price zone (0=DownBig, 1=UpBig)
- `db`/`ml`/`ub` — MRC band boundaries

### Future Development Ideas

1. **Movement-aware allocation**: When `mov` is strong, increase allocation to trending side and reduce counter-trend entries
2. **Early exit signal**: When `mov` drops sharply (BuySellGain spike), close the losing side before SL
3. **Grid spacing**: Widen grid in strong movement (fewer counter-trend fills), tighten in sideways (more mean-reversion profit)
4. **Movement threshold for re-entry**: Delay re-entry on the losing side when `|mov|` is high

## Config Parameters (v0.11.0)

| Param | Default | Description |
|-------|---------|-------------|
| `breakout_guard` | **false** | Block counter-trend entries during SMC break + NTPS > threshold |
| `imbalance_guard` | **false** | Cap one side at N× the other side's notional |
| `margin_sl` | true | Enable margin-based stop loss |
| `imbalance_ratio` | 3.0 | Max ratio for imbalance guard |
| `breakout_ntps` | 200 | NTPS threshold for breakout guard |
| `sl_margin_pct` | 0.9 | SL triggers at this % of margin lost |
| `seed` | 0.05 | Seed position size ($ amount) |
| `cap` | 0.90 | Max fill ratio before stopping entries |
| `cooldown_ticks` | 5 | Ticks to wait after position close before re-entry |
| `nearest` | 2 | Number of nearest grid levels to place orders at |

**Default config matches v0.10.0 behavior** (guards off, same constants).

## Grid Mechanics

- 9 bands per side (MRC levels + midpoints) = 17 unique price levels per side
- 2 nearest candidates selected per tick, placed as limit orders
- Fill tracking via `mark` field (`slot:band` format, e.g., `l-e1:l-db`)
- Filled bands skipped until position closes (FinishPosition clears fills for that side)
- Mark slots: `l-e1`, `l-e2`, `l-ext` (entries), `l-t1`, `l-t2` (TPs), same for `s-*`

## State Persistence

Only critical restore-after-restart fields are serialized:
- `ticks`, `long_close_tick`, `short_close_tick`
- `long_needs_reentry`, `short_needs_reentry`
- `long_entry_idx`, `long_tp_idx`, `short_entry_idx`, `short_tp_idx`

MRC bands, indicators, and fill set are rebuilt from live data on restart.

## Known Issues

- **Imbalance guard `< 1.0` exemption**: When one side is at seed level (~$0.09), the other can grow without limit. Needs rework if guard is enabled.
- **SL not firing**: Observed on EDEN at -$7.55 past -$6.16 threshold. Root cause unclear — possibly PnL lag, leverage mismatch, or config not applied. Enhanced debug (v0.10.3+) should help trace.
