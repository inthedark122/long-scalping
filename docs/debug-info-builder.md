# Debug Info Builder (runtime-switchable debug presets)

## Purpose

The strategy exposes its internal state through the `last_debug` string surfaced by
`list_bots`. The platform stores **one** debug line per snapshot, and it refreshes only
~once per minute — landing unpredictably on whichever event fired last (`init` / `ind` /
`sug`). The rich data (V-tracker, sizing, bracket state) lived only on the `sug` line, so
when the snapshot landed on an `ind` line that data was **unreachable** — e.g. the V-tracker
(`vt=`) could not be read at all for an entire session of `ind`-line snapshots.

The Debug Info Builder fixes this generally: a single runtime-switchable config value
(`debug_info`) selects **full-detail topic blocks** that are appended to *every* event line.
Set it to the preset you care about and that view rides whatever snapshot the API serves —
readable straight from `list_bots` with no extra indicator or tool calls.

## Core Concept

- `debug_info` is a **bitmask preset selector** (config value, default `0`):

  | bit | value | preset | block content |
  |-----|-------|--------|---------------|
  | —   | `0`   | **default** | per-event logs only (no extra block) |
  | 0   | `1`   | **V**   | `vt=` V-tracker table (per-MRC-line resistance/support V counts, power%, exhausted) |
  | 1   | `2`   | **MRC** | 1m band price map `db..ub` (low→high) + 5m mean-line `5ml` |
  | 2   | `4`   | **BIG** | Big-line bracket state: `big_tested` flags, `invalid_side`, best-negative exits |
  | 3   | `8`   | **CAS** | movement cascade dir / current run leg / stair-cross ladder / 5m hard-break |
  | 4   | `16`  | **DMP** | uncertainty damp + risk drawdown |
  |     | `31`  | **ALL** | every block |

  Add values to combine (e.g. `3` = V+MRC). When `debug_info != 0`, the selected blocks are
  appended as a ` | sw[...]` suffix.

- **One builder, all lines.** `debug_builder(state, sel)` produces the suffix from `state`
  alone, so it is identical on any tick or event. It is appended to the `init`, `ind`, and
  `sug` lines. (The `sug` line already prints `vt=`, so it is passed `sel & !1` to drop the
  duplicate V block.)

- **Switching is a config push, not a request.** The bot cannot be *asked* for data on
  demand (no request→response channel — future work). Instead `debug_info` is **pushed** via
  the config/Start action (same path as `seed`, `cap`, etc.) and applied on the **next tick**
  (context accumulation). So a focused look is **two switches**: set the preset → read the
  next snapshot → set back to `0`. The `debug_builder` is already written so a one-shot
  "dump preset N now" action can be added trivially once a request channel exists.

## State

No new runtime state. One config field:

```rust
// state.rs — Config
pub debug_info: i64,   // 0 = default; bitmask preset (1 V · 2 MRC · 4 BIG · 8 CAS · 16 DMP · 31 ALL)
```

Applied in `Config::apply` (`"debug_info" => self.debug_info = n as i64`) and echoed in the
config-apply ack (`cfg ... dbg={}`). Exposed in `strategy.json` as the `debug_info` Start
param (default `0`).

## State Transitions

- **Config action with `debug_info=N`** → `Config::apply` sets it; takes effect on the next
  event line emitted (one-tick accumulation delay).
- No automatic changes — it is operator-controlled only.

## Signals Derived

| `debug_info` | what every debug line then carries (suffix) |
|--------------|---------------------------------------------|
| `0` | nothing extra (default per-event logs) |
| `1` | ` | sw[vt=UB^L..S.. US.. UI.. ML.. DI.. DB..]` |
| `2` | ` | sw[mrc=db.. do.. ds.. di.. ml.. ui.. us.. uo.. ub.. | 5ml..]` |
| `4` | ` | sw[big=s0/l0 inv=0 negL../negS..]` |
| `8` | ` | sw[cas=dir./run.t sc./lc. b5=.]` |
| `16` | ` | sw[dmp=.. dd=..]` |
| sum | the selected blocks, space-joined inside one `sw[...]` |

The `vt=` token inside preset `1` is the **same format** the `v-tracker` skill parses, so the
V-tracker report works off any line once `debug_info` includes bit 0.

## History of Changes

### v0.57.0 — 2026-05-30 (built, NOT deployed — awaiting user)
- **Why:** during live monitoring the V-tracker (`vt=`) was unreadable for a whole session
  because every `last_debug` snapshot the MCP served was an `ind` line (no `vt=` there; it
  only rode `sug` lines), and the snapshot refreshes ~1/min so re-fetching never caught a
  `sug` line. User asked for *"an info switch in the debug… pass a number to detect what info
  to display, keep 0 as default"* and a *"debug builder… show default log for any event or a
  preset log"* so any view can be surfaced on demand.
- **Built:** `debug_info` bitmask config (default 0) + `debug_builder(state, sel)`; selected
  full-detail blocks (V / MRC / BIG / CAS / DMP) appended to `init` / `ind` / `sug` lines
  (`sug` gets `sel & !1` to avoid a duplicate `vt=`). Echoed in the cfg ack (`dbg=`); exposed
  as the `debug_info` Start param in `strategy.json`. Compiles clean on wasm32, zero warnings.
- **Scope cut (user):** a one-shot `dbg` *action* that returns the preset immediately was
  drafted then removed — *"we cannot request something from the bot"* (no request→response
  channel yet; user will design one later). The builder is request-ready for that future.
- **Operating model (user note):** switching is a config push applied next tick (context
  accumulation), so a focused read is two switches — preset → read → back to 0.
