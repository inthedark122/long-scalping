# ZigZagTrend (`zigzagtrend`) - Direction, Reversal Zones, and Structural Levels

> Source: LTE indicator feed. This is the ZigZag structural read used for trend-following and level placement.

## How to fetch it

Use the full `symbol_key` in the URL, not the plain symbol name.

Example:

```bash
curl -s 'https://okx.tg.trading.sinvoid.me/api/indicators?symbol_key=OKX%23ETH-USDT-SWAP%23SWAP&timeframe=1800&indicator_name=zigzagtrend&limit=1'
```

Useful reminders:

- `symbol_key` format is `EXCHANGE#SYMBOL#INST_TYPE`
- URL-encode `#` as `%23`
- `timeframe=60` = 1m
- `timeframe=300` = 5m
- `timeframe=1800` = 30m
- `timeframe=3600` = 1h
- `indicator_name=zigzagtrend` is the live API value

## What it gives you

`zigzagtrend` exposes:

- `Direction`
- `PrevDirection`
- `EndDirection`
- `Fibo`
- `BuyPrice`
- `SellPrice`
- `BreakPrice`
- `FollowPrice`
- `LastPrice`
- `EndPrice`
- `KLines`

The useful interpretation for trading is:

- `Direction` = pattern state from the 4-dot structure
- `EndDirection` = the current ZigZag direction you usually want to follow
- `BreakPrice` = the current structural break zone
- `BuyPrice` / `SellPrice` = the nearest support / resistance levels built from the last 9 days

## How to read it

### 1. Direction

`Direction` comes from the ZigZag 4-dot pattern:

- `LONG` when the last low is above the previous low
- `SHORT` when the last high is below the previous high

This is useful when the market is not clean and you want to understand the current structure pattern.
It is not always the main trading direction by itself.

### 2. EndDirection

`EndDirection` is the most practical direction field for most cases.

Use it as:

- `LONG` = follow long direction
- `SHORT` = follow short direction

This is the direction that should usually drive the trade bias unless the higher timeframe disagrees or the market is clearly in a reversal zone.

### 3. BreakPrice

`BreakPrice` is the key prediction level.

Use it as the price that may:

- break and continue
- hold and reject
- become the start of a reversal zone

This is especially useful on 5m / 15m / 30m because price often tests the break price and then reverses.
If the market keeps failing to break it, that is useful information too.

### 4. BuyPrice / SellPrice

`BuyPrice` and `SellPrice` are the nearest structural prices collected from the last 9 days.

Use them as support / resistance lines:

- `BuyPrice` = support side
- `SellPrice` = resistance side

These levels are useful for order placement and for seeing whether price has enough power to break through.
Strong movement can still break them, but often it does so slowly and with repeated tests.

### 5. Fibo

`Fibo` is the retracement depth of the current ZigZag leg.

Simple read:

- low `Fibo` = move is early / not stretched
- high `Fibo` = move is late / stretched

This is not the main direction signal, but it helps decide whether the current direction still has room.

## Practical rules

### 1. Trend-following rule

If `EndDirection=LONG`, trade long only when the market keeps continuing long.

Do not trade just because `EndDirection` is long.
Trade long only when the price action confirms continuation.

### 2. Reversal-zone rule

Use `BreakPrice` to identify a possible reversal zone.

The market often does one of these:

- breaks the level and continues
- touches the level and rejects
- tests the level multiple times before breaking

That makes `BreakPrice` useful both for prediction and for order placement.

### 3. Support / resistance rule

Use `BuyPrice` and `SellPrice` as structural support and resistance.

This is especially useful when:

- the move is slow
- the market is coiled
- you want to place orders near the line instead of chasing price

## Range vs directional use

This indicator can be used in two ways:

- direction detection: follow `EndDirection`
- structure detection: use `BreakPrice`, `BuyPrice`, and `SellPrice`

The best use is usually:

- `Direction` for the pattern state
- `EndDirection` for trade bias
- `BreakPrice` for the likely reaction zone
- `BuyPrice` / `SellPrice` for support / resistance

## Prediction model

Use ZigZagTrend as a prediction tool like this:

- if price is below `SellPrice` and keeps failing to break it, expect rejection or pause
- if price breaks `BreakPrice` cleanly, expect continuation in that direction
- if price keeps testing the same level without breaking, count that as weakness in the move
- if `EndDirection` flips, the current leg may be ending

## How to count break attempts

The feed only returns the latest structural view, not a long history of every failed break.

So the practical approach is:

- store the latest read while the bot is running
- track how many times price touches or rejects `BreakPrice`
- compare the live price against the current level over time

That lets us build a better trend/reversal memory inside the bot.

## Suggested score

You can reduce the read to a simple score:

```text
trend_score = EndDirection
structure_score = price vs BreakPrice
sr_score = price vs BuyPrice / SellPrice
```

Interpretation:

- `EndDirection` aligned with price continuation = clean trend
- repeated failure at `BreakPrice` = reversal pressure
- price near `BuyPrice` or `SellPrice` = structural reaction zone

## Recommended usage

Use `zigzagtrend` as a structure-first indicator.

- use `Direction` to understand the pattern state
- use `EndDirection` to choose the trade direction
- use `BreakPrice` for prediction / reversal zone
- use `BuyPrice` and `SellPrice` for support / resistance placement
- use `Fibo` to decide whether the current leg is still fresh or already stretched

## Practical summary

- `Direction` = 4-dot pattern state
- `EndDirection` = current tradable ZigZag direction
- `BreakPrice` = likely reaction / reversal zone
- `BuyPrice` = support line
- `SellPrice` = resistance line
- repeated break failure = weak move
- clean break = continuation

## Higher timeframe usage

For 5m / 15m / 30m / 1h:

- use `EndDirection` as the main directional context
- use `BreakPrice` to place or stage orders
- use `BuyPrice` / `SellPrice` as the broader structural lines
- prefer the direction that continues to hold rather than the one that only flashes once

## Notes

- The indicator is useful for both trend direction and structural prediction.
- It is not a guarantee. It is a structural map for continuation and reversal zones.
- For the best result, combine it with `SMC` to compare swing direction against structural levels.
