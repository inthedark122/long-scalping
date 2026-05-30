# Future Improvements

Observed from bot 703 live run, ETH falling scenario (2026-05-22).

## 1. Use TF mov in allocation and bias

`compute_allocation` and `compute_bias` only use `smc_dir` and `pzone`.
The 5m `mov` score carries real directional signal that is being ignored.

```rust
// compute_allocation: shift long/short ratio based on 5m mov
let mov5_bias = (state.tf5.mov / 100.0) * 0.10; // up to ±10%
long_r -= mov5_bias; // negative mov5 → less long, more short
```

Also apply to `compute_bias`: negative 5m mov → reduce lbias, boost sbias.

## 2. Regime-gated reentry

On `FinishPosition`, the bot immediately reseeds after `cooldown_ticks`.
When 5m mov is strongly negative this keeps reopening longs into a falling market.

Gate reentry on TF state:
```rust
// only reenter long if 5m isn't strongly bearish
if state.long_needs_reentry
    && state.tf5.mov > -40
    && ntps < 400
    && ticks.saturating_sub(long_close_tick) > cooldown_ticks
{
    open_positions.push(seed_position(Direction::Long, seed));
    state.long_needs_reentry = false;
}
// mirror for short: tf5.mov < +40
```

Threshold -40 is a starting point — tune based on observation.

## 3. Increase cooldown_ticks (operational, config change)

Default `cooldown_ticks=5` is ~5 seconds at SugInfo rate.
Too short — new seeds get NTPS-killed before the grid can build (observed: positions
21313/21314 lasted ~8s each).

Raise to 30–60 via Start action. No code change needed.

## 4. Regime classification — tight range at extreme

When floor=ceiling at the same non-zero level (e.g. floor=ceiling=−1 on 5m),
`classify_regime` returns WIN because `(floor >= -1) && (ceiling <= 1)` is both true.
But this pattern means price has been locked at one cross level for 10+ candles —
closer to a trend than a range.

Consider: if `floor == ceiling && floor != 0` → treat as early trend signal.
```rust
if tf.floor == tf.ceiling && tf.floor != 0 {
    return if tf.floor > 0 { 2 } else { -2 }; // early UP / DOWN
}
```
