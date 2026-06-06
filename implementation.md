# Long Scalping — Implementation Plan

Derived from live session analysis (2026-05-26 / 2026-05-27). Issues ranked by severity.

---

## Priority 0 (NEW #3, 2026-05-30 PM) — Observability: runtime debug-info builder

Context: during live monitoring the V-tracker (`vt=`) was unreadable for a whole session because every `last_debug` snapshot the MCP served was an `ind` line (no `vt=`; it only rode `sug` lines) and the snapshot refreshes ~1/min. See `docs/debug-info-builder.md`.

### O.2 — Hedge-as-insurance redesign (pump-gated, pullback-placed, primary-stays-primary) — ✅ IMPLEMENTED v0.58.0 (built, not deployed)
The v0.49 market hedge fired twice live on transient `mkt_mode` blips (NTPS ~20), market-buying the range top against `trandm −68` and orphaning a losing long. User kept the hedge but redefined it: open at the **best price** (pullback-limit, not market) and only on a **real pump** (high NTPS). Built: pump gate (`ntps_mult ≥ hedge_ntps_mult`=1.6 + fast-candle + `mkt_mode`), pullback-limit entry (`cur_hi−fc_dist`/`cur_lo+fc_dist`, trails/auto-cancels), `hedge_side` temp leg (primary never invalidated) closed on pump-resolution (`hedge-cl`/`hedge-cs`), `cooldown_ticks` anti-churn. New config `hedge_ntps_mult`. Old `invalid_side`/`neg_exit` left inert. See `docs/hedge-protection.md`.

### O.1 — `debug_info` runtime preset switch — ✅ IMPLEMENTED v0.57.0 (built, not deployed)
`debug_info` bitmask config (default 0; `1 V · 2 MRC · 4 BIG · 8 CAS · 16 DMP · 31 ALL`). `debug_builder(state, sel)` appends the selected full-detail blocks to `init`/`ind`/`sug` lines so any view is readable from `list_bots` last_debug. `sug` gets `sel & !1` (no duplicate `vt=`). Echoed in cfg ack (`dbg=`), exposed as a Start param. Switching = config push applied next tick (context accumulation) → two switches for a focused read (preset → read → 0). One-shot request action **cut** per user ("we cannot request something from the bot" — future work); builder is request-ready.

---

## Priority 0 (NEW #2, 2026-05-30 PM) — Per-type auto-scaling + Big-line reaction bracket (live monitoring directives)

Context: while watching the live v0.54 `auto-s` short (single TP at DownBig, fixed 0.008 size), the user gave two refinements to the (still-undeployed) v0.55 sizing/TP work. See `docs/auto-scaling-per-type.md`, `docs/big-line-reaction-bracket.md`. These fold into the next build (v0.56).

### 0.D — Per-trade-type auto-scaling under one risk budget — ✅ IMPLEMENTED v0.56.0 (built, not deployed)
> `budget_size()` + `uncertainty_damp()`; every entry & DCA re-based to a fraction of `R_side − notional`; risk_pct kept 0.40, de-risk dynamic. See `docs/auto-scaling-per-type.md`.
**Directive (user, 2026-05-30):** "we still in the unpredictable situation, so maybe we can make the risk less. but we should be able to use auto-scale for ALL types of trading — even stairs or trend — using **own calculation** of auto-scaling. so each type of trading should have its own auto-scaling solution."
**Fix direction:** one `R_side = risk_sized_notional(...)` budget per side (≤ max_amount); each entry type (seed/trend/stairs/tm/re/spike) draws a fraction `f_type` of *remaining room* via its **own curve** (trend→trend-strength, stairs→depth, tm→|trandm|, re→ML-angle, spike→spike-size); global `uncertainty_damp` (0.6–0.8× in Sideways/UNK or thin `asset_01`) + `risk_drawdown`; hard invariant `current + new ≤ min(R_side, max_amount)` so a full scale-in never exceeds the budget. Lower `risk_pct` to de-risk the whole envelope for the unpredictable regime. (Refines 0.A — replaces the uniform `risk_sized_notional` with per-type curves.)

### 0.E — Big-line first-reaction bracket (replace single DownBig TP) — ✅ IMPLEMENTED v0.56.0 (built, not deployed)
> 4-order bracket (split 1×+2× overshoot) gated on `!*_big_tested`; STRICT profit-only per rung (`tp_is_profitable` + `exp_pnl>0` + `reduce_only`) so stairs around Big never bank a loss. See `docs/big-line-reaction-bracket.md`.
**Directive (user, 2026-05-30):** "currently you place a single limit order just on the down big. this is wrong. I expect at least 3 orders — middle between big and outer / slightly below big / below big — but only the **first time** before we cross Big (unknown reaction). Two reactions: **just touch**, or **fast candle 1–2× the Outer→Big distance**."
**Fix direction:** while `!side_big_tested`, replace the `mode==1` single 90% DownBig TP (`engine.rs:1228-1233`) with a bracket: rung1 `(O+B)/2` (pre-touch), rung2 `B−0.15·d` (touch), rung3 `B−(1…2)·d` (fast-candle overshoot), `d=DownOuter−DownBig`; collapse to normal continuation/extended-TP once Big is tested. Mirror for longs at UpBig. (Refines 0.B — gives the "don't cap the fly" rule a concrete first-touch order structure.)

---

## Priority 0 (NEW, 2026-05-30) — Position building + breakout TP (from the v0.54 `tm-s` live test)

Context: v0.54's `tm-s` trandm-short worked — it rode ETH 2029.68 → 2008.9 (~1.0%) on a clean down-leg v0.53 would have missed. But two critical gaps showed up. See `docs/trend-target-5m-ml.md`, `docs/trandm-asset01.md`, memory `feedback_risk_based_sizing`.

### 0.A — CRITICAL: position stays tiny while the trend runs (under-sizing the winner) — ✅ IMPLEMENTED v0.55.0
> `risk_sized_notional` applied to all primary entries; `stair-s/l` build toward the cap on continuation. Refinement open: pullback-add ladder (vs the down-line stairs) and tuning risk_pct/stop_pct live.
**Problem (user, 2026-05-30):** "We are holding a small amount of position even when the price is dumping — this is critical." The `tm-s` short entered 0.002 ETH (~8% of the `max_amount` cap) and rode a full ~1% move with that tiny size → tiny profit on a big, correct move. The scale-in (v0.55 pullback ladder) never fired because the price broke *straight down* (no pullback bounce to add into) — so on a clean continuation we never built size at all.

**Fix direction:**
- **Pyramid into a confirmed trend**, not only on pullbacks. Add as the move *continues* in our favor — on new extremes / breaks of the next MRC level (e.g. break of 1m DownInner/DownBig, or each 5m band crossed) — AND on pullbacks. Build toward the risk-sized target promptly while the thesis (trandm side + zigzag + TDN) holds.
- **Size by the risk calc, capped at `max_amount`** (memory `feedback_risk_based_sizing`): `Q_total = min((max_margin × risk%)/(stop−avg), max_amount/price)`. Reach a meaningful fraction of the cap quickly when the trend is confirmed — don't trickle.
- Guard with the stop so a full-built position that reverses is still bounded (the whole point of risk sizing).
- **Net:** when our directional thesis is live and running, be *big* (up to cap, risk-bounded), not tiny.

### 0.B — TP at MRC lines is wrong; ride breakouts (don't cap the fly) — ✅ IMPLEMENTED v0.55.0
> No line TP; ride held while the strong-trandm side controls, exit on regime-neutralize (`tm-exit`). 5m angle/hard-break state added. Refinement open: explicit block-rejection / NTPS-exhaustion exits + zigzag-flip (currently trandm-neutralize is the proxy).
**Problem (user, 2026-05-30):** "TP on the MRC lines is useless, because if price goes OUT of the 1m MRC Big it means price can fly for a big percentage. 5m shows we're still in the mid, so there are no blocks now to make −1% or more." Taking profit *at* a line (incl. the 5m ML) caps a winner exactly when a momentum breakout is starting. Corrects the earlier "5m ML = hard TP" framing in `docs/trend-target-5m-ml.md`.

**Fix direction:**
- **Detect the 1m MRC-Big breakout** (price exits the 1m outer band with momentum: fast-candle + NTPS surge + 1m cross at extreme) → treat as a *fly* signal: **do NOT TP**, let it run.
- **Use higher-TF structure for room/blocks, not as fixed TPs.** If the 5m position is mid-range (price far from the 5m DownInner/DownBig and ML), there is *no block* → big room → hold. The 5m ML is "room available," not a take-profit.
- **Exit on a real block or exhaustion, not a line:** an actual 5m band hit with rejection, momentum exhaustion (NTPS fade + run stalls), or a structure flip (zigzag `EndDirection` flips, trandm eases toward 0) — trail rather than fixed-line TP.
- Pairs with 0.A: big size + let it fly = capture the big-percentage move the strategy is built for.

### 0.C — Stairs entries at MRC down-lines + close-to-reenter at the 1m mean line — ✅ IMPLEMENTED v0.55.0
> `stair-s/l` at each new 5m down/up line; `re-s/l` re-enter on a soft 1m-ML touch with confirming ML angle. Refinement open: explicit close-to-reenter (vs re-enter-when-flat), per-line resistance reject/break check.
**Problem / expectation (user, 2026-05-30):** "I expect the bot can close the position and try to open at the **MRC 1m Mean Line**, or **wait for any of the MRC down-lines** to check what the resistance is and take it as a **high-priority for stairs trading**." Today the bot just holds one entry; it doesn't (a) re-enter at a better mean-line price, nor (b) treat the descending MRC lines as the stair-build ladder. (This is the concrete form of the deferred half of 0.1.)

**Fix direction:**
- **MRC down-lines = high-priority stair-add levels.** As price steps down through **DownInner → DownSmall → DownOuter → DownBig** (1m, and confirm vs 5m), at each line *check the level as resistance/support* (does it reject or break?) and **add a short stair** there (risk-sized, per 0.A), building the position into the move. These down-lines — NOT a fixed TP — are where size is added.
- **1m Mean Line = re-entry price.** When price pulls back UP to the **1m MRC ML**, that's the spot to (re-)open / add a short at the mean (the best counter-trend fill). The earlier rule stands: *close only to re-enter at a better price* (the 1m ML), never to bank a small gain.
- **Priority:** stair-at-down-line and re-enter-at-1m-ML should be **high priority** in `compute_auto_open` when trandm-side + trend agree — above the generic triggers.
- Composes with 0.A (size the stairs by risk to cap) and 0.B (don't TP at these lines — they're add/`check-resistance` levels; exit only on block-rejection / exhaustion / structure flip).

---

## Priority 0 — Critical Bugs (from 2026-05-29 session)

> **Status (v0.48.0, 2026-05-29):** 0.0 ✅ done · 0.1 ⚠️ partial (TP-hold done, add-stairs deferred) · 0.2 ✅ done (subsumed by s-ts SMC-only + reversal gate) · 0.3 ✅ done · Timing 0.1 ✅ done · Timing 0.2 ✅ done. See `docs/` and `module/CHANGELOG.md`.

### 0.0 Volume indicator collapses during quiet periods → false signals — ✅ DONE v0.48.0 (`docs/volume-normalization-floor.md`)
**Problem:** `vol_gain_max` and `vol_bsg_max` are computed as rolling maximums over recent candles. After a silent period (50+ quiet candles), both max values shrink toward zero. Any small volume event then reads as a large relative signal because `gain_pct = (vol_gain - vol_gain_min) / (vol_gain_max - vol_gain_min)` spikes to 1.0 even from a tiny absolute move. Observed: `gain=0.17` → `gain=1.89` in ~5 candles after a quiet sideways stretch, triggering stair TP logic prematurely.

**Symptoms:**
- s-ts TP fires on small noise bounces in sideways markets
- BSG exhaustion detection fires on tiny volume spikes
- `vol_exhausted` stays false (max < 0.5 threshold) → stair entries allowed when they shouldn't be

**Fix direction:** Add a minimum floor to the volume reference range so the normalization denominator can't collapse below a meaningful threshold. Options:
- `gain_range = max(vol_gain_max - vol_gain_min, GAIN_FLOOR)` where `GAIN_FLOOR = 0.5` or based on historical baseline
- Or use a slow-decaying EMA for vol_gain_max instead of a rolling window max, so quiet periods decay slowly rather than collapsing
- Also apply to vol_bsg_max/min for BSG normalization

### 0.1 Zone-change trading: stair signals are entries, not exits — ⚠️ PARTIAL v0.48.0 (`docs/zone-change-tp-hold.md`)
> Done: stop nibbling the winner — band TPs only fire after a ≥1-zone reversal toward ML; `s-ts` noise triggers removed. **Deferred:** the "add a stair (DCA) at each favorable boundary" half — needs grid/DCA path changes; planned for a later isolated version.

**Problem:** When price drifts from the upper MRC half (above ML) to the lower half (below ML), this is "zone-change trading" — the system should be BUILDING stair positions (DCA adding) at each zone boundary, not closing them. Currently the "top stair short signal" fires at the first zone-1 boundary crossing and partially closes the existing SHORT. This is wrong: crossing zone 0→-1 is a STAIR ENTRY trigger, not an exit trigger.

**Expected behavior:**
- SHORT entered at zone 0 (ML): first stair
- Price crosses to zone -1 (DownInner): add another SHORT stair (DCA)
- Price crosses to zone -2 (DownSmall): add another stair
- TP orders placed only AFTER stair accumulation is complete — when price reverses at the bottom zone back toward ML
- During the downward zone-change phase, all stair crossings = entries, not exits

**Fix direction:** Distinguish between "zone change mode" (gradual drift crossing boundaries) and "reversal mode" (price returning toward entry from extended zone). Only activate stair TPs in reversal mode. In zone change mode, stair signals should add to the position instead. Gate on: if price is moving in the same direction as the open position AND still crossing new zone boundaries → zone change mode → add, not close.

### 0.2 s-ts TP absolute distance too tight in sideways markets — ✅ DONE v0.48.0
> Resolved structurally: `s-ts` now fires only on an SMC flip (urgent, near-market). The premature small-distance partial-close in sideways no longer happens because the noise triggers were removed and band TPs are held until a ≥1-zone reversal.

**Problem:** (Secondary to 0.1) The stair trailing stop places TP orders at a distance that is too small for sideways/low-volatility markets. After a tiny dip below ML in a sideways range (not reaching DownInner), the TP fires and partially closes at a loss. The expected behavior is to wait for at least one MRC level crossing before engaging the TP, or to DCA-add on the bounce instead of closing.

**Fix direction:** Gate the initial TP placement on price having crossed at least one meaningful MRC level in the desired direction (e.g., for SHORT: price must drop below DownInner before TP is placed). In sideways mode, use a minimum distance floor tied to the MRC range (e.g., TP >= ML - range*0.3 for a short entered at ML).

### 0.3 Exit logic: never close at a loss — only panic-close on "sudden" directional move — ✅ DONE v0.48.0 (`docs/sudden-move-exit.md`)
> Implemented the price-run tracker and `sudden_move` gate (≥5 no-rollback ticks + rising NTPS + >0.5%), closing only the adverse side. Band TPs stay profit-only; deep margin/staged SL retained as liquidation protection.

**Problem:** Current s-ts and stair TP logic closes positions at loss/breakeven when triggered by noise. We should only exit when the absolute best price is available, OR when a genuine "sudden" adverse move is detected.

**Definition of "sudden" move (user's specification):**
- Price moving in one adverse direction **without rollbacks** (no counter-move between candles)
- NTPS **increasing** simultaneously (real market participation, not a local spike)
- Move size **> 0.5%** from reference price
- Single spike or isolated small move < 0.5% = NOT sudden — treat as local noise, ignore

**NOT a sudden move:**
- Slow zone-change oscillation across ML (even 15+ times)
- Single NTPS spike followed by return
- Price move < 0.5% in any direction

**Detection approach:**
- Track `sustained_adverse_ticks`: count consecutive sug ticks where price moves adversely without any rollback
- Monitor `ntps_trend`: whether ntps is rising over last 3-5 sug ticks
- Panic-close condition: `sustained_adverse_ticks >= 5 AND ntps_trend > 0 AND adverse_move_pct > 0.5`
- Without these conditions → hold and wait for best exit price regardless of current pnl

**Why:** In sideways markets, small adverse moves are noise and will reverse. Exiting on them locks in losses unnecessarily. Only real directional momentum (sustained move + rising NTPS) warrants panic exit.

## Priority 0 — Entry Timing (High, from 2026-05-29 session) — ✅ DONE v0.48.0 (`docs/sug-tick-entry.md`)

### 0.1 Organic entry fires at candle close, not mid-candle — ✅ DONE v0.48.0
**Problem:** `compute_auto_open` is called inside `handle_indicators` which only runs on 1m candle CLOSES. This means all organic entries fire at the exact candle close price, not at the moment the signal is valid. The signal (V-pattern + zone condition) may be valid mid-candle but the bot only acts on the next close.

**Expected behavior:** Entry should fire on the sug tick (real-time) when conditions align — specifically when `v_bear_fresh`/`v_bull_fresh` + cross zone + pzone are simultaneously true, regardless of candle close timing.

**Fix direction:** Move the relevant entry check (or a subset of it) into `handle_sug_info` so it fires mid-candle on the sug tick when conditions are first met. Use a flag or cooldown to prevent multiple orders per condition-window.

### 0.2 `v_lines` (V-tracker) and `level_stats` are separate trackers — organic entry doesn't see V-tracker data — ✅ DONE v0.48.0
> `v_bull_fresh`/`v_bear_fresh` now also accept a recent `v_lines` crossing (`last_cross_tick`/`last_cross_dir`, 90-tick window) in addition to `level_stats`.

**Problem:** `compute_auto_open` checks `level_stats.v_bull/v_bear` (updated on 1m zone changes). The V-tracker shown in debug (`UIvL3 S2`) uses `v_lines` (updated on sug ticks). These are disconnected — many crossings visible in V-tracker don't flow into `level_stats.v_bear_fresh` which gates organic entries.

**Fix direction:** Either feed v_lines crossing counts into level_stats, or make `v_bull_fresh`/`v_bear_fresh` also check `v_lines[i].short_side.count` / `v_lines[i].long_side.count` with a freshness window. This would make organic entries respond to the fine-grained price-level crossings already being tracked.

---

## Priority 1 — Risk Control (Critical)

### 1.1 BSG/Volume gate for counter-trend entries
**Problem:** Bot adds DCA shorts/longs regardless of market conditions. During low-NTPS directional drift (retail accumulation), it adds into a sustained move with no volume confirmation of exhaustion.

**Rule:**
- Block ALL new counter-trend entries (short DCA above ML, long DCA below ML) unless BSG > 2.0 OR Gain > 2.0 within the last 1–2 candles
- Rationale: BSG > 2.0 + Gain > 2.0 = exhaustion/climax candle = real reversal signal
- Without BSG spike, low-NTPS directional move will continue

**Implementation:** In `compute_limit_orders` / `compute_auto_open`, gate DCA entry placement on `bsg_max_recent > 2.0` or `gain_recent > 2.0`

**Addendum — staircase pattern (2026-05-27):** A single BSG>2.0 spike does NOT confirm exhaustion in a staircase. In a staircase, each step has a small pullback followed by a new higher high. The gate must remain CLOSED for the entire staircase sequence, not just reset after one pullback. Suggested logic: once a BSG>2.0 spike fires, block counter-trend entries until price makes a FAILED new high (tests prior high, BSG>2.0 again, then closes below the test level). A pullback that doesn't test the prior high is NOT a reversal — it's a step consolidation.

---

### 1.2 NTPS directional drift detection
**Problem:** Low NTPS + directional price = sustained user-interest drift (NOT sideways, NOT low conviction). Current code treats NTPS < threshold as "calm" but doesn't check if price is drifting against position.

**Rule:**
- If NTPS < 100 AND price has moved > 0.3% in one direction over last 3 candles → flag as "directional drift"
- During directional drift AGAINST open position: suppress new DCA adds
- Only re-enable when BSG spike (> 2.0) or price reverses

---

### 1.3 NTPS gate for high-activity entries
**Problem:** Bot rebuilt long into falling knife at NTPS=516 (documented 2026-05-26 21:41). The `breakout_guard` param exists but defaults to Off.

**Fix:**
- Enable `breakout_guard = On` by default
- When NTPS > `breakout_ntps` (default 200): suppress new entries for that direction
- Consider separate thresholds: warn at 300, block at 400

---

### 1.4 Staged exit / partial close (missing entirely)
**Problem:** Current SL is all-or-nothing (90% margin loss). No staged exits. A $5–10 adverse move consumes the entire account.

**Needed:**
- Partial position close at -50% unrealized (reduce exposure before full SL)
- Or: reduce DCA size as position goes underwater (scale down, not up)
- Or: max DCA count per direction (currently uncapped until `cap` limit)

---

## Priority 2 — Mode Classification

### 2.1 Suppress TrendLong when MRC cross ≤ -2 + above-avg volume
**Problem:** mkt_mode=TrendLong persisted during downtrend (2026-05-26 21:43). Mode lagged actual market.

**Fix in `classify_mode`:**
- If `mrc_current_cross ≤ -2` AND `volume_gain > 1.0`: cannot classify as TrendLong
- Require cross > 0 for TrendLong, cross < 0 for TrendShort

---

### 2.2 3-candle hysteresis for mode entry
**Problem:** mkt_mode flipped Sideways↔TrendLong 5+ times in 40 minutes (2026-05-27 22:31). Each flip generates order churn (ext-TP orders placed/cancelled).

**Fix:** Require 3 consecutive confirming candles (not 2) before accepting a new mode. Already tracked via `mkt_mode_candles` — raise threshold from 2 to 3.

---

### 2.3 NTPS awareness in mode classification
**Problem:** TrendLong shown during NTPS=628 sell spike (2026-05-26 21:43). High NTPS + opposing cascade should suppress trend modes.

**Fix:** If `ntps > 400` AND cascade direction opposes proposed mode: revert to Sideways.

---

## Priority 3 — Entry Logic

### 3.1 pzone gate for kickstart and grid entries in Sideways mode
**Problem:** Kickstart opened short at $2,116 when price was near MRC lower band (pzone ≈ 0.4). Counter-productive in Sideways mode.

**Fix:**
- In Sideways mode, gate kickstart direction: short only if `pzone > 0.6`, long only if `pzone < 0.4`
- Apply same gate to MRC grid entries (don't add short at lower band, long at upper band)

---

### 3.2 Hedge unwind on TP fills at range extremes
**Problem:** When long TP fills at upper range, short is left at max size exactly when momentum is bullish (2026-05-26 21:18).

**Fix:** When long closes via ext-TP (l-ext-t1/t2), simultaneously reduce short by 30–50%. Inverted for short → long transition.

---

## Priority 4 — Signals

### 4.1 BSG extreme as high-conviction directional signal
**Pattern (2026-05-26 21:37):** BSG < BuySellGainMin (50-candle rolling minimum) + Gain > avg → high-probability directional candle.

**Use:** Temporarily block counter-trend entries for 1–2 candles after BSG extreme.

### 4.2 MRC triple cross + volume = sustained move signal
**Pattern (2026-05-26 22:31):** `mrc_current_cross ≤ -3` + `Gain > 1.0` + `BSG < -0.5` = sustained directional move with volume.

**Use:** Pause counter-trend DCA for 1–2 candles.

---

## Current Config Issues (strategy.json)

| Param | Current | Recommended |
|-------|---------|-------------|
| `breakout_guard` | Off (0) | **On (1)** |
| `breakout_ntps` | 200 | 300 |
| `imbalance_guard` | Off (0) | On (1) for new sessions |
| `margin_leverage` | 100 | Consider 50x to widen SL buffer |
| `sl_margin_pct` | 0.9 | 0.7 (trigger earlier, leave room to re-enter) |

---

## Priority 1 Addendum — Balance Gate (newly observed)

### 1.5 Insufficient balance guard
**Problem:** Bot attempts to open a new position when account has insufficient free margin. Error: "Insufficient balance to open position for %{symbol}". This happens silently — the order attempt fails but the bot continues as if it succeeded, producing ghost state (bot thinks it opened, exchange rejected it).

**Fix:**
- Before any `CreateOrder` call, check available free margin against required margin for the position
- If free margin < required: skip entry entirely and log/alert
- Do NOT retry the failed order — the balance condition won't resolve by itself mid-session
- This is distinct from the SL — the SL fires when unrealized loss hits the threshold; this fires when the account simply can't fund a new trade

---

---

## Session 2026-05-28 — Live Monitoring Analysis & New Findings

### Bugs Fixed in v0.31.2 (coded locally, pending deploy)

**Fix 1 — s-ts spike-exit conflicts with simultaneous DCA (critical)**
The spike-exit for shorts (`s-ts`) placed a BUY limit to partially close the short on the same tick that a DCA SELL was being added. The result: the DCA increased the short position while the spike-exit simultaneously tried to reduce it — double fees, conflicting intent, net zero or negative effect.
Fix: compute `is_averaging_short` before the s-ts block. If any non-reduce-only SELL is already in `place_orders`, skip the s-ts entirely.

**Fix 2 — ts_price clamped the wrong direction (critical)**
`s-ts` is a BUY limit to partially close a short. The limit price should be at or below the MeanLine (buying cheap). The code used `.max(mrc_ml)` which placed the order *above* the MeanLine — guaranteed adverse fill for the short. Fix: `.min(mrc_ml)`.

**Fix 3 — Trend entries open at worst pzone (medium)**
`trend-s` fired after a large dump when pzone ≈ 0.15 (price near lower band) — the worst time to add short. `trend-l` had the same problem at high pzone. Fix: gate `trend-s` on `pzone > 0.35`, gate `trend-l` on `pzone < 0.65`.

---

### Live Observation: NTPS + Low-Activity Directional Moves

**Correct interpretation of low NTPS + directional price:**
When NTPS is *below* the running average and price is moving directionally through key levels (e.g. above MRC UpBig), this is a **liquidity hunt** — engineered price movement to sweep stop losses, not genuine trend momentum. It is NOT "weak buying" or "low conviction trend."

Session example (2026-05-28 ~07:30–07:45 UTC):
- Price pushed above UpBig (1989.56) to 1994.45
- NTPS: 112–124 (avg was ~150 at the time)
- interest_dir=1, interest_cross=4 — one cascade from triggering TrendLong
- Pattern: classic short-stop hunt above UpBig in low-liquidity conditions
- Outcome: price reversed fully as 5m predicted

**Current code has no awareness of this pattern.** A future improvement: if NTPS < 0.75 * avg_ntps AND price just breached an MRC extreme (UpBig / DownBig), suppress new same-direction entries for 2–3 ticks and flag the move as a potential stop hunt.

---

### Multi-TF MRC Analysis Gap

The 1m MRC showed price above UpBig with interest_cross building toward TrendLong. Taken alone, this looks like a bullish breakout. But the 5m MRC showed normal consolidation heading toward MeanLine — the 1m UpBig breach was just the stop-hunt portion of a range-bound 5m candle.

**Improvement needed:** When 5m mkt_mode is Sideways (or explicitly pulling toward MeanLine) and 1m is near an MRC extreme, suppress TrendLong/TrendShort classification on the 1m and reduce new-entry aggressiveness. The 5m context is the higher-authority signal.

Currently the engine only processes 1m. Adding 5m regime awareness is a meaningful filter.

---

### State Machine Observation: interest_dir / interest_cross / interest_tests

Observed timeline during session:
```
t=2286  interest_dir=1  cross=4  tests=1   — one cascade from TrendLong flip
t=2320  interest_dir=1  cross=4  tests=1   — price at 1993.86, stop hunt peak
t=2361  interest_dir=1  cross=4  tests=1   — cascade flipped bearish mv=-1/2
t=2398  interest_dir=1  cross=4  tests=2   — bearish cascade registered as test
t=2543  interest_dir=-1 cross=2  tests=0   — FLIP: bearish interest established, price dropped to 1991.99
```

Key insight: `interest_tests=2` accumulated from the bearish cascades prevented the bullish interest from reaching its flip threshold. Once tests exceeded the tolerance, interest_dir reversed to -1 and the bullish cross counter reset. This is the hysteresis working correctly — the 1m TrendLong classification was correctly prevented by the tests.

**Risk that was present:** At interest_cross=4, interest_dir=1, a single sustained bullish cascade would have flipped mkt_mode to TrendLong and triggered `trend-l` market entries — adding a long while we held an open short at a loss above UpBig. The v0.31.2 pzone guard (`pzone < 0.65` for trend-l) would have blocked that entry since pzone was ~0.80 at the time.

---

### V-Movement Tracker (new concept — 2026-05-28)

**Concept:** Each time price pierces an MRC band (UpBig for shorts, DownBig for longs) and then returns below it, that constitutes one "V". The V has measurable power: how far it penetrated the band, how many sub-bands it crossed, and how long it stayed above. Successive Vs that show decreasing power = exhaustion = the stop hunt is done and the reversal to MeanLine is genuine.

**State to track (per direction, example = short side):**
```rust
struct VTracker {
    active: bool,          // currently above UpBig
    entry_tick: u32,       // tick when price crossed above UpBig
    peak_price: f64,       // highest price reached during this V
    peak_penetration_pct: f64, // (peak - mrc_ub) / mrc_range * 100
    lines_crossed: u8,     // how many sub-bands above UpBig were hit (UpOuter, UpSmall...)
    // History of completed Vs:
    last_v_power: f64,     // peak_penetration_pct of the most recently closed V
    prev_v_power: f64,     // the one before that
    v_count: u8,           // total Vs observed this session above this band
}
```

**State transitions:**
- When `price > mrc_ub` and `v.active == false`: open new V, record `entry_tick`, start tracking `peak_price`
- While `price > mrc_ub`: update `peak_price`, update `lines_crossed` (check UpOuter, UpSmall, UpInner sub-bands)
- When `price <= mrc_ub` and `v.active == true`: close V, shift `last_v_power → prev_v_power`, store new `last_v_power = peak_penetration_pct`, increment `v_count`

**Signals derived:**
1. **V exhaustion** — `v_count >= 2` AND `last_v_power < prev_v_power * 0.70`: the second spike couldn't reach 70% of the first spike's penetration depth → stop hunt power fading → strong reversal signal
2. **V in progress** — `v.active == true`: do NOT DCA the short further, position s-ts at `mrc_ub` (not `price - offset`)
3. **V closed, no exhaustion yet** — anchor s-ts at `mrc_ub` for one more cycle (next V expected)
4. **V exhaustion confirmed** — resume normal TP placement (price going to MeanLine), allow stair TP orders to work

**Lines crossed (sub-bands above UpBig, example for OKX MRC):**
- UpBig → UpOuter: +1 line
- UpOuter → UpSmall: +2 lines
- UpSmall → UpInner: +3 lines (price approaching MeanLine from above — unusual)

**Live example (2026-05-28 session):**
- V1: price peaked at 1994.45, UpBig=1989.56, mrc_range≈24
  - penetration = (1994.45 - 1989.56) / 24 = 20.4%
  - lines_crossed = 1 (reached above UpBig but not UpOuter)
  - BSG/Gain had no exhaustion spike during V1
- V1 closed when price dropped back to 1989.83 (t=2703)
- Watch for V2: if price pushes back above UpBig but peaks below 1994.45 * 0.70 penetration → exhaustion confirmed

**Why this matters for DCA:**
The bot DCA'd the short at t=2631 (sn 7.9→13.9) at p=1993.28, during what turned out to be V1 still in progress. With V-tracking, that DCA would have been blocked: `v.active == true` → suppress new short adds until V closes.

---

### s-ts Spike-Exit Price Anchoring (new finding — 2026-05-28)

**Problem:** The current s-ts price is computed as `(price - offset).min(mrc_ml)` — a target just below current price, capped at MeanLine. This works for the *first* spike above UpBig. But after price retreats from UpBig and then tries again (second test), the s-ts order has already been placed too low and won't catch the second push unless it gets replanned.

**Context:** BSG and Gain are the exhaustion signals. If neither has spiked, the stop hunt is not confirmed finished. Low NTPS + price revisiting UpBig zone = second test likely before the actual reversal to MeanLine.

**Improved logic:**
When `price` has recently been above `mrc_ub` (tracked via a `last_above_ub_tick` or simply `price_high_recent > mrc_ub`) AND `vol_bsg` / `vol_gain` have NOT yet exceeded exhaustion threshold (e.g. `bsg > 2.0` or `gain > 2.0`), anchor the s-ts price at `mrc_ub` instead of `price - offset`. This way, if price retests the UpBig level on the second push, the BUY limit fires at that band.

```rust
// Proposed ts_price logic (non-SMC case):
let ts_price = if price_recently_above_ub && !bsg_exhausted {
    // anchor to UpBig to catch second test
    state.mrc_ub
} else {
    (price - (state.mrc_range * 0.05).max(4.0 * tick)).min(state.mrc_ml)
};
```

Where `price_recently_above_ub` = `state.price_high_n > state.mrc_ub` (highest price in last N ticks), and `bsg_exhausted` = `state.vol_bsg > 2.0 || state.vol_gain > 2.0`.

**Note:** This requires tracking recent price high over a short rolling window (e.g. 5–10 ticks). The UpBig anchor should only activate for short positions, not longs (symmetric logic applies: anchor to DownBig for long spike-exit when price has recently tested below DownBig with no BSG exhaustion).

---

### Kickstart Implementation Issue (pending fix)

Current condition: `(mkt_mode == 2 || mkt_mode == -2) && ticks % 120 == 0`

Problems:
1. Never fires in Sideways mode (mkt_mode=1) — but that's when the bot most needs to restart after a long quiet period
2. `ticks % 120` fires at fixed global intervals, not 120 ticks *after the last close* — could fire immediately after a close or after 119 idle ticks

Better logic: `ticks - last_close_tick > 120` (fire once, 120 ticks after the position was last closed, regardless of mode). This is already tracked as `short_close_tick` / `long_close_tick` in module_state.

---

## Risk Control Summary (Current State = Useless)

**Why current risk control fails:**
1. Single SL at 90% margin = binary outcome, no room for multi-test scenarios
2. No entry gates (BSG, NTPS, directional drift) = adds into any market
3. No partial exits = can't reduce exposure as trade goes against position
4. 100x leverage = $5 adverse move = ~10% margin loss; $50 move = liquidation

**Minimum viable improvements:**
1. Enable `breakout_guard = On` immediately (uses existing code)
2. Add BSG gate (new code needed)
3. Lower `sl_margin_pct` to 0.7 (uses existing param)
4. Add max DCA count per direction (new code)
