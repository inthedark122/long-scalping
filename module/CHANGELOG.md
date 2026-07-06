# Changelog

## 1.0.3

Fast-candle entry gating and exit-state tracking.

- Restored `fast_candle_entry_block` as a dedicated confirmation gate for first entry and continuation filtering.
- Kept entry wait policy layered on top of the FC gate with post-SL reclaim checks and TP reliability checks.
- Added stored TP/SL tracking in position runtime so the next tick can reason about the last exit cleanly.
- Preserved the one-position limit-only model and the current `cap_init` / `trading_amount` risk surface.

## 1.0.2

`cap_init` risk-model reset.

- Removed the public multi-variable sizing stack.
- Added a single Start parameter: `cap_init` as a percentage from `0` to `100`.
- Internal deployable budget now starts from `max_amount * cap_init / 100`.
- Profits double the internal budget until that cap is reached, then reinvest with leverage.
- Limit entries and the one-position trading model stay intact.

## 1.0.1

Limit-only entry release.

- Removed the last market-open helper from the active path.
- Initial entries now use `open_positions` with `Limit` order payloads only.
- Manual force-open actions were aligned to the same limit-entry flow.
- Bot-group config should now point at `1.0.1` for the LAB module repo.

## 1.0.0

Fresh baseline reset.

- Removed legacy branch history, retired execution paths, and compatibility config.
- Reduced the public config surface to `seed_pct`, `cap_pct`, `risk_pct`, `stop_pct`, `tp_pullback_pct`, `cooldown_ticks`, `confirm_tf_fallback`, and `debug_info`.
- Rebuilt the Rust module around a detector pipeline:
  - `market_data.rs`
  - `detection/mrc.rs`
  - `detection/cascade.rs`
  - `detection/vlines.rs`
  - `detection/sharp_return.rs`
  - `detection/regime.rs`
  - `decision.rs`
  - `execution.rs`
  - `debug.rs`
  - `engine.rs`
- Strategy inputs are now limited to `MRC`, `DPSignal`, `ZigZagTrend`, candle OHLC, and live price.
- Trading behavior is deterministic: exactly one market state, exactly one open position at a time, hard stop first, TP on pullback second, regime invalidation last.
