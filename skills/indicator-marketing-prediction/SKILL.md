---
name: indicator-marketing-prediction
description: Turn the live MRC and volume read into a concise trade-facing summary. Use when the user asks for a current market read, prediction, entry, TP, or how to trade.
---

# MRC and volume prediction

Use only the strategy's supported inputs:

- MRC 200 lines on 1m and 5m;
- volume 50 gain, gain bounds, and buy/sell gain;
- candle OHLC and live price for crossing and execution context.

Fetch the data with the full URL-encoded symbol key and `timeframe=60` or
`timeframe=300`. Do not consult SMC, ZigZag, NTPS, higher timeframes, or a
second directional strategy.

The response should state:

- the current MRC pattern or the reason it is still waiting;
- the likely next MRC line and price area;
- whether volume supports continuation or signals exhaustion;
- the MRC entry, target, and stop when a valid candidate exists;
- how to keep trading a staircase until another pattern becomes more
  profitable.

The active pattern is selected from all matching pattern candidates. A single
weak observation is not enough to switch away from an active profitable
pattern; explain the challenger and its realized evidence when discussing a
switch.
