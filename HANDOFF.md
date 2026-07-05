# long-scalping — Project Handoff

> Last updated 2026-06-06. Read this first. It captures the full state, the architecture, the
> build/deploy/backtest workflow, and (most importantly) the **non-obvious gotchas** that took a long
> time to learn. Detailed per-feature docs live in `docs/`; the chronological log is `analysis_notes.md`.

---

## 1. What this is
A **WASM strategy module** (Rust → `wasm32-wasip1`) for the **LivTorgEx (LTE)** trading platform. The
compiled `.wasm` runs server-side inside both the **backtest engine** and the **live trading worker**,
which feed it market events over the **`lte_strategy_bridge` ABI** and execute the orders it returns.

| | |
|---|---|
| Source | `module/` (`src/engine.rs` ≈ 2.8k lines, `src/state.rs`, `src/main.rs`) |
| Module id | `20dd415e-a179-4cc1-a7eb-a42cbcfd9f1e` |
| ABI dep | `lte_strategy_bridge` **pinned** in `module/Cargo.toml` at rev `b6f8fad2` (the rev that carries `values` on the Start signal) |
| Test/live bot group | **29** — `OKX#ETH-USDT-SWAP#SWAP` (margin 0.5, the designated test group) |
| Live bot | id **694** (currently **STOPPED**) |
| All MCP ops | use the **`mcp__claude_ai_LTE__*`** tools (never the `_App` variant) |

## 2. Current state (2026-06-06)
- **Deployed on group 29: `v0.61-long`** — the **long-only** build (all short + hedge code removed).
- Live bot 694 is **stopped**; nothing is trading.
- Group 29 `api_key_id` is **null** (see gotcha #9) and its symbols are **`ETH` + `HOME`** (HOME added for backtesting), `margin_leverage` 100.
- **Don't assume bot-group / account setup.** Group 29 is *this* strategy's group, but a fresh
  tool/developer should **ask the user** which bot-group(s) and account (api_key) to use rather than
  hardcode it — groups/accounts change (a separate group 31 existed earlier and was removed; api_key 5
  was deleted, see gotcha #9). Treat the current group/symbol/leverage as *state to confirm*, not gospel.

### Branch map (the live code is NOT on `main`)
| branch | what it is |
|---|---|
| **`v0.61-long`** | **current / deployed** — long-only (short + hedge stripped). |
| `v0.61-hedge` | the previous **both-sided + hedge** version, preserved intact. |
| `v0.61-signal` | the milestone that wired config via the Start signal (`handle_signal`) + removed `cfg.apply` from `handle_action`. |
| `v0.61-cfgdebug` | throwaway debug builds (`CFG_DEBUG` override, `sig#` tracking) used to prove the signal path. |
| `v0.61-scalp`, `v0.61-sudden-fix`, `v0.62-asset01-conviction` | older rejected experiments (kept for the record). |
| `main` | **behind** — the pre-signal baseline. Merge `v0.61-long` → `main` if you want main to reflect production. |
All branches are pushed to `origin` (`git@github.com:inthedark122/eth-adaptive-strategy.git`).

## 3. Architecture

### Event model → `run()` (engine.rs)
The bridge sends `ModuleEvent`s; `run()` dispatches them. All 8 variants are matched (no catch-all):
- `Init` — once at start, full indicator history → `handle_init`.
- `Indicators { timeframes }` — on each candle close → `handle_indicators` (also fires market opens).
- `SugInfo` — every "suggestion" tick (~sub-second) → `handle_sug_info` — **the main loop**: closes,
  staged SL, then `compute_limit_orders`; also fires mid-candle opens via `compute_auto_open`.
- `Signal { code, values }` — the **bot-group Start command**. `handle_signal` applies `values` to
  `cfg` **once** (guarded by `state.cfg_loaded`, only on `code == "start"`). This is how config arrives.
- `NewPosition` / `FinishPosition` — position lifecycle (reset per-side state on finish).
- `OrderUpdate` — fill tracking (`state.filled`, fill indices).
- `Action { name, values }` — individual bot actions (`force_open_long`/`force_close`/`reset_fc`).
  **Does NOT apply config** anymore (removed deliberately — see gotcha #2).

### Key functions
- `compute_auto_open` (engine.rs ~786) — **market opens**. Long-only emits: `auto-l`, `trend-l`,
  `re-l`, `stair-l`. Sized by `budget_size`/`risk_sized_notional`.
- `compute_limit_orders` (~1036) — **resting limits + TPs**: band-DCA `l-e1/l-e2`, cascade `mv-le*`,
  trend-DCA `dca-l`, trend TPs `t-l*`, Big bracket `l-bk*`, band TPs `l-t1/l-t2`, extended `l-ext-t*`,
  staged SL `psl-l`. (The band-ladder is gated on `cfg.enable_grid`.)
- `handle_sug_info` — orchestration + **closes**: `MARGIN SL`, `trend-sl`, `sudden-down` (panic), `tc-cl*` (transition close).
- Regime/analysis: `update_movement`/`advance_market_mode`/`classify_mode` (the **movement cascade**
  → `mkt_mode`: `1` Sideways, `2` TrendUp, `-2` TrendDown), `update_v_lines` (V-tracker), `update_fast_candles`.
- Sizing: `risk_sized_notional` = `min((max_amount/leverage × risk_pct)/stop_pct, max_amount)`;
  `margin = max_amount/leverage`. **Sizing is `max_amount`-driven** (see gotcha #4).

### Indicators read (`read_*` helpers)
`mrc_200` (MRC bands `DownBig..UpBig` + cross), `smc`, `candle` (OHLC), and the **SuggestionInfo**
analysis (`ntps`, fast-candle). Many more are
*available* (see `list_indicators`: `ZigZagTrend`, `Supertrend`, `Smc.DirectionSwing`, `BollingerBands`,
`Imbalance`, `Window`, …) — the regime redesign (`docs/regime-modes.md`) will pull `ZigZagTrend` + `Candle`.

## 4. Build / deploy / backtest workflow

**Compile-check (never `cargo build` — it overloads the box):**
```
cd module && cargo check --target wasm32-wasip1   # MUST be zero warnings
```
**Build the wasm server-side:**
```
tar czf ../tmp/<v>.tar.gz Cargo.toml Cargo.lock src         # from module/
get_module_upload_url(module_id, "<v>")  →  curl -X PUT -T ../tmp/<v>.tar.gz "$url"   # URL valid 5 min, expect HTTP 200
submit_module_build(module_id, "<v>")  →  poll get_module_build_status(build_id) until Success (~10s)
```
**Deploy** (patch the group — **both** fields):
```
update_bot_group(29, settings_patches=[
  {path:"strategy.module_version", value:"<v>"},
  {path:"strategy.professional.dynamic_module.module_version", value:"<v>"}])
```
**Backtest:**
```
run_bot_group_backtest(29, start_ms, end_ms, name,
  action_config={signal_action_code:"start", margin:10, leverage:100,
                 trigger:{type:"Infinity"},                 # REQUIRED — without it: 0 trades
                 action_params:{...13 Start defaults...}})
# poll list_bot_group_backtests(29) → get_backtest_run(run_id) (paginated; large → saved to a file)
```
The 13 Start params (group `signal_actions[].default`): `breakout_guard 0, imbalance_guard 0,
margin_sl 1, imbalance_ratio 3, breakout_ntps 300, sl_margin_pct 0.7, seed 0.05, cap 0.9,
cooldown_ticks 5, nearest 2, enable_grid 0, enable_stairs 1, enable_trend 1`.

## 5. KNOWN ISSUES & GOTCHAS — read before trusting any result

1. **🔴 BACKTEST PnL IS BROKEN (`issues/001`).** The sim **under-reports losses** on multi-cycle
   (re-entry) positions — the per-position `total_profit` ends up reflecting only the **last** re-entry
   cycle, not the sum. Proven on position 31978: reported **−$0.04**, real (fill-reconstruction) **−$1.31**.
   → **Every backtest net-PnL number is untrustworthy** (headline, the grid's "+$3.73", the long-only
   results, HOME's "+$13"). For a *real* number, use `issues/001-…/reconstruct_pnl.py` (average-cost
   over the actual fills). **The team must fix the sim before any backtest can be believed.** `issues/`
   is gitignored (local; share out-of-band).
2. **Params only land via the Start SIGNAL.** `action_config.action_params` work **only** because the
   host now delivers `ModuleEvent::Signal{values}` (ABI `b6f8fad2` + a host-side producer fix). `handle_signal`
   applies them once. Params **not** in the 13-param Start list (`risk_pct`, `stop_pct`, `debug_info`,
   etc.) stay at `Config::default()` → changing them needs a **code change + redeploy**. `handle_action`
   no longer applies config (would reset/clobber the running cfg).
3. **Leverage override is IGNORED.** `action_config.leverage` and the per-symbol override leverage do
   nothing — the backtest uses the **group's top-level `margin_leverage`**. To backtest at a different
   leverage, set `update_bot_group(29, margin_leverage=N)`, run, then revert.
4. **`margin` override doesn't change sizing.** Sizing is `max_amount`-driven; margin 0.5 vs 10 give
   identical results.
5. **Fee model has NO maker/taker distinction** (~0.025–0.032%/side flat). A maker-scalp strategy can't
   be shown profitable in backtest even if it would be live (killed the `v0.61-scalp` experiment).
6. **Backtest `symbols` override only accepts symbols already in the group.** To backtest a new token,
   `update_bot_group(29, symbols=[…, {symbol_key, leverage}])` first (HOME = `OKX#HOME-USDT-SWAP#SWAP`).
7. **Concurrency cap = 3.** Launching >3 backtests queues/slows them.
8. **`pnl_chart` is garbage** (per-hour fee exceeds total). Use positions self-calc — but note that's
   now also subject to gotcha #1.
9. **`api_key`** on group 29 is **null** because key **id 5 (`inthedark4`) was deleted**. The valid OKX
   key is now **id 8 (`inthedark1`)**. `update_bot_group(29, api_key_id=5)` errors ("key not found").
   **Set `api_key_id=8` (or re-add inthedark4) before any live start.** Backtests don't need it.
10. **Restarting a bot** = `run_bot_action(694, "lte:stop")`; **starting with config** =
    `run_bot_group_action(29, "start", symbol_key, price, values={…})` (the live Start signal — it
    carries the params to the module). Each start is a fresh module instance (`cfg_loaded` resets).

## 6. Strategy summary & key findings
- **`v0.61` (hedge version)** was the long-time baseline (back-tested +$1.60/28d) — *but that number is
  now suspect under gotcha #1*. It traded both directions with a persist-hedge for protection.
- **Long-only (`v0.61-long`)** was adopted to stop shorts bleeding into pumps (e.g. a short that fought a
  +5% move). Trade-off: it **sits out the oversold tape** and trades little when buyers aren't in control.
- **Backtest snapshots (all PnL-bug-caveated):** ETH 30d long-only ≈ net −$0.92 (low volume);
  HOME 30d long-only ≈ +$13 (100x) / +$15.6 (20x) — HOME trended up, so the bug bites it less.
  `enable_grid=1` looked like +$3.73/3× volume but that was the bug hiding stop-out losses → likely net-negative.
- `mkt_mode` (cascade) is the regime. MRC bands are the price ladder.

## 7. Roadmap
1. **Next feature:** the 3-mode **regime redesign** — `docs/regime-modes.md` (Downtrend / Sideway /
   Uptrend, 1h ZigZag confirm, `enable_grid` folded into Sideway). Build on a
   `v0.61-long-regime` branch.
2. **Blocking everything quantitative:** fix the sim PnL bug (#1), then re-validate long-only, the grid,
   and the regime modes with trustworthy numbers.
3. Before live: set group 29 `api_key_id=8`, and decide ETH-only vs ETH+HOME symbols.

## 8. Where everything is
- `module/` — the Rust source. `module/CHANGELOG.md` — version-by-version history + rationale.
- `docs/` — one file per feature (hedge-protection, sudden-move-exit, trend-dca-bounce-placement,
  v-movement, window-stairs, signal-config-load, scalp-volume-experiment, **regime-modes**, …).
- `implementation.md` — ranked bugs/improvements with design notes.
- `analysis_notes.md` — chronological session log (note: split across `v0.61-signal`/`v0.61-cfgdebug`
  branches; `v0.61-long` has the latest committed copy).
- `issues/` — bug reports for the dev team (gitignored; reproducible — e.g. `issues/001-…` has a
  standalone `reconstruct_pnl.py` + fixture). Share out-of-band.
- `strategy.json` — portable bot-group template. `README.md` — human-facing overview.
- `skills/strategy-improvement-testing/skill.md` — the branch→build→backtest→document→tag standard.
