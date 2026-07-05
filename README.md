# Long Scalping

`1.0.0` is a clean baseline for a one-position regime trader built for the LTE dynamic module bridge.

The module uses only `MRC`, `DPSignal`, `ZigZagTrend`, candle OHLC, and live price. `5m` is the primary regime timeframe, `1m` handles execution, `15m` is the fallback confirmer, and `1h` is context only. The engine always classifies the market as `Long`, `Short`, or `Sideways`.

Start with [HANDOFF.md](/home/dev/strategies/long-scalping/HANDOFF.md) for the current operating model, then see [docs/strategy-overview.md](/home/dev/strategies/long-scalping/docs/strategy-overview.md) for the detector pipeline.
