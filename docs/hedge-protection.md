# Hedge-as-Insurance (pullback-placed, pump-gated protective leg)

## Purpose

The original hedge (v0.49–v0.56) opened an **opposite-direction position at market** the
instant `mkt_mode` flipped against the side we held, and it **invalidated** the side we were
on (best-negative exit only) while making the hedge the new primary. Live (2026-05-30) this
misfired repeatedly: `mkt_mode` flipped to TrendLong on a *quiet* drift (NTPS ~20) to the top
of a range, so the bot **market-bought the local top** while `trandm` was −68 (sellers in
control) — a wrong-way long at the worst price — then `mkt_mode` reverted after ~3 candles,
leaving an orphaned losing long *and* a short that had been demoted to "invalid."

The user's reframing fixed the design intent: the hedge is **insurance**, not a directional
flip. It should protect the trandm-aligned primary against a **real** sudden pump/dump, open
at the **best price** (not chase the extreme), and the primary must **stay primary**. The
"90%-sure it's pumping" tell is **high NTPS** — the real breakout ran NTPS ~59 vs the quiet
grind's ~20.

## Core Concept

Three rules (all from the user, 2026-05-30):

1. **Pump-gated trigger.** Arm the hedge only on a *genuine* sudden move: the trend label
   agrees (`mkt_mode == 2` for a long hedge / `== -2` for a short), a fast-candle points that
   way (`fc.dir`), **and NTPS has surged** to `≥ hedge_ntps_mult ×` its own baseline
   (`ntps_mult`, default 1.6 — a real pump, not a drift). A quiet `mkt_mode` blip never arms.

2. **Pullback-placed entry (best price).** Never market-buy the top. Rest the hedge as a
   limit a **FastCandle-distance back from the running extreme** — a long hedge buys a dip
   below the running high (`cur_hi − fc_dist`), a short hedge sells a bounce above the running
   low (`cur_lo + fc_dist`). It's re-emitted each tick so it trails the extreme, and it is
   auto-cancelled the moment the pump fades (the arm clears). Same bounce-placement idea as
   `dca` (see `trend-dca-bounce-placement.md`).

3. **Primary stays primary; hedge is temporary.** The trandm-aligned side is **never**
   invalidated. On fill, the hedge is tagged `hedge_side` (the temp leg). When the pump
   resolves — the trend label is gone (`mkt_mode != 2`) or a fast-candle turns back toward the
   primary (`fc_dn` for a long hedge) — the leg is **closed** (`hedge-cl` / `hedge-cs`) and the
   primary keeps riding. A `cooldown_ticks` gate prevents re-hedge churn on `mkt_mode`
   oscillation.

The old `invalid_side` / best-negative-exit (`neg_exit_l/s`) path is left inert (never set) —
superseded by `hedge_side`.

## State

```rust
// state.rs — State
pub hedge_side: i8,      // live temp leg: +1 long-hedge, -1 short-hedge, 0 none (set on FILL)
pub hedge_arm: i8,       // transient: hedge armed THIS sug tick (dir) → pullback-limit
pub hedge_arm_amt: f64,  // transient: armed size (neutralize stuck side + 5m-mom excess, margin-capped)
// state.rs — Config
pub hedge_ntps_mult: f64, // pump gate: NTPS ≥ this × baseline to arm (default 1.6, live-tunable)
```

`hedge_side` is in-memory only (like `invalid_side`, not in `StateSerde`) — a hedge episode is
short-lived.

## State Transitions

- **Arm (handle_sug_info, each sug tick):** `hedge_arm`/`hedge_arm_amt` reset to 0, then set if
  `enable_hedge` & not already hedged & hold exactly the opposite side & `mkt_mode` trend-against
  & fast-candle that way & `ntps_mult ≥ hedge_ntps_mult` & not on cooldown. Size =
  `stuck_notional × (1 + 5m-mom excess)`, capped by free-margin room.
- **Place (compute_limit_orders):** while `hedge_arm != 0` and the hedge side isn't open yet,
  rest the pullback-limit at `cur_hi − fc_dist` (long) / `cur_lo + fc_dist` (short); only if it
  rests on the correct side of market (below for a buy, above for a sell).
- **Fill (handle_order_update, `hedge-l`/`hedge-s`):** `hedge_side = ±1`, clear `hedge_arm`.
- **Close (handle_sug_info):** if `hedge_side == 1` and (`mkt_mode != 2` or `fc_dn`) →
  `close(Long, "hedge-cl")`, clear `hedge_side`, set `long_close_tick` (cooldown). Mirror for
  `hedge_side == -1`. Also clears if the leg vanished (`!has_long`).

## Signals Derived

| Signal | Source | Meaning |
|--------|--------|---------|
| `hedge_arm` | sug tick | a pump is confirmed → a pullback-limit hedge is resting |
| `hedge_side` | fill/close | the temporary protective leg currently open (±1) |
| `hedge-l` / `hedge-s` | order mark | the resting pullback-limit hedge |
| `hedge-cl` / `hedge-cs` | close reason | the leg closed because the pump resolved |
| debug `hs={}` | run_dbg | live `hedge_side`; `cfg … hntps={}` echoes the gate on apply |

## History of Changes

### v0.58.0 — 2026-05-30 (built, NOT deployed — awaiting user)
- **Why:** the v0.49 market hedge fired twice live on transient `mkt_mode` blips (NTPS ~20),
  market-buying the range top against `trandm −68` and orphaning a losing long when the blip
  reverted. User: *"I like the hedge — but find the best price to open, and only open when we're
  90% sure the token is pumping, using high NTPS."*
- **Built:** pump-gated (`ntps_mult ≥ hedge_ntps_mult`, default 1.6) + fast-candle + `mkt_mode`
  trigger; **pullback-limit** entry (`cur_hi − fc_dist` / `cur_lo + fc_dist`, trails & auto-cancels);
  **primary stays primary** — the hedge is `hedge_side` (temp), closed on pump-resolution
  (`hedge-cl`/`hedge-cs`); `cooldown_ticks` anti-churn. Removed the market hedge from
  `compute_auto_open` and the `invalid_side` role-flip (left inert). New config `hedge_ntps_mult`
  (Start param + live-apply). Debug `hs=` + `cfg … hntps=`. Compiles clean on wasm32, zero warnings.
- **Deferred / tunable:** `hedge_ntps_mult` default 1.6 is a first pass — calibrate from live
  pumps (the real 2021.8 break hit NTPS ~59 ≈ 3× baseline, the quiet grind ~20 ≈ 1×). Could add
  an absolute-NTPS floor and/or a depth (`asset_01`) confirmation later.
