---
name: indicator-marketing-prediction
description: Turn LTE indicator reads into marketing-style trading summaries with headline, range, stretch, reversal potential, trend strength, price prediction, entry, TP, and how-to-trade guidance. Use when the user asks for ETH/HYPE/other token analysis, a short resume, prediction, or a marketing-style indicator read. Fetch indicators from the API URL with full symbol_key and explain the current direction in tradeable terms.
---

# Indicator Marketing Prediction

Use this skill when the user wants a concise, trade-facing summary of an LTE indicator read.

## Core rule

Never stop at "wait" alone. Always translate the indicator into a usable market view:

- direction
- price prediction
- entry
- TP
- how to trade

If the setup is stretched or mixed, keep the fixed TP light or optional, but still provide a likely next price area or range.

## Multi-step analysis

Do not build the prediction from one indicator alone.

Use the docs as a stack:

1. Read the indicator rules in [`docs/indicators/README.md`](/home/dev/strategies/long-scalping/docs/indicators/README.md)
2. Apply the specific indicator notes:
   - [`docs/indicators/mrc-prediction.md`](/home/dev/strategies/long-scalping/docs/indicators/mrc-prediction.md)
   - [`docs/indicators/zigzag.md`](/home/dev/strategies/long-scalping/docs/indicators/zigzag.md)
3. Combine them with the strategy summary in [`docs/solutions/trend-prediction.md`](/home/dev/strategies/long-scalping/docs/solutions/trend-prediction.md)
4. Turn the stack into one tradeable conclusion:
   - structure
   - swing direction
   - local structural bounds
   - exhaustion / acceleration
   - base TF
   - local TP versus runner extension

The analysis should answer:

- what is the backbone structure
- what is the active swing
- whether the move is stretched
- whether the current TP is local or a higher-TF runner target
- which TF is the base for the prediction

When the docs conflict:

- prefer the base TF setup that is currently most tradeable
- use the higher TF as runner context, not as the main scalp target
- explain the conflict in the output instead of hiding it

## TP hierarchy

Always explain why the TP is small or large.

Use this order:

1. `TP1` = nearest local level that is currently tradeable.
1. `TP2` = next nearby extension if the first wall breaks.
1. `Higher-TF extension` = only if 30m / 1h structure still supports continuation.

Choose the smaller TP when:

- price is close to the local wall
- higher TF is stretched or mixed
- the move is late-stage and likely to pause
- the nearest level is enough for a fast scalp

Choose the larger TP when:

- 1m and 5m agree cleanly
- 30m / 1h still has room
- local resistance is thin
- momentum is expanding, not fading

When you choose a local TP instead of a high-TF target, say why:

- "local TP because the next 1m/5m wall is near"
- "higher-TF extension exists, but it is only a runner target"
- "use trailing stop after TP1 because the higher TF still has room"

## Base timeframe rule

Every prediction must say which timeframe is the base for the call.

Default to the highest TF available as the main base for the conclusion.

Use this rule:

- highest TF = regime, main direction, and runner-target context
- `30m` = secondary structure context
- `5m` and `1m` = entry timing, local reversal risk, and TP1 / TP2

When writing the prediction, state:

- `base TF` = the timeframe that drives the main conclusion
- `support TFs` = the timeframes used to confirm or reject it
- why the highest TF is the base:
  - it sets the regime and the main directional target
  - it tells you if a local move is only a pullback inside a bigger trend
  - it defines the runner context for the bigger target
- why lower TFs are secondary in this setup:
  - they are timing and local spike risk inside the higher-TF move
  - they can show a local counter-move without changing the larger structure
  - they are useful for entry, but not for the main conclusion

Practical wording examples:

- "Base TF: 1h. 5m is only timing; 1m is local noise."
- "Base TF: 1h short, 5m bounce is local, so keep TP local and use 1h as runner context."
- "Base TF: 30m extension, but use 5m for actual entry because lower TF is just timing."

## Data source

Read indicator data directly from the API URL pattern the user provided.

Required format:

```text
https://okx.tg.trading.sinvoid.me/api/indicators?symbol_key=EXCHANGE%23SYMBOL%23INST_TYPE&timeframe=...&indicator_name=smc&limit=1
```

Rules:

- use full `symbol_key`
- URL-encode `#` as `%23`
- use `indicator_name=smc` when requesting SMC specifically
- use `timeframe=60` for 1m
- use `timeframe=300` for 5m
- use `timeframe=1800` for 30m
- use `timeframe=3600` for 1h

## Reading rules

From `SMC`:

- `Direction` = current structural bias
- `DirectionSwing` = broader swing bias
- `LowPrice` / `HighPrice` = local structural bounds
- `BreakPrice` / `SBP` = break and continuation levels
- `OBLow` / `OBHigh` = order-block boundaries
- `FP` = current structural focus price

Interpretation:

- agreement between `Direction` and `DirectionSwing` = cleaner structure
- price near `LowPrice` or `OBLow` = support/reaction area
- price near `HighPrice` or `OBHigh` = resistance/reaction area
- break beyond `BreakPrice` or `SBP` = continuation must be confirmed with ZigZag and MRC
- 1m + 5m agreement is cleaner timing
- 30m + 1h is higher-timeframe direction context

## Response format

Use this structure:

### 1. Headline

One sentence with the current bias and whether the move is stretched, mixed, or clean.

### 2. Scorecard

- `Range`
- `Stretch`
- `Reversal-potential`
- `Trend-strength`

### 3. Trade view

- `Base TF`
- `Direction`
- `Entry`
- `TP1`
- `TP2` or `extension` when relevant
- `TP`
- `Prediction`
- `Price prediction`
- `How to trade`

## Output rules

- If price is stretched, say fixed TP is weak or should be ignored.
- If lower TF supports the move, describe continuation or retest probability.
- If higher TF is stretched, prefer trailing stop or exit on lower-TF flip to `LONG` or mixed/sideways.
- Always include a concrete likely price area, range, or retest zone when possible.
- If the user asks why TP is small, explain the nearest structural wall, then mention the higher-TF extension as a secondary target.
- If the user asks for "high information", include a short hierarchy like `TP1`, `TP2`, and `runner extension`.
- Always say which TF the prediction is based on and why the other TFs are less accurate for this specific call.
- If the user asks for "marketing" or "headline" style, make the result concise and actionable, not technical.

## Practical style

Good outputs read like:

- "Short pressure is active, but ETH is late-stage stretched."
- "Likely downside retest first, then trail the move."
- "Best entry is on rejection, not chase."

Avoid outputs that only say:

- "wait"
- "mixed"
- "no signal"

If the setup is unclear, still give the most likely next move and the area that matters.

## Default card style

When the user asks for a marketing analysis, use this compact card format by default:

- headline
- range
- stretch
- reversal-potential
- trend-strength
- direction
- entry
- TP
- prediction
- price prediction
- how to trade

Keep it short, concrete, and tradeable. Prefer exact nearby levels over vague language. If the setup is stretched, mixed, or late, say that directly and shift emphasis from fixed TP to trailing stop or lower-TF confirmation.

## Explanation rule

When the user asks "why is TP so small?" or "why not the higher level?":

- identify the nearest structural resistance/support
- identify the next extension level
- say whether the extension is a valid runner target or stale high-TF target
- explain if the current setup is best for scalping, not swing continuation

When the user asks "why is the lower TF / higher TF less accurate?":

- explain the role separation:
  - lower TF = timing and local spike risk inside the higher-TF move
  - highest TF = regime and the main directional target
  - say which TF is the base and which ones are only confirmation

Example language:

- "TP1 is local because 1m/5m hit the first wall."
- "0.2324 is a higher-TF extension, but it is only the runner target."
- "Use local TP plus trailing stop because the higher timeframe is still stretched."
