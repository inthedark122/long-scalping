# MRC pattern scalping

This module is the limit-only release for a one-position adaptive pattern trader built for the LTE dynamic module bridge.

The module uses MRC lines, volume, candle OHLC, and live price to classify every supported pattern independently. Each pattern keeps its own realized-PnL ledger; the selector trades the best candidate and stays with it until another candidate earns a meaningful advantage. MRC line-to-line continuation is the core execution path.

Risk sizing has three public knobs: `cap_init`, `trading_amount`, and the optional `risk_target_percent`. `cap_init` is a percentage from `0` to `100` and seeds the internal budget from `max_amount * cap_init / 100`. `trading_amount` is a percentage from `10` to `100` and caps the normal entry size as a fraction of the deployable budget (`budget × leverage`). When `risk_target_percent` is greater than zero, the aggregate entry size is reduced when the projected stop-loss price movement is wider than the target; `0` disables this protective adjustment. Profits still grow the internal budget automatically.

Start with [HANDOFF.md](/home/dev/strategies/long-scalping/HANDOFF.md) for the current operating model, then see [docs/mrc-pattern-classification.md](/home/dev/strategies/long-scalping/docs/mrc-pattern-classification.md) for the detector and adaptive selector architecture.

For historical pattern classification and per-pattern trade reports, run:

```bash
cargo run --locked --features mrc-research,native-tools --bin mrc_crossing_research -- --symbols ALLO,CAP,BILL
```
