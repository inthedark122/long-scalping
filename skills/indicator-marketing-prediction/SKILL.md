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

## Data source

Read indicator data directly from the API URL pattern the user provided.

Required format:

```text
https://okx.tg.trading.sinvoid.me/api/indicators?symbol_key=EXCHANGE%23SYMBOL%23INST_TYPE&timeframe=...&indicator_name=dpsignal&limit=1
```

Rules:

- use full `symbol_key`
- URL-encode `#` as `%23`
- use `indicator_name=dpsignal`
- use `timeframe=60` for 1m
- use `timeframe=300` for 5m
- use `timeframe=1800` for 30m
- use `timeframe=3600` for 1h

## Reading rules

From `DPSignal`:

- `Zee1` = signal anchor / base for distance
- `Zee` = live edge / turning area
- `Direction` = current bias

Interpretation:

- far from `Zee1` = stretched / out of range
- close to `Zee` = near the turn zone
- 1m + 5m agreement = cleaner timing
- 30m + 1h = higher-timeframe direction context

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

- `Direction`
- `Entry`
- `TP`
- `Prediction`
- `Price prediction`
- `How to trade`

## Output rules

- If price is stretched, say fixed TP is weak or should be ignored.
- If lower TF supports the move, describe continuation or retest probability.
- If higher TF is stretched, prefer trailing stop or exit on lower-TF flip to `LONG` or mixed/sideways.
- Always include a concrete likely price area, range, or retest zone when possible.
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
