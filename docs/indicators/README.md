# MRC and volume inputs

The strategy input surface is intentionally small: MRC bands and volume on the
1m and 5m timeframes. Candle OHLC and live price are market data used to detect
crossings and execute orders; no directional indicator is consulted.

## Fetching research data

Use the full `symbol_key` and URL-encode `#` as `%23`:

```bash
curl -s 'https://okx.tg.trading.sinvoid.me/api/indicators?symbol_key=OKX%23ETH-USDT-SWAP%23SWAP&timeframe=60&indicator_name=mrc_200&limit=1'
curl -s 'https://okx.tg.trading.sinvoid.me/api/indicators?symbol_key=OKX%23ETH-USDT-SWAP%23SWAP&timeframe=60&indicator_name=volume_50&limit=1'
```

The live module reads `mrc_200` and `volume_50` for 1m and 5m. The historical
research loader uses the same two indicator families for each 1m candle.

## MRC fields

MRC provides the ordered nine-line channel:

`DownBig`, `DownOuter`, `DownSmall`, `DownInner`, `MeanLine`, `UpInner`,
`UpSmall`, `UpOuter`, and `UpBig`.

The detector records line crossings, classifies every matching local pattern,
and chooses the most profitable candidate. The target and stop are always MRC
lines calculated from the actual entry price.

## Volume fields

Volume is supporting evidence and a continuation/exhaustion signal. The module
accepts `Gain`, `GainMin`, `GainMax`, and `BuySellGain` from `volume_50`. Missing
volume fields are neutral zeroes; missing MRC fields invalidate the snapshot.

Volume does not replace pattern classification or create a second strategy. It
helps the MRC selector and risk gate decide whether the current MRC movement is
strong enough to keep trading.
