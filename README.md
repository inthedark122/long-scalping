# Long Scalping

`1.0.2` is the limit-only release for a one-position regime trader built for the LTE dynamic module bridge.

The module uses only `MRC`, `DPSignal`, `ZigZagTrend`, candle OHLC, and live price. `5m` is the primary regime timeframe, `1m` handles execution, `15m` is the fallback confirmer, and `1h` is context only. The engine always classifies the market as `Long`, `Short`, or `Sideways`.

Risk sizing now has one public knob: `cap_init`. It is a percentage from `0` to `100` and seeds the internal deployable budget from `max_amount * cap_init / 100`. Profits then grow that budget internally: first by doubling until the cap is reached, then by leverage-based profit reinvestment.

Start with [HANDOFF.md](/home/dev/strategies/long-scalping/HANDOFF.md) for the current operating model, then see [docs/strategy-overview.md](/home/dev/strategies/long-scalping/docs/strategy-overview.md) for the detector pipeline.
