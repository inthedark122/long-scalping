# DP Signal (`DPSignal`) - Direction, Stretch, and Range/Out-of-Range Read

> Source: LTE indicator feed. The API name is `dpsignal` in the URL, while the strategy indicator
> list shows `DPSignal`.

## How to fetch it

Use the full `symbol_key` in the URL, not the plain symbol name.

Example:

```bash
curl -s 'https://okx.tg.trading.sinvoid.me/api/indicators?symbol_key=OKX%23ETH-USDT-SWAP%23SWAP&timeframe=1800&indicator_name=dpsignal&limit=1'
```

Useful reminders:

- `symbol_key` format is `EXCHANGE#SYMBOL#INST_TYPE`
- URL-encode `#` as `%23`
- `timeframe=60` = 1m
- `timeframe=300` = 5m
- `timeframe=1800` = 30m
- `timeframe=3600` = 1h
- `indicator_name=dpsignal` is the live API value

## What it gives you

`DPSignal` exposes:

- `Direction`
- `Zee`
- `Zee1`
- `Zee2`
- `ZeeKLines`
- `Zee1KLines`
- `Zee2KLines`

The useful interpretation for trading is:

- `Zee1` = the signal anchor
- `Zee` = the live moving reference
- `Zee2` = older secondary reference

So if you want to understand how far price has moved, use `Zee1` as the base, not `Zee`.

## How to read it

### 1. Direction

`Direction` is the current DP bias:

- `LONG` = upward pressure
- `SHORT` = downward pressure

This is the first read, but it is not enough on its own. Direction tells you where the signal is
leaning, not how stretched the market already is.

### 2. Signal distance

The main distance to watch is:

```text
signal_distance_pct = (price - Zee1) / Zee1 * 100
```

Interpretation:

- close to `0%` = near the signal anchor, more range-like
- `> +3%` = meaningfully stretched upward
- `< -3%` = meaningfully stretched downward

This is the best metric for "how far has the move already run".

### 3. Live-edge distance

Use `Zee` as the live edge / local turning area:

```text
edge_distance_pct = (price - Zee) / price * 100
```

Interpretation:

- `abs(edge_distance_pct) < 1.5%` = price is near the current edge
- if price is far from `Zee1` and close to `Zee`, the move is stretched and may be near a turn

## Range vs out-of-range

This is the practical rule:

- **In range**: price is still close to `Zee1`
- **Out of range**: price is far from `Zee1`
- **Best directional setup**: price is far from `Zee1` and close to `Zee`

Why this matters:

- in-range reads are better for mean-reversion or waiting
- out-of-range reads are better for directional trading

## Prediction model

Use the two distances together:

- `signal_distance_pct` = how much the move has already traveled
- `edge_distance_pct` = whether price is near the current pivot/turn zone

Simple prediction:

- large `signal_distance_pct` and small `abs(edge_distance_pct)` = stretched move, likely continuation is weaker and reversal risk is higher
- large `signal_distance_pct` and `Direction` aligned on both 1m and 5m = directional move is strong, but late
- opposite 1m/5m directions = one timeframe is trying to reverse while the higher timeframe is still in control

## Suggested score

You can reduce the read to a simple score:

```text
stretch_score = abs(signal_distance_pct)
edge_score = 100 - abs(edge_distance_pct)
```

Interpretation:

- `stretch_score` high, `edge_score` high = stretched and near a turn
- `stretch_score` low = still in range
- `stretch_score` high, `edge_score` low = extended but not yet at the turn zone

## Worked examples

### BASED

If price is about `5%` below `Zee1` and close to `Zee`, that means:

- the move is already stretched
- the market is near the live edge
- the token is likely in a bottoming / reversal zone

This is the kind of read that can support a directional short that is near exhaustion or a bounce
candidate if the lower timeframe flips.

### LAB

If price is far above `Zee1` on 1m, but 5m still disagrees, that means:

- the move is strong
- the short-term signal is extended
- higher timeframe confirmation is not complete

This is usually a momentum watch, not a clean all-in directional trade.

## Recommended usage

Use `DPSignal` as a context filter, not as a standalone entry trigger.

- `Zee1` tells you how far the move has run
- `Zee` tells you whether price is near the live turning area
- `Direction` tells you which side is currently leading

Best practice:

- use `DPSignal` on both 1m and 5m
- require timeframe agreement for clean direction
- treat large distance from `Zee1` as stretch / exhaustion information
- treat proximity to `Zee` as a possible reversal zone

## Practical summary

- `Zee1` = signal price
- `Zee` = live edge
- far from `Zee1` = out of range
- close to `Zee` = near the turn zone
- 1m + 5m agreement = cleaner direction
- 1m + 5m disagreement = mixed regime, use caution

## Higher timeframe usage

For 30m / 1h, use `DPSignal` more as a directional regime read than a fixed TP map.

Recommended rule:

- if the higher TF is strongly stretched and direction is aligned, treat it as a trend context
- do not force a tight TP just because `DPSignal` exists
- manage the exit with:
  - trailing stop, or
  - lower-TF flip to the opposite direction, or
  - lower-TF mixed / sideways read that says the move is spent

This is the practical difference:

- **lower TF** = timing
- **higher TF** = direction and trend continuation context

## Exit rule for higher TF

When 30m / 1h is already stretched:

- ignore fixed TP
- keep the trade while lower TF still agrees
- exit when lower TF turns `LONG` against a short, or becomes `SIDEWAY` / mixed enough to lose conviction

This works better than a small TP because it avoids capping a strong move too early.

## Notes

- The indicator is useful for both direction and volatility context.
- It is not a guaranteed prediction. It is a stretch/positioning read.
- For the best result, combine it with `Mrc`, `ZigZagTrend`, and the 1m/5m candle direction.
