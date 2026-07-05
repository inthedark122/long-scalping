# Algo Suggestion: Projection Candle / FC Semantics

This note describes how to read the live `Projection` websocket feed when using the LAB fast-monitor logic.

## Core objects

### `candle`

`candle` is the small live micro-candle snapshot in the projection frame.

Use it as:

- immediate price movement context
- micro confirmation of a spike or pause
- a very short-horizon range read

It can still catch a big move even though it is small in time.

### `FC`

`FC` is the fast-candle group gathered while the stream is in `Fast` status.

Use it as:

- a fast-move state container
- a measure of whether the fast move is still developing
- a clue for whether the move is spiking, growing, or finishing

If `FC` is growing, the move is still alive.
If `FC` is large and `candle` becomes small, the fast move may be tiring.

### `klines`

`klines` are the current timeframe candle values in the live projection frame.

Use them as:

- the current TF candle state
- the live candle structure for that timeframe
- the near-term trend / range context

### `indicators`

`indicators` are the past / derived context from earlier candles.

Use them as:

- historical memory
- directional bias
- anchor / regime context

## Reading rules

### 1. Small candle after a big FC

If the fast move already expanded and the current candle gets small, that often means:

- the fast move is finishing
- momentum is pausing
- reversal or consolidation may be near

### 2. Big FC distance

If FC distance is large, the market has likely already moved sharply.

That can mean:

- spike
- strong continuation
- late move with exhaustion risk

### 3. Updating FC prices

If FC prices keep updating, the move is still building.

That usually means:

- continuation is still possible
- do not assume the move is finished yet

### 4. First `Normal` after `Fast`

The feed can still show FC information on the first `Normal` status after a `Fast` phase.

That should be read as:

- recent fast state still being tracked
- post-fast transition
- not a fresh new fast event

## Practical interpretation

- `candle` = micro entry / exit signal
- `FC` = fast-move state
- `klines` = current TF candle context
- `indicators` = history and structure

Suggested reading:

- `FC` active and growing + `candle` still expanding = move can continue
- `FC` large and `candle` tiny = fast move may be ending
- `FC` absent = avoid calling it a fast-move setup
- `klines` and `indicators` should be used together for trend context

## Strategy use

- use `candle` for immediate timing
- use `FC` for momentum tracking
- use `klines` for the current TF candle state
- use `indicators` for prior structure and direction memory

This is the interpretation used by the LAB fast-monitor docs and signal tracker.
