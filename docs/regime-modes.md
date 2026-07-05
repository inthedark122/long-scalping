# Regime Modes

The module always produces exactly one market state:

- `Long`: `5m` MRC structure, DP, and ZigZag all align bullish.
- `Short`: `5m` MRC structure, DP, and ZigZag all align bearish.
- `Sideways`: the `5m` stack is mixed, flat, or blocked.

## Confirmation
- Primary regime comes from `5m`.
- If `5m` is ambiguous, `15m` can confirm `Long` or `Short`.
- `1h` never opens a trade by itself. It only blocks a conflicting lower-timeframe read.

## Entry policy
- `Long` can open a long only while flat.
- `Short` can open a short only while flat.
- `Sideways` opens nothing.

## Exit policy
1. Hard stop / risk exit
2. TP after a favorable move pulls back by `tp_pullback_pct`
3. Regime invalidation exit if the thesis is gone and TP did not resolve first
