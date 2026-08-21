# Adaptive pattern-engine overview

The module is a market-entry MRC staircase trader. Every supported pattern is
evaluated on every new MRC movement, but only staircase candidates can open a
position. Pullback patterns remain available for analysis and learning without
entry authority.

## Trading model

- Input timeframes: `1m` and `5m` MRC lines plus volume/NTPS; candle OHLC, live price, and cascade speed provide market context
- Position policy: one open position at a time
- Pattern policy: classify all matches, preserve overlaps, and score them independently
- Selection policy: stay with the active staircase pattern until its protected structure ends
- Continuation policy: advance through successive MRC lines while the staircase remains valid
- Execution policy: use stairs for structural context and directional levels; retain cascade, volume, and fast-candle data as diagnostics, while valid stairs force a market entry

## Pattern pipeline

1. `detection/mrc_cross_analysis.rs` converts MRC transitions into normalized movement events.
2. `detection/mrc_pattern_classification.rs::classify_all` runs every matcher and keeps overlapping results.
3. `detection/mrc_crossing.rs` builds a candidate for each match with a directional target, stop, and score.
4. `state.rs` updates the observation ledger and the realized net-PnL ledger for each pattern.
5. The selector picks the best staircase candidate; pullback matches remain
   diagnostic candidates and cannot be executed.
6. `engine.rs` executes only the selected candidate and attributes the completed trade to its opening pattern.

## Line execution plan

`detection/mrc_prediction.rs` builds one small plan for each of the nine MRC
lines. A plan reports signed movement from the entry, absolute distance, stop
distance, the selected ETA and its source, historical sample count, and a route
role. Route roles explain whether a line is the current line, the planned
target, covered on the way to another target, beyond the target, or on the
opposite side. NTPS and its fast-trade timestamp are carried with every plan
as market-speed context; they do not pretend to be a probability.

The selector has an explicit one-direction lock after a directional `start`
signal. Candidates in the opposite direction remain observable but cannot be
selected or executed while the lock is active. The cascade measures active
movement speed and supplies fast-trading context; it does not independently
open a position or veto a structurally valid staircase leg. Only an invalidated
staircase is blocked by the live stairs gate.

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
