# Strategy Overview

This is the limit-entry release for the long-scalping module.

## Trading model
- Detection regime: `5m`
- Execution timeframe: `1m`
- Confirmation fallback: `15m`
- Long context only: `1h`
- Allowed states: `Long`, `Short`, `Sideways`
- Position policy: one open position at a time

## Inputs
- `MRC`
- `DPSignal`
- `ZigZagTrend`
- Candle OHLC
- Live price

## Pipeline
1. `market_data.rs` reads candles, MRC bands, DP direction, and ZigZag direction for `1m`, `5m`, `15m`, and `1h`.
2. `detection/mrc.rs` computes zone, window position, and mean-line slope.
3. `detection/cascade.rs` tracks directional movement using candle path plus DP/ZZ agreement.
4. `detection/vlines.rs` tracks MRC crossing memory.
5. `detection/sharp_return.rs` marks sharp moves that fail to return quickly.
6. `detection/regime.rs` classifies `5m`, falls back to `15m`, and lets `1h` only block overconfident entries.
7. `decision.rs` outputs one state and an entry allowance.
8. `execution.rs` enforces one-position trading and a limit-only entry with the internal `cap_init` bankroll model plus `trading_amount` entry guard.

## Risk model
- The public risk knobs are `cap_init` and `trading_amount`
- `cap_init` seeds the internal budget from `max_amount * cap_init / 100`
- `trading_amount` caps the normal entry size as a fraction of the deployable budget (`budget × leverage`)
- Profit first doubles the budget until the cap is reached
- After the cap is reached, further profit growth is reinvested with leverage
