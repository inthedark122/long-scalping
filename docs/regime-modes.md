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
- Regime mode does not directly define exit timing.
- The execution engine keeps the one-position invariant and closes conflicting dual positions if they ever appear.
- A held position is not reversed on the same tick.

## Risk policy
- The module keeps the one-position invariant and uses limit-only entries.
- Risk sizing is controlled only by `cap_init`.
- The internal deployable budget doubles on profit until the cap is reached, then grows with leverage-backed profit reinvestment.
