# Movement Cascade Research

This note records why the movement layer exists and how it is split now:

- completed movement is seeded from `Init` and refreshed only from indicator updates
- live movement is provisional and comes only from `SugInfo`
- realtime trading should consume the cached completed state plus the current `SugInfo` movement snapshot

## Why movement matters

The movement cascade is the 1m timing layer that sits under the higher-level regime and structure path.

We need it because:

- regime tells us direction context, but not when the current push is starting or already tired
- `sug_info.candle` gives a better live entry / exit read for scalping, but it still needs movement context
- the movement average gives us a dynamic distance anchor for continuation, reversal, and TP / SL
- static percentages are too blunt for fast market phases

The intended stack is:

1. regime / structure for direction context
2. SugInfo candle for default scalping execution
3. completed movement from indicator refresh for continuation / reversal timing
4. provisional SugInfo movement for the final live entry / exit price path

## Original research plan

The point of the research was to answer one question:

- can a 1m movement model tell us whether we are at the start of a move, in the middle, or already late?

The initial plan was to:

- seed the completed movement state from the last 10 closed 1m candles on `Init`
- refresh completed movement only from `ModuleEvent::Indicators`
- keep a rolling average of movement block size
- wait for at least 3 reverses before trusting the average
- treat a move above roughly `average * 1.3` as a reverse / maturity warning
- treat a small block before a larger extension as a continuation candidate
- use `sug_info.candle` as the provisional live movement source for scalping
- derive TP / SL from the movement average instead of fixed percentages

That plan is still the same. What changed is that we now have replay output to validate it.

## What we verified

The replay tool is now runnable as `scripts/movement_research.py`.

It seeds the 1m movement state from the first 10 candles, then replays the rest of the sample and reports:

- block direction
- phase: `start`, `building`, `mature`
- reverse-ready / safe-continuation conditions
- virtual entry / exit events
- win rate, average win, average loss, MFE, and MAE

Observed replay results:

- `ETH` with 60 candles
  - `trades=6`
  - `win_rate=50.0%`
  - `avg_win=0.099%`
  - `avg_loss=-0.071%`
- `SLX` with 60 candles
  - `trades=3`
  - `win_rate=33.3%`
  - `avg_win=1.234%`
  - `avg_loss=-0.700%`
- `LAB`
  - shorter runs worked
  - longer `200` candle fetch stalled on the remote API
  - the note here is not that LAB fails, only that it needs a longer wait or a more resilient fetch path

What the sample runs mean in practice:

- `average * 1.3` is a usable first reverse threshold for the movement state machine
- ETH is noisy, so the movement layer should be used as a timing filter, not as a standalone trade trigger
- SLX has wider swings, so a symbol-specific threshold set is likely better than one global constant
- LAB needs more observation time, so a short replay is not enough to lock the threshold

Best practice from the replay:

- use the movement layer to decide whether the push is starting, building, or already late
- use `sug_info.candle` for the actual scalping entry and exit price path
- use the movement average as a distance guide for TP / SL, not as a fixed static rule
- keep the higher-level regime logic as context and fallback only
- compare symbols before promoting one threshold to production
- do not treat a single 60-candle result as a universal truth

## Current usage

The movement logic should now be used like this:

- use `1m` for movement timing
- seed completed movement from the last 10 candles on `Init`
- refresh completed movement only from `ModuleEvent::Indicators`
- require at least 3 reverses before trusting the average
- treat a block above roughly `average * 1.3` as a reverse / maturity warning
- treat a smaller block before the larger push as a continuation candidate
- use `sug_info.candle` for the provisional live scalp entry / exit path
- keep the completed indicator-backed movement cached for realtime decisions
- keep structural regime logic as a fallback and higher-level context only
- use the replay tool to compare thresholds before changing production logic
- prefer symbol-specific threshold tuning when the replay output differs materially across tokens

## How to use the script

`scripts/movement_research.py` is the research entry point.

Useful modes:

- `--mode replay` to fetch closed 1m candles and simulate entries / exits
- `--mode live --ws-seconds 300` to trace the websocket for 5 minutes
- `--mode both` to replay first and then watch live
- `--symbol-key` can be repeated for multiple symbols
- `--symbols` accepts a comma-separated batch list
- `--csv results.csv` writes trade and summary rows

Example batch run:

```bash
python3 scripts/movement_research.py --mode replay --history 60 \
  --symbols OKX#ETH-USDT-SWAP#SWAP,OKX#SLX-USDT-SWAP#SWAP,OKX#LAB-USDT-SWAP#SWAP
```

## Next research

What remains open is mostly threshold tuning:

- compare `1.2x`, `1.3x`, and `1.5x` reverse thresholds
- compare continuation entry quality across symbols
- rerun LAB with a longer wait or a more resilient fetch path
- compare how often SugInfo entry / exit beats the structural fallback on live trading

The script should stay simple. The point is to expose movement behavior and live scalping behavior, not to hide it behind a heavy backtest framework.
