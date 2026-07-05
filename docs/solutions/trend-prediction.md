# Indicator Strategy Summary

This note compresses the indicator work into one strategy-facing map.

Goal:

- use `mrc_200` for structure and regime
- use `zigzagtrend` for swing direction and reversal zones
- use `dpsignal` for stretch and live-edge context
- use `volume_50` later for movement start/end and exhaustion

The strategy should not treat these as separate signals. They should be read as a stack.

## Recommended read order

1. `mrc_200`
2. `zigzagtrend`
3. `dpsignal`
4. `volume_50` and `ntps`

That order answers:

- where the market is inside the channel
- which swing direction is active
- whether the move is stretched or still has room
- whether the move is ending or accelerating

## What each indicator contributes

### 1. MRC (`mrc_200`)

Use MRC as the structural backbone.

It tells you:

- where price sits in the band stack
- whether the channel is flat, controlled, or stretched
- where the decision zone is
- where support / resistance is likely to react
- which band can act as a trailing support / resistance shelf during a trend
- how to trade a band ladder with staged entries and exits

Best use:

- `MeanLine` angle = trend slope and decision pressure
- `CurrentCross` = current zone in the band stack
- `DownBig` / `DownInner` / `UpInner` / `UpBig` = structural support / resistance ladder
- repeated touches of `UpSmall` / `UpInner` / `UpOuter` = band strength and trend continuation

Strategy role:

- regime filter
- entry zone selection
- target zone selection
- trailing entry / staged exit ladder

### 2. ZigZagTrend (`zigzagtrend`)

Use ZigZag for swing structure and break levels.

It tells you:

- the active swing direction
- the current breakout / reversal zone
- the nearest structural support / resistance prices

Best use:

- `Direction` = pattern state
- `EndDirection` = the direction to follow most of the time
- `BreakPrice` = reaction / reversal / continuation trigger
- `BuyPrice` / `SellPrice` = longer structural support / resistance

Strategy role:

- direction confirmation
- order placement near structural levels
- break-fail counting

### 3. DPSignal (`dpsignal`)

Use DP for stretch and live-edge context.

It tells you:

- how far the move has already run
- where the live edge is
- whether price is close to the turn zone

Best use:

- `Zee1` = anchor price
- `Zee` = live edge / turning area
- `Direction` = short-term bias

Strategy role:

- confirmation that a move still has room
- warning that a move is late
- out-of-range vs in-range classification

### 4. Volume (`volume_50`)

Volume is a future filter, but the strategy should already reserve space for it.

Planned use:

- big volume crossing `MeanLine` = possible continuation
- big volume near `UpBig` / `DownBig` = extreme / exhaustion / fast scalp zone
- volume spike after a long trend = possible end of movement

Strategy role:

- movement start / end detection
- exhaustion detection
- spike continuation vs reversal discrimination

### 5. NTPS

NTPS is the activity / speed layer.

Use it to tell whether the current move is alive, dead, or noisy.

Strategy role:

- momentum confirmation
- spike vs chop discrimination

## Combined structure

Think of the strategy in layers:

### Layer 1: Structure

`mrc_200`

- where is price in the channel?
- is the channel flat, controlled, or stretched?

### Layer 2: Swing

`zigzagtrend`

- which swing direction is active?
- where is the break / reaction level?

### Layer 3: Stretch

`dpsignal`

- is price still close to its anchor?
- is the move in range or already extended?

### Layer 4: Exhaustion / acceleration

`volume_50` + `NTPS`

- is movement starting or ending?
- is this a spike or a controlled move?

### Layer 5: Macro flow

This layer is reserved for later expansion.

## Long-first strategy rules

These are the long-side rules. Mirror them for short.

### 1. Decision zone

If MRC `MeanLine` angle is near zero and price sits near the mean:

- do not force a trade
- wait for high-TF agreement or a break
- use ZigZag / DP for confirmation, not prediction

### 2. Controlled trend

If MeanLine angle is gently rising and ZigZag `EndDirection=LONG`:

- prefer long continuation
- buy pullbacks, not spikes
- keep TP light if DP is already far from `Zee1`

### 3. Extended trend

If MRC is steep, DP is far from `Zee1`, and ZigZag is still long:

- trend may continue, but it is late
- prefer trailing stop
- avoid chasing into `UpBig`

### 4. Reaction zone

If price reaches `BreakPrice` or `SellPrice`:

- expect either continuation or rejection
- count repeated failures to break
- use that failure count as reversal pressure

### 5. MRC ladder trade

If price keeps touching a band without breaking it for many candles:

- treat that band as a live support / resistance shelf
- enter on the side of the attempted break
- trail behind the shelf if price keeps respecting it
- scale out into the next outer band

Example:

- if price bounces between `UpInner` and `UpOuter`, enter near `UpInner`
- exit / reduce near `UpOuter`
- if `UpSmall` holds for many candles and price cannot break it, use `UpSmall` as a trailing entry zone for long continuation

This is how the ladder should be used in the live strategy:

- enter near the inner shelf when the market keeps rejecting the outer shelf
- trail the position behind the shelf that keeps holding
- reduce or take profit into the next outer shelf
