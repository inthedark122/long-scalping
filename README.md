# Long Scalping

`1.0.2` is the limit-only release for a one-position regime trader built for the LTE dynamic module bridge.

The module uses only `MRC`, `DPSignal`, `ZigZagTrend`, candle OHLC, and live price. `5m` is the primary regime timeframe, `1m` handles execution, `15m` is the fallback confirmer, and `1h` is context only. The engine always classifies the market as `Long`, `Short`, or `Sideways`.

Risk sizing now has two public knobs: `cap_init` and `trading_amount`. `cap_init` is a percentage from `0` to `100` and seeds the internal budget from `max_amount * cap_init / 100`. `trading_amount` is a percentage from `10` to `100` and caps the normal entry size as a fraction of the deployable budget (`budget × leverage`). Profits still grow the internal budget automatically.

Start with [HANDOFF.md](/home/dev/strategies/long-scalping/HANDOFF.md) for the current operating model, then see [docs/strategy-overview.md](/home/dev/strategies/long-scalping/docs/strategy-overview.md) for the detector pipeline.
