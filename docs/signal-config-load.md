# Start-Signal Config Load (`handle_signal`)

## Purpose
For the whole project history, `action_config.action_params` had **no effect** in a backtest —
every param sweep (grid/cooldown/cap/seed/stairs/trend) came back byte-identical to the baseline.
The module only applied config via `ModuleEvent::Action` (`handle_action → cfg.apply`), which the
backtest never emits; the bot-group **Start SIGNAL** that *does* fire in a backtest (via
`action_config` + trigger) was ignored (`_ => ModuleOutput::default()`) **and** the old ABI's
`Signal` event carried no values. This feature wires the Start signal to load its params into the
module so that — once the signal actually carries values — backtest and live-start params take
effect with **no redeploy per variant**.

## Core Concept
The team extended the `lte_strategy_bridge` ABI (`22e8c3cf → b6f8fad2`) so the signal event is now:

```rust
ModuleEvent::Signal { direction: Direction, code: String, values: HashMap<String, serde_json::Value> }
```

`values` is the *same shape* `Config::apply` already consumes (the `handle_action` config path).
The module adds a `Signal` arm that, on the **Start** code, applies those values to `cfg` — **once**.

```rust
ModuleEvent::Signal { code, values, .. } => handle_signal(state, code, values),

fn handle_signal(state, code, values) -> ModuleOutput {
    if code != "start" || state.cfg_loaded { return ModuleOutput::default(); }
    state.cfg.apply(values);   // config-only — never touches runtime state
    state.cfg_loaded = true;
    /* debug ack: "signal start cfg seed=.. cap=.. stairs=.. trend=.. hedge=.." */
}
```

Two design rules, both from the user:
1. **Apply ONCE.** The Start signal re-fires — in a backtest on every closed position
   (`trigger: Infinity`), and live it can be re-issued. Re-applying each time would reset `cfg`
   back to the Start values and **clobber any later live `run_bot_action` config push**. The
   `cfg_loaded` flag makes the load idempotent.
2. **Config-only, never reset runtime state.** `cfg.apply` mutates only the `Config` struct
   (param fields); positions / MFE / run / cooldown state are untouched. (User: *"cfg.apply() for
   action is wrong, because it will reset state"* — the guard above is why the Start path is a
   separate `handle_signal`, not a reuse of `handle_action`, and why it never re-inits.)

Removing the catch-all: with the `Signal` arm added, all 8 `ModuleEvent` variants are matched, so
the old `_ => ModuleOutput::default()` became unreachable and was removed. A future ABI variant now
correctly forces a compile error instead of being silently swallowed.

## State
```rust
// state.rs — State
pub cfg: Config,
pub cfg_loaded: bool,   // NEW: true once the Start signal has loaded its params into cfg
```
`cfg_loaded` initialises to `false` in `State::default()`.

## State Transitions
| Event | Condition | Effect |
|---|---|---|
| `Signal { code:"start", values }` | `!cfg_loaded` | `cfg.apply(values)`; `cfg_loaded = true` |
| `Signal { code:"start", values }` | `cfg_loaded` already true | no-op (idempotent — protects live config pushes) |
| `Signal { code != "start" }` | — | no-op (only the Start code loads config) |
| `Action { name, values }` (live `run_bot_action`) | — | unchanged — `handle_action` still pushes config live |

## Signals Derived
- **Effective `cfg`** for the whole run = the Start signal's `values` (falling back to
  `Config::default()` for any key not present in `values`, e.g. `risk_pct`, `stop_pct`,
  `enable_hedge`, `hedge_ntps_mult`, `debug_info`, which are not Start params).
- **Debug ack** on the loading tick: `signal start cfg seed=.. cap=.. cd=.. near=.. grid=.. stairs=.. trend=.. hedge=.. hntps=.. dbg=..` (not surfaced in backtest order notes; visible live in `last_debug`).

## History of Changes

### v0.61-signal — 2026-05-31 (built; module side ready — backtest producer gap, NOT deployed)
- **Added** the `ModuleEvent::Signal` arm + `handle_signal` (apply-once via `State.cfg_loaded`,
  keyed on `code == "start"`, config-only). Removed the now-dead `_` catch-all. Pinned the ABI
  rev (`b6f8fad2`) in `Cargo.toml`. Compiles clean on wasm32, zero warnings. Built `0.61-signal`
  (server Success vs pinned ABI). Branch `v0.61-signal` off clean v0.61.
- **Proof attempt (failed at the producer, not the module):** ran A (defaults, v84/run102) vs
  B (`enable_stairs=0, enable_trend=0`, v85/run103) on the same 28d window — **byte-identical**
  (profit 3.39761, fee 1.68954, vol 6867.71). Run B's *recorded* `action_config` held
  `enable_stairs:0/enable_trend:0`, yet its orders still contain `dca-l`, `l-ext-t1/t2`,
  `l-bk1/bk2` → stairs+trend executed → the module ran `Config::default()`.
- **Diagnosis:** the ABI now carries `values` and the module handles them, but the **backtest
  producer does not populate `Signal.values` from `action_config.action_params`** — it stores the
  params on the run but does not forward them on the constructed Signal event. The module side is
  complete and correct; the remaining work is the backtest engine / bridge that builds the Signal.
- **Open:** once the producer forwards values, re-run the A-vs-B proof (B should collapse to
  hedge-only). The live Start path may already populate values — validate there if needed.

### v0.61-cfgdebug / cfgdebug2 — 2026-05-31 (throwaway live diagnostic → PRODUCER FIXED, signal works)
- **`0.61-cfgdebug`** (branch `v0.61-cfgdebug` off `v0.61-signal`, `const CFG_DEBUG=true`): replaces
  every event's debug with the loaded-config line so `last_debug` always shows it. Also removed
  `cfg.apply` from `handle_action` (user: config loads only via the Start signal now; an Action
  re-applying config would reset/clobber the running config).
- **First live test (producer still broken):** deployed live, stopped bot 694, started via
  `run_bot_group_action(29,'start', values={seed:0.30,…})` → after 167 ticks `CFG loaded=false …`
  + all defaults. So `handle_signal` never fired — the host wasn't delivering the Signal. Matched
  the backtest A==B result: the **producer (host)** wasn't emitting `ModuleEvent::Signal` to the module.
- **`0.61-cfgdebug2`** (commit `0d92818`): added `State.sig_count/sig_code/sig_vals`; `handle_signal`
  now records **every** signal (count + code + sorted `k=v`) *before* gating, then applies cfg only
  on `code=="start"` & `!cfg_loaded`. Debug shows `sig#=N code='…' vals=[…]`.
- **Producer fixed (user "bot updated"):** stop/start with `values={cap:0.95,cooldown_ticks:9,
  enable_stairs:0,nearest:4}` → `CFG loaded=true … sig#=1 code='start'
  vals=[cap=0.95,cooldown_ticks=9,enable_stairs=0,nearest=4] … cap=0.95 cd=9 near=4 stairs=0`.
  **The Signal arrives with the exact values and `cfg.apply` lands them** (un-passed keys stay
  default = correct partial-apply). End-to-end config-via-Start-signal is now **live-functional**.
- **Next:** confirm the backtest engine got the same fix (re-run A-vs-B); then retire the cfgdebug
  builds and run the clean `0.61-signal` (no CFG_DEBUG override) as the live baseline.
