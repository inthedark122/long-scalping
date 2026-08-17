# Adaptive pattern-engine overview

The module is a limit-entry MRC pattern trader. It does not choose between
separate strategy modes. Every supported pattern is evaluated on every new MRC
movement, and one selector chooses the best currently tradable candidate.

## Trading model

- Input timeframes: `1m` and `5m` MRC lines plus volume; candle OHLC and live price provide market context
- Position policy: one open position at a time
- Pattern policy: classify all matches, preserve overlaps, and score them independently
- Selection policy: stay with the active pattern until a challenger beats it by the configured switch margin
- Continuation policy: advance through successive MRC lines while a staircase or outward continuation remains valid

## Pattern pipeline

1. `detection/mrc_cross_analysis.rs` converts MRC transitions into normalized movement events.
2. `detection/mrc_pattern_classification.rs::classify_all` runs every matcher and keeps overlapping results.
3. `detection/mrc_crossing.rs` builds a candidate for each match with a directional target, stop, and score.
4. `state.rs` updates the observation ledger and the realized net-PnL ledger for each pattern.
5. The selector picks the best candidate, with hysteresis to avoid noisy pattern flapping.
6. `engine.rs` executes only the selected candidate and attributes the completed trade to its opening pattern.

## Pattern learning

Each pattern tracks observations, trades, wins, total net PnL, average net PnL,
and an exponential moving average of net PnL. New patterns use movement evidence
as a prior; realized trades progressively dominate that prior. Research mirrors
this process with seven virtual accounts so pattern quality can be compared
without mixing outcomes between patterns.

## Risk model
- The public risk knobs are `cap_init`, `trading_amount`, and optional `risk_target_percent`
- `cap_init` seeds the internal budget from `max_amount * cap_init / 100`
- `trading_amount` caps the normal entry size as a fraction of the deployable budget (`budget × leverage`)
- `risk_target_percent` is disabled at `0`; when enabled, entry margin is multiplied by `min(1, target stop-distance % / projected stop-loss %)`, using price movement percentages independent of leverage
- The adjustment applies to aggregate exposure and rejects entries that cannot satisfy exchange minimum notional without exceeding the configured risk target
- Profit first doubles the budget until the cap is reached
- After the cap is reached, further profit growth is reinvested with leverage
