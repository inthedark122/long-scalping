# Indicator Instructions

This folder holds practical notes for reading live LTE indicators in the strategy.

## How to fetch indicator data

Always use the full `symbol_key` in the URL:

```text
EXCHANGE#SYMBOL#INST_TYPE
```

Example:

```bash
curl -s 'https://okx.tg.trading.sinvoid.me/api/indicators?symbol_key=OKX%23ETH-USDT-SWAP%23SWAP&timeframe=1800&indicator_name=smc&limit=1'
```

Rules:

- URL-encode `#` as `%23`
- use `indicator_name=smc`
- use `timeframe=60` for 1m
- use `timeframe=300` for 5m
- use `timeframe=1800` for 30m
- use `timeframe=3600` for 1h

## How to read `SMC`

Use the SMC fields for directional structure and local price levels.

Short version:

- `Direction` confirms directional bias
- `DirectionSwing` shows the broader swing direction
- `LowPrice` / `HighPrice` provide local structural bounds
- `BreakPrice` and `SBP` identify structural break levels
- `OBLow` / `OBHigh` provide the active order-block range
- 1m + 5m agreement is cleaner for timing
- 30m + 1h is better for direction context and exit management

## Current usage rule

- use lower TF for entry timing
- use higher TF for directional bias
- if higher TF is stretched, ignore small fixed TP
- prefer trailing stop or exit on lower-TF flip to `LONG` / mixed

## Output format for marketing-style asks

When the request is for a marketing summary, include trade-facing fields in the response:

- direction
- entry
- TP
- prediction
- price prediction
- how to trade

Use this rule:

- `entry` should reflect the actionable area, not just the current price
- `TP` should be explicit when it makes sense, but omit or de-emphasize it if the higher TF is stretched
- `prediction` should explain the most likely next move in plain language
- `price prediction` should give a concrete expected area or range when possible, even if approximate
- `how to trade` should say whether to enter now, wait, trail, or exit on lower-TF confirmation

If the setup is stretched or mixed:

- say that fixed TP is weak or should be ignored
- prefer trailing stop
- prefer exit on lower-TF flip to `LONG` or sideways/mixed
- still include a directional price expectation, such as likely retest, continuation, bounce, or range between nearby levels

## Related notes

- [ZigZagTrend notes](./zigzag.md)
- [MRC prediction read](./mrc-prediction.md)
