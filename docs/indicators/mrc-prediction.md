# MRC Prediction Read

Use `mrc_200` as the structure indicator for direction, decision zones, and reversal risk.
This note focuses on the long case first. For short, mirror the logic.

## What to read

Use three pieces together:

- `MeanLine` angle
- current position inside the MRC band stack
- `DPSignal` `Zee` / `Zee1` to see where price was and where it is now

## MeanLine angle

The angle of `MeanLine` tells you whether the channel is drifting, flat, or expanding.

Practical reads:

- angle near `0` = decision zone
- angle gently rising = controlled uptrend
- angle rising too fast = stretched move / possible spike
- angle turning down after a fast up move = possible reversal or dump start

Do not use one absolute threshold for all markets. The angle should be judged against:

- the current MRC band width
- the current timeframe
- how fast price is moving inside the channel

## Position inside the MRC

Use the band location to understand where price is now:

- near `MeanLine` = balance / decision zone
- near `UpInner` / `UpSmall` = strong long pressure
- near `UpBig` = extreme / expansion / possible exhaustion
- near `DownInner` / `DownSmall` = weak / mean-reversion / support test
- near `DownBig` = deep stress / high reaction risk

The MRC bands can also act as a trailing support / resistance ladder:

- if price keeps touching `UpSmall` without breaking it, that shelf can support a long continuation entry
- if price bounces between `UpInner` and `UpOuter`, enter near `UpInner` and scale out near `UpOuter`
- repeated touches with no break are useful strength information, not just noise

In live trading, use the shelf behavior like this:

- enter on the inner band when the outer band keeps holding
- trail the entry behind the band that keeps being respected
- scale out as price reaches the next outer band
- treat ten candles of touches without a break as a stronger trend-read than a single touch

### Long-side reading

For long analysis:

- price near `MeanLine` with flat angle = unclear, wait or use higher TF confirmation
- price above `MeanLine` with gentle positive angle = healthy long trend
- price pushing toward `UpInner` / `UpBig` with steep angle = trend is strong, but chase risk rises
- price already far above `MeanLine` while angle is still rising fast = avoid buying the spike
- price hugging `UpSmall` / `UpInner` and refusing to break = a good place to trail entries and scale out into the next band

## Using `DPSignal` for context

`DPSignal` helps answer two questions:

- where we are now
- where we were before

Use:

- `Zee1` as the main signal anchor
- `Zee` as the live moving edge

### Long-side interpretation

- if price is close to `MeanLine` and `Zee1` is close to current price, the market is in a decision zone
- if price is above `MeanLine` and `Zee1` is still below or far behind, the trend has room and can keep grinding up
- if price is already far above `Zee1`, the move is stretched and the next leg is less attractive
- if price is near the upper MRC lines but `Zee` is also far above the anchor, expect slower continuation and watch for exhaustion

## Prediction rules

### 1. Angle near zero

This is the decision zone.

Meaning:

- the market may go sideways
- the next direction is unclear
- high timeframe context matters more than the local setup
- waiting is acceptable if no higher-TF confirmation exists

### 2. Angle growing gently

Example:

- `0.010%` to `0.012%`

Meaning:

- the move has structure
- the trend is controlled
- long is valid if price does not get far ahead of the signal
- do not buy a spike; buy continuation / pullback

This is the best long environment when the channel is still orderly.

It is also the best environment to use MRC bands as a ladder:

- enter on the inner shelf
- trail behind the next shelf
- reduce into the outer shelf

### 3. Angle moving too fast

If the angle accelerates too much relative to the MRC width, the move is likely becoming unstable.

Meaning:

- the market can still continue
- but it is now in spike territory
- the probability of a sharp reversal rises
- if the angle flips after a spike, expect fast dumping or a reset

## Short-side mirror

For shorts, reverse the logic:

- near zero angle = decision zone
- gently falling angle = controlled downtrend
- overly steep negative angle = spike / exhaustion risk
- after a fast short spike, angle turning up can mean a bounce or reversal

## Volume notes for later

Volume is not required for the first version, but it should be added later because it helps separate movement from exhaustion.

Planned volume interpretation:

- big volume crossing the `MeanLine` = move can continue
- big volume near `UpBig` / `DownBig` = extreme / reversal / fast scalp zone
- volume spike after a long trend = possible end of movement
- volume spike while price is still near `MeanLine` = breakout or regime change

## Practical summary

- `MeanLine` angle tells you whether the channel is flat, controlled, or stretched
- `MeanLine` location tells you whether you are in decision, trend, or extreme zone
- MRC bands can act as trailing support / resistance shelves
- `Zee1` tells you the prior signal anchor
- `Zee` tells you the live edge
- use the combination to judge whether the move still has room or is already late

## Live 5m examples

Date: 2026-07-05

| Symbol | MRC Dir | CurrentCross | Close vs MeanLine | DP Dir | Close vs Zee1 | Read |
|---|---|---:|---:|---|---:|---|
| ETH | LONG | -1 | -0.16% | SHORT | -2.48% | Slightly below mean, but still structurally long; DP says the move is stretched and cooling. |
| HYPE | SHORT | -1 | -0.38% | LONG | +0.59% | Below mean with short structure, but DP is still leaning up, so this is a mixed transition zone. |
| LAB | SHORT | 0 | +1.57% | LONG | +6.76% | Above mean, but DP is far ahead of the anchor, so the move is more extended and less fresh. |

### What these examples show

- ETH: structure is still long, but DP says the live push is late.
- HYPE: MRC and DP disagree, so expect chop or a decision break.
- LAB: price is above mean, but DP is already far from `Zee1`, which usually means the move has run and can slow down or reverse.

## Notes

- This is a long-first interpretation.
- For short, reverse the direction and mirror the bands.
- For best results, compare the MRC read against `DPSignal` and later add `volume_50` as an exhaustion / continuation filter.
