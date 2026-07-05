# V-Movement Tracker

## Purpose

Detects when price makes successive "V-shaped" spikes through any MRC band line and measures the relative strength of each spike. Successive Vs with decreasing penetration = exhaustion of the stop hunt = safe to resume normal TP/DCA logic.

The tracker is **line-agnostic**: it auto-detects which MRC line is currently acting as the pivot (support/resistance) and tracks Vs around that line. The active line can be any of the 9 MRC levels: UpBig, UpOuter, UpSmall, UpInner, MeanLine, DownInner, DownSmall, DownOuter, DownBig.

---

## Core Concept

A **V-movement** is:
1. Price crosses through an MRC line (in either direction)
2. Price reaches a local extreme on the far side of that line
3. Price returns back through the line

Each V has measurable **power**: how far it penetrated beyond the line (as % of total MRC range).

**Long V** (cross_direction = +1): price crossed above the line — a potential stop hunt upward.  
**Short V** (cross_direction = -1): price crossed below the line — a potential stop hunt downward.

Both sides are tracked independently around the same active line. The **active line** is the MRC line price most recently crossed. When price crosses a new line, the tracker migrates to that line and resets V history.

When a V fails to reach 70% of the prior V's penetration depth → **exhaustion** → the stop hunt is done, the move is genuine.

See [[window-stairs]] for how the long/short V sequences are combined to detect ranging windows and staircase momentum.

---

## MRC Lines (in order)

```
UpBig        mrc_ub   (index 8)
UpOuter      mrc_uo   (index 7)
UpSmall      mrc_us   (index 6)
UpInner      mrc_ui   (index 5)
MeanLine     mrc_ml   (index 4)
DownInner    mrc_di   (index 3)
DownSmall    mrc_ds   (index 2)
DownOuter    mrc_do   (index 1)
DownBig      mrc_db   (index 0)
```

---

## State

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct VSide {
    pub count: u8,          // closed Vs recorded on this side
    pub last_power: f64,    // penetration % of most recent closed V
    pub prev_power: f64,    // penetration % of V before that
    pub exhausted: bool,    // last_power < prev_power * 0.70
    pub stair: bool,        // last_power > prev_power * 1.10 (momentum building)
}

// One per MRC line — 9 total, indexed 0=DB 1=DO 2=DS 3=DI 4=ML 5=UI 6=US 7=UO 8=UB
pub struct VLineTracker {
    pub was_above: Option<bool>,  // previous tick: was price >= this line? (None = uninit)
    pub peak: Option<f64>,        // max price seen while above (open long V)
    pub trough: Option<f64>,      // min price seen while below (open short V)
    pub long_side: VSide,         // Vs that crossed above this line
    pub short_side: VSide,        // Vs that crossed below this line
}

// State stores all 9:
pub v_lines: [VLineTracker; 9],
```

**Design note**: each line tracks its own V history independently and permanently — no reset when price oscillates through adjacent lines between Vs. This means V1/V2/V3 exhaustion comparison works correctly across multiple separate UpBig probes even with ranging in between.

The `stair` field in `VSide` feeds the [[window-stairs]] detection. `exhausted` is the primary V-movement signal.

---

## State Transitions

Called on every SugInfo tick with current price. Runs once per MRC line per tick:

```
For each line X (level = lvl):

  now_above = price >= lvl

  If was_above == None (first tick):
    was_above = now_above
    if now_above: peak = price
    else: trough = price

  Else if now_above != was_above (CROSSING):
    if now_above (crossed above):
      close short V:
        power = (lvl - trough).abs() / mrc_range * 100
        short_side.prev_power = short_side.last_power
        short_side.last_power = power
        short_side.count += 1
        short_side.exhausted = count>=2 && last < prev * 0.70
        short_side.stair    = count>=2 && last > prev * 1.10
      trough = None
      peak   = price   ← start tracking new long V

    if !now_above (crossed below):
      close long V:
        power = (peak - lvl).abs() / mrc_range * 100
        long_side.prev_power = long_side.last_power
        long_side.last_power = power
        long_side.count += 1
        long_side.exhausted = count>=2 && last < prev * 0.70
        long_side.stair    = count>=2 && last > prev * 1.10
      peak   = None
      trough = price   ← start tracking new short V

    was_above = now_above

  Else (same side, no crossing):
    if now_above: peak   = max(peak,   price)
    else:         trough = min(trough, price)
```

**Key property**: crossing history at each line accumulates forever. When price oscillates between UpBig and UpOuter between V spikes, `v_lines[8]` (UpBig) keeps its `long_side.count` intact — V1, V2, V3 are all compared at the same line without reset.

---

## Signals Derived

| State | Meaning | Action |
|-------|---------|--------|
| `v_active == true`, `v_cross_dir == +1` | Stop hunt above line in progress | Block short DCA; place s-ts BUY at 40% of V peak height above line |
| `v_active == true`, `v_cross_dir == -1` | Stop hunt below line in progress | Block long DCA; place l-ts SELL at 40% of V trough depth below line |
| `long_side.exhausted == true` | Long V power fading — genuine downward return | Resume normal short TP/DCA; s-ts reverts to standard formula |
| `short_side.exhausted == true` | Short V power fading — genuine upward return | Resume normal long TP/DCA |
| `active_line_idx == -1` | No line interaction detected | Normal operation |

Window and Stairs signals (combining long_side + short_side) are in [[window-stairs]].

---

## s-ts Price Formula (short side, long V active)

When a long V is active above an MRC line, place the s-ts BUY at a fraction of the V's penetration depth rather than at the line itself — anchoring exactly at the line is too strict since price may range above for many ticks without returning.

```rust
let ts_price = if v_tracker.v_active && v_tracker.v_cross_dir == 1 {
    // Place at 40% of peak height above line
    let v_mid = v_tracker.active_line_level
        + (v_tracker.v_peak_price - v_tracker.active_line_level) * 0.40;
    v_mid.min(price - tick)
} else if v_tracker.long_side.count > 0 && !v_tracker.long_side.exhausted {
    // Long V closed, second test expected — anchor at line
    v_tracker.active_line_level
} else {
    // Normal formula
    (price - (state.mrc_range * 0.05).max(4.0 * tick)).min(state.mrc_ml)
};
```

The 0.40 factor is tunable: lower catches retracement earlier (more fills, smaller closes); higher waits deeper (fewer fills, larger closes).

---

## NTPS Context

Low NTPS + price moving through an MRC line = likely **liquidity hunt** (stop sweep). When NTPS < 0.75 × avg_ntps and price crosses an MRC line, treat the move as a V candidate and activate tracking immediately.

---

## History of Changes

### v0.31.2 — Concept defined, UpBig-specific (2026-05-28, not yet implemented)

Initial design hardcoded to UpBig. Live session: bot 694 spiked above UpBig (1989.56) to 1994.45 with NTPS=112-124. Bot DCA'd short 3 times during the spike (sn: 7.9→13.9→19.9). V-tracker would have blocked all 3 adds.

V1: peak=1994.45, penetration=20.4%. V2: peak=1991.50, penetration=8.1%. V2/V1=39.7% → exhaustion confirmed.

### v0.31.2 — Generalized to any MRC line (2026-05-28, not yet implemented)

Revised to auto-detect the active MRC line from price crossings. V tracking migrates between lines as price moves through the MRC structure. Any line can be the V pivot — MeanLine, DownInner, UpInner, UpSmall, UpOuter, UpBig, etc.

Also revised s-ts price: instead of anchoring exactly at the line (too strict — price may range above for many ticks without returning), use 40% of V penetration depth above the line.

### v0.32.0 — Per-line VLineTracker, dual-side, permanent history (2026-05-28, not yet deployed)

Replaced the single-active-line VTracker with 9 independent `VLineTracker` instances — one per MRC line. Each line tracks its own `long_side` and `short_side` VSide permanently.

**Key fix**: the original design reset V history when price migrated through adjacent lines between spikes. This meant V1 and V2 at UpBig could never be compared (each reset to count=0). The per-line design never resets — V1, V2, V3 at UpBig all accumulate in `v_lines[8].long_side` regardless of price movement between them.

`stair` flag added to VSide: fires when each successive V penetrates deeper than the prior (momentum building rather than exhausting). See [[window-stairs]] for how stair+exhaustion combine into window and staircase detection.

**Live validation**: bot 694 session (2026-05-28):
- V1 above UpBig (1989.56): peak=1994.45, power=20.4%
- V2 above UpBig: peak=1991.50, power=8.1% → exhaustion (8.1% < 20.4%×0.70=14.3%) 
- V3 above UpBig: peak=1990.11, power=2.3% → triple exhaustion confirmed

With VLineTracker deployed: `v_lines[8].long_side` would record count=3, last_power=2.3%, exhausted=true, confirming no further UpBig stop hunts expected.

### v0.56.0 — Single-source V: removed the parallel crossing-counter (2026-05-30, built, not deployed)

The codebase had **two** V/bounce mechanisms: this real-time `VLineTracker` (A) **and** a separate candle-close `level_stats` crossing-counter (B, `update_level_stats`). B was mislabeled — its comment claimed an A→B→A V-pattern via `cross_prev2`, but the code only counted raw boundary crossings; `cross_prev2`, `current_tests`, `min_dist`, `crossings` were dead (written, never read). User decision (2026-05-30): *"Can we remove completely B? I would like to have only A."*

**Removed entirely:** the `LevelStats` struct, `level_stats: [LevelStats; 9]`, `update_level_stats`, `update_level_min_dists`, and the dead `cross_prev2` field/writes.

**All four B consumers re-sourced to A** (this tracker):
- `v_bull_fresh` / `v_bear_fresh` (sideways `auto-l`/`auto-s` seed) → A's `last_cross_dir` + `last_cross_tick` within `V_CROSS_WINDOW` (90 ticks ≈ 6 candles).
- `stair_long_ok` / `stair_short_ok` (stairs entry gate) → A's `last_cross_dir` + `last_cross_tick` within `STAIR_CROSS_WINDOW` (180 ticks ≈ 12 candles).
- `stair_lscale` / `stair_sscale` (stair sizing) → A's real completed-V counts `long_side.count` / `short_side.count` (replacing the raw `v_bear` / `v_bull` crossing tallies — a strict upgrade: these are power-measured Vs).
- `strong_long` / `strong_short` (size asymmetry, `compute_entry_scale`) → A's `last_cross_tick` age (`RECENT_BREAK_TICKS=450 ≈30c`, `STALE_BREAK_TICKS=900 ≈60c`), still anchored by candle-based `zone_streak`.

**Trade-off:** A's recency is tick-based (~15 ticks/candle, drifts with market activity) vs B's candle-exact aging. The only place that matters is `strong_long`/`strong_short` (30–60 candle staleness); `zone_streak` (candle-based) still anchors it, bounding the drift. Everything else A does better (real penetration power, not just a crossing tally). Net: one V system, ~80 fewer lines, no dead code. Compiles clean, zero warnings.

**State note:** the live struct also carries `last_cross_tick: u64` and `last_cross_dir: i8` (added for the Timing-0.2 sug-tick entry, now the backbone of all freshness/age signals) beyond the fields shown in the State section above.

### v0.56.0 — Cold-start seeding from candle history (2026-05-30, built, not deployed)

With B gone, A started **blind** at every (re)start and warmed up over ~6–12 min of live ticks before freshness/stair signals were usable. User: *"can we implement cold start? … currently only 10 candles available, let's implement it to have at least correct implementation to be ready for more candles."*

**Built — `seed_v_lines(state, ind)`, called once at `Init`:**
- **Depth-agnostic.** Scans the MRC history until it runs out (cap 512), so it uses 10 candles today and *N* candles automatically when the feed sends more.
- Replays each candle **oldest → newest** through the shared `apply_v_lines()` core (extracted from `update_v_lines` so live + seed share one implementation), using **that candle's own MRC bands**.
- Feeds each candle's **High / Low / Close in temporal order by direction** (`up: low→high→close`, `down: high→low→close`) so upper/lower **wick rejections register as real Vs with measured penetration power** — not just close-to-close crossings.
- Runs on a **synthetic tick timeline**: candle *k* (0 = oldest) → tick `(k+1)·TICKS_PER_CANDLE` (=15). After seeding, `state.ticks = depth·15` so **live ticks continue on the same scale** and the freshness windows age correctly.

**Window behaviour with the current 10 candles** (verified): the most recent ~6 candles seed as **fresh** (V window = 6 candles), all 10 feed the **stair** window (12 candles), and the **strong_long/short** staleness check (30–60 candles) correctly stays dormant — it can't trigger until ≥60 candles exist, which is the intended "ready for more candles" property. `zone_streak` still anchors `strong_*` meanwhile.

**Net:** the bot is no longer blind at start — the entry modes already fired cold; now the `auto` mode-seeds + stair sizing also have immediate historical context. Init debug gains `vt=` so you can see the seeded V-state. Compiles clean, zero warnings. When the feed is upgraded to send >10 candles, **no code change is needed** — depth scales automatically.
