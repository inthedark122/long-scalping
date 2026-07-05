---
name: lte-strategy-monitoring
description: Live monitoring loop for LTE bot groups (default ETH group 29). Each check fetches bots/positions/orders/balances + the Projection WS feed (scripts/sug_ws.py: trandm, asset_01, order-book, OI) + MRC lines, gives a FULL report with a one-liner, flags escalations, and self-paces via ScheduleWakeup. RECOMMENDS trades — never executes a close/open/deploy without an explicit user keyword. Appends only notable events to analysis_notes.md.
---

# LTE Strategy Monitoring

A self-rescheduling watch loop over a deployed bot group (default **group 29 = ETH-USDT-SWAP on OKX, bot 694**). Runs via `ScheduleWakeup`, passing a self-contained prompt back to itself each tick.

## Golden rule — RECOMMEND, don't execute (non-negotiable)

**Never open, close, bank, add, hedge, or deploy on your own market read.** Flag the situation, give a clear recommendation, and **wait for an explicit user keyword.** Banking a winning short on a 5m-ML reclaim *without* the user's "bank" was a real overstep (2026-05-30) — the call was defensible but it wasn't mine to make. Reserve autonomous action for: (a) an explicit keyword this turn, or (b) a pre-authorized hard stop (balance < floor) — and even then, prefer to surface it.

User keywords → action (only act on these, this turn):
- **bank / close / close short** → `run_bot_action(694, force_close_short)` (or `force_close_long`) + report realized + balance.
- **short / force short [N] / add / stair / risk N** → `run_bot_action(694, force_open_short, values:{amount: notional})` (risk-sized; see Sizing).
- **flat / close all** → force_close both sides + stop bot + conclude.
- **hold / resume** → keep monitoring.
- **stop** → `run_bot_action(694, lte:stop)`.
- **slow / stretch** → lengthen the interval (e.g. 240s).
- **deploy / deploy a** → run the module deploy flow (see Deploy below), then restart.
- **wait** → keep current version live, keep monitoring.

## Data sources — fetch in parallel each check

1. `mcp__claude_ai_LTE__list_bots` (group 29, Active) — `last_debug`, `mkt_mode`, `module_state`, `realised_pnl`.
2. `mcp__claude_ai_LTE__list_bot_positions` (bot_id 694, `["InProgress"]`) — side, qty (**signed**), entry_price, fee.
3. `mcp__claude_ai_LTE__list_bot_orders` (group 29, limit 5) — new entries/TPs/closes + their `note`. **ANY long opened while trandm ≤ −50 = a v0.53+ BUG → flag hard.**
4. `mcp__claude_ai_LTE__get_account_balances` — inthedark4 (id 5) balance. **See balance caveat.**
5. **Projection WS feed** — `python3 /home/dev/strategies/long-scalping/scripts/sug_ws.py` → price, **trandm**, **asset_01**, NTPS, dir, price_1h/4h/24h, klines (1m/1h/4h), **order-book walls** (resist/support + buy/sell amounts), **OI** + change, **net_flow** (qtym_asset). (`--stream` to follow, `--raw` for full JSON.)
6. MRC lines when relevant — `curl -s "https://okx.tg.trading.sinvoid.me/api/indicators?symbol_key=OKX%23ETH-USDT-SWAP%23SWAP&timeframe=60&limit=1"` (and `timeframe=300` for 5m): `mrc_200` ML / DownInner / DownBig / CurrentCross, `candle` Close/High, `zigzagtrend` EndDirection/Fibo.

## Key gauges & concepts

- **trandm** (Projection `indicator.trandm`, NOT in /api/indicators): which side is in control, −100 (selling) … +100 (buying). **Ride the side — never fade/buy-the-dip.** `≤ −50` strong sell, `≤ −25` lean. The engine blocks counter-trend opens while strong (v0.53) and rides via `tm-s`/`tm-l` (v0.54+). See `docs/trandm-asset01.md`.
- **asset_01** ($ to move trandm 1%; **USD = raw × 100**): depth/power. Raw <~50k ($5M) = thin/coiled (sudden move loaded); ~200k ($20M) = spent.
- **MRC mean line — 3 states** (`docs/trend-target-5m-ml.md`): **soft touch** → the ML *angle* (`state.angle`, 1m ML slope) projects the drift (down-angle + price under = bearish, short-bias / re-short at the ML); **hard break + hold below** (candle highs under the ML) → continuation, no bounce — *don't fade it*; **big breakout out of the 1m MRC Big** → FLY, ride it, **no line TP**.
- **5m structure**: 5m **ML = room/target** (price far from it = big runway, "no block"); 5m **down/up-lines = stair-add levels**; **1m ML = re-entry** on a soft touch. ZigZag `EndDirection` flip + high `Fibo` (~0.9) = counter-move exhausted.
- **NTPS** = avg trades / 3s (activity). Spike (>2× avg) = momentum/fly; fade = exhaustion. >400 warn, >600 danger.
- **sug debug fields** (in `last_debug`): `tm=`(trandm) `a01=`(asset_01) `tms=`(tm_side regime) `ss=`(short_stair zone) `b5=`(5m hard-break) `run=`(price-run) `inv=`(invalid_side) `negL/negS=`(best-neg exits).

## PnL, balance, floor

- **uPnL = (mark − entry_price) × qty** (qty is **signed**; negative for shorts). `position.pnl` is **REALIZED**, not unrealized — don't use it for live PnL.
- **Balance caveat:** `get_account_balances` `balance` = **AVAILABLE = total equity − frozen margin** (open position + pending limit orders), NOT equity. It drops while orders are open and jumps when flat. **Judge the floor on total / when-flat**, don't false-alarm.
- **Floor (test group 29):** WARN < $0.55, **STOP < $0.50** (force-close all, conclude). Restarts cost ~$0.50.

## Sizing (only when the user asks to add/short)

Risk-based, capped at the bot's `max_amount` (NOT equity): `notional = min( (max_amount/leverage × risk_pct) / stop_pct , max_amount )`. Group 29: max_amount 50, lev 100 → $0.50 margin / $50 / ~0.0246 ETH cap; defaults risk_pct 0.40, stop_pct 0.007 → ~0.014 ETH. `force_open_short` `amount` is **notional USDT** (qty ≈ amount/price).

## Report format

Lead with a **one-liner**: `price | mkt_mode | position(side+qty+uPnL vs entry) | balance | trandm | dir | NTPS`. Then **expand only the flags**. Keep no-change ticks to the one-liner + one sentence. Always report when a position is open or an escalation fires.

## Escalation triggers (flag + recommend)

- Position opened/closed/changed (size, partial TP fill) → mechanism + note + realized/uPnL + balance.
- **ANY long opened while trandm ≤ −50** → v0.53+ regression BUG, flag hard.
- 5m ML hard-break **repair** (candle high reclaims the broken ML) → reversion risk → recommend bank.
- Price reaches a 5m down/up-line (stair/target zone) → continuation vs reject.
- 1m-Big breakout / fly (out of the 1m MRC Big + momentum) → ride, don't cap.
- trandm crosses toward 0 / zigzag EndDirection flips → thesis ending.
- Balance(total) < 0.55 WARN / < 0.50 STOP. NTPS 400 warn / 600 danger. BSG > |2|.
- A single net_flow flush that **doesn't break price structure** = chop; require a *price* break to confirm (don't over-react — the 17:21 fakeout lesson).

## Deploy flow (only on user "deploy")

`cargo check --target wasm32-wasip1` (zero warnings) → `tar czf /tmp/module-X.Y.Z.tar.gz Cargo.toml src/` (from `module/`) → `get_module_upload_url(module_id, version)` → `curl -X PUT -T <tar> "$url"` → `submit_module_build` → poll `get_module_build_status` to Success → `update_bot_group` patching **BOTH** `strategy.module_version` AND `strategy.professional.dynamic_module.module_version` + rename → `run_bot_action(694, lte:stop)` → confirm Stopped → `run_bot_group_action(29, start, symbol_key=OKX#ETH-USDT-SWAP#SWAP, price=<live>)`. Module id `20dd415e-a179-4cc1-a7eb-a42cbcfd9f1e`. Deploying closes any open position (stop→start) — flatten/confirm first. (See memory `project_module_deploy_flow`.)

## Analysis notes

Append **only notable** events (entry/close/TP, bug, new market behavior, design insight) to `/home/dev/dev/strategy/long-scalping/analysis_notes.md` as `[HH:MM] <what + why>`. **Skip no-change ticks.**

## Cadence (dynamic, via ScheduleWakeup)

Default **90s**. Tighten to **60–75s** at a live decision point (target/reclaim/break). Lengthen to **~240s** during dead chop or on "slow". Mind the prompt cache: <270s stays warm; jumping to 300s+ pays a cache miss, so either stay ≤270s or commit to a longer idle. Pass the **same self-contained prompt** back each tick (current STATE + the parallel-fetch list + escalation rules + keyword→action map + "reschedule every Ns"). Carry the latest STATE forward so each wake-up has context.

## Starting / stopping

- **Start** (`/lte-strategy-monitoring`): default group 29 / bot 694; run the first full check immediately; then `ScheduleWakeup` with the self-contained prompt; tell the user it's running.
- **Stop** ("stop monitoring"): simply **do not reschedule** — omitting the `ScheduleWakeup` call ends the loop. Confirm it's stopped.
