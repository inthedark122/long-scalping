---
name: strategy-improvement-testing
description: The single standard for proposing, building, backtesting, and validating an improvement to an LTE WASM strategy module BEFORE it goes live. One branch -> one backtest name -> one version tag. Every change is proven by a self-calc backtest (Sigma profit - Sigma fee from positions) against the current live baseline, on the same window/margin/params, with the live bot never exposed to an untested version. Trigger when the user wants to "create a new version", "test an improvement", "backtest a change", or asks for the testing standard.
---

# Strategy Improvement Testing — Standard Workflow

The repeatable standard for taking an idea to a validated module version. Default target: **group 29 / bot 694, OKX ETH-USDT-SWAP**, module `20dd415e-a179-4cc1-a7eb-a42cbcfd9f1e`.

## Golden rules (non-negotiable)

1. **RECOMMEND / validate only — never deploy live without an explicit user keyword.** Build + backtest freely; flipping the live bot needs the user's word.
2. **Never expose the live bot to an untested version.** To backtest a non-live version you patch the group's `module_version`, run, then **REVERT**. The live in-memory bot keeps running the old version until an explicit restart — a config change alone does not touch it. Always revert.
3. **Self-calc the net from positions.** Headline `profit`/`fee`/`volume` and the hourly `pnl_chart` are **UNRELIABLE** (can be byte-identical across genuinely different runs). NET = `Σ total_profit − Σ fee`.
4. **Only use supported signals.** The strategy replays MRC crossings, MRC pattern classifications, and volume fields. Do not add NTPS, fast-candle, regime, cascade, V-tracker, OI, or order-book dependencies to the strategy.
5. **One branch -> one backtest name -> one version tag**, all the same slug, so git history and LTE backtest history read as one timeline.

## 0. Baseline (the number to beat)

Keep the current proven baseline and always compare against it on the **same window + margin + params**. As of 2026-05-30: **v0.61 = Run 22 = NET +$1.60** (m10). Window: **Apr30 00:00 = `1777507200000` ms → May28 00:00 = `1779926400000` ms**.

## 1. Branch

`git checkout -b vX.YY-<slug>` from `main`. One improvement = one branch. The slug IS the version name (reused for the backtest name and the git tag).

## 2. Implement

Edit `module/src/`. Tag intent in code comments (`// vX.YY: ...`). Keep the change isolated and reversible. Prefer the smallest change that tests the hypothesis.

## 3. Compile-check — ZERO warnings

`cd module && cargo check --target wasm32-wasip1`. Zero warnings is mandatory (`cargo fix` unused imports, `cargo fmt`). A warning = not done. **Never `cargo build`** (linking overloads the machine) — `check` only.

## 4. Commit on the branch

`git commit` with `feat(vX.YY): <what + why>` and the `Co-Authored-By` footer; note "backtest pending."

## 5. Server build (source -> WASM)

From `module/`:
```
tar czf /tmp/module-X.YY.0.tar.gz Cargo.toml src
```
then:
- `get_module_upload_url(module_id, module_version="X.YY.0")`
- `curl -sS -X PUT -T /tmp/module-X.YY.0.tar.gz "$upload_url" -w "HTTP %{http_code}\n"` (expect **HTTP 200**; URL valid 5 min)
- `submit_module_build(module_id, "X.YY.0")` -> `build_id`
- poll `get_module_build_status(build_id)` until **Success** (on **Failed**, `build_output` has the compiler errors). Server compiles in ~10s.

## 6. Backtest (non-destructive to live)

**a. Patch the group's version for the run** (config only):
```
update_bot_group(29, settings_patches=[
  {path:"strategy.module_version", value:"X.YY.0"},
  {path:"strategy.professional.dynamic_module.module_version", value:"X.YY.0"}
])
```
BOTH fields. The live bot is unaffected (in-memory).

**b. Run with `action_config`** (carries margin/leverage/trigger on the RUN — no live mutation):
```
run_bot_group_backtest(29,
  start_time=1777507200000, end_time=1779926400000,
  name="vX.YY-<slug>",
  action_config={
    signal_action_code:"start", margin:10, leverage:100,
    trigger:{type:"Infinity"},
    action_params:{cap_init:2, trading_amount:50, min_tp_percent:0,
      min_sl_percent:0, risk_target_percent:0, required_samples:3,
      entry_type:1}
  })
```
⚠️ **Without the trigger Infinity (action_config) the start signal never fires -> 0 trades / 0 volume / ~50s finish.** Looks like a broken module but isn't — suspect a missing trigger first. Margin 0.5 vs 10 does NOT change behavior (sizing is `max_amount`-driven) — use **m10** for readable $ and to match the baseline.

**c. Poll** `list_bot_group_backtests(29)` until the run `status` = **FINISHED** (progress maxes ~98%, not 100%).

**d. REVERT immediately** — `update_bot_group(29, ...)` setting both `module_version` fields back to the LIVE version. Never leave the live group on an untested version.

## 7. Self-calc the net (MANDATORY)

Page `get_backtest_run(run_id, position_limit=50, position_offset=0,50,100,...)`, dedup positions by `id`, then **NET = Σ `total_profit` − Σ `fee`**. Save the raw tool results to a file and parse with python for large runs. Also **bucket by the open-order `note`** (`auto-s`/`re-s`/`hedge-l`/`stair-s`/`neg-*`/...) for attribution: which entry types win/lose, trade count, total fees. (Sanity: a clean run's headline `profit − fee` often equals the self-calc — but always self-calc.)

## 8. Compare + decide

Report: **NET vs baseline (+$1.60)**, by-type breakdown, trade count, total fee, biggest winners/losers. Verdict:
- Beats / safely matches baseline -> **candidate for deploy** (await user keyword).
- Worse -> **discard the branch** (or iterate). The by-type breakdown usually shows why — document it.

## 9. Document (after every test, win or lose)

- `module/CHANGELOG.md`: new `vX.YY.0` entry — what changed, build status, **backtest NET**.
- `docs/<feature>.md` -> **History of Changes**: version, date, what changed, the live observation that motivated it, the backtest NET. Never delete old history.
- `implementation.md`: the ranked entry.

## 10. Tag / merge (winning + user-approved version only)

`git tag vX.YY.0` on the validated commit (backtest name == branch == tag). Merge the branch to `main`. Discard losing branches but **keep their CHANGELOG note** (why it lost — saves re-testing dead ends).

## 11. Deploy (ONLY on explicit user keyword)

Patch both `module_version` fields + rename the group -> `run_bot_action(694, lte:stop)` (this **flattens** open positions) -> confirm Stopped -> `run_bot_group_action(29, start, symbol_key="OKX#ETH-USDT-SWAP#SWAP", price=<live>, values={...action params...})`. See the `lte-strategy-monitoring` skill and memory `project_module_deploy_flow`.

## Constants

| | |
|---|---|
| module_id | `20dd415e-a179-4cc1-a7eb-a42cbcfd9f1e` |
| group / bot | 29 / 694, `OKX#ETH-USDT-SWAP#SWAP`, lev 100 |
| backtest window | `1777507200000` -> `1779926400000` (Apr30->May28 2026) |
| baseline | v0.61 Run 22 NET **+$1.60** (m10) |
| supported | MRC crossings, MRC patterns, volume |
| NOT backtestable | OI, order_book |
| target | `wasm32-wasip1` |
