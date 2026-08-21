# MRC pattern scalping

This module is a one-position adaptive pattern trader built for the LTE dynamic module bridge. Valid staircase entries force an immediate market open so a continuation is not lost waiting for a one-candle limit fill.

The module uses MRC lines, volume, candle OHLC, and live price to classify the active patterns independently. Each pattern keeps its own realized-PnL ledger; only staircase candidates can open positions. Pullback data remains analysis and learning evidence, not an entry authority. MRC line-to-line staircase continuation is the core execution path.

Risk sizing has three public knobs: `cap_init`, `trading_amount`, and the optional `risk_target_percent`. `cap_init` is a percentage from `0` to `100` and seeds the internal budget from `max_amount * cap_init / 100`. `trading_amount` is a percentage from `10` to `100` and caps the normal entry size as a fraction of the deployable budget (`budget × leverage`). When `risk_target_percent` is greater than zero, the aggregate entry size is reduced when the projected stop-loss price movement is wider than the target; `0` disables this protective adjustment. Profits still grow the internal budget automatically.

Start with [HANDOFF.md](/home/dev/strategies/long-scalping/HANDOFF.md) for the current operating model, then see [docs/mrc-pattern-classification.md](/home/dev/strategies/long-scalping/docs/mrc-pattern-classification.md) for the detector and adaptive selector architecture.

For historical pattern classification and per-pattern trade reports, run:

```bash
cargo run --manifest-path module/Cargo.toml --locked --features mrc-research,native-tools --bin mrc_crossing_research -- --symbol-key 'OKX#CAP-USDT-SWAP#SWAP' --symbol-key 'OKX#BEAT-USDT-SWAP#SWAP' --symbol-key 'OKX#ETH-USDT-SWAP#SWAP' --output tmp/reports/ --html tmp/reports/
```
