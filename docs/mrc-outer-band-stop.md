# MRC outer-band stop experiment

## Rule

Trending longs start with `down_big` as their structural stop. Trending shorts
use `up_big`. A trailing MRC level can tighten the stop only after it passes
the filled entry. If the market routes to Sideways, the range target may
change, while the Trending stop remains active and keeps trailing.

This changes stop management; it does not add a `slow uptrend` market-regime
label. A move through the outer band is treated as a break on an intrabar
touch. The strategy does not wait for a candle close to confirm it.

## CAP validation

Compared local run #308 (module build revision 4, pre-release strategy version
24) with run #310 (revision 7, pre-release strategy version 26). Both used CAP,
long direction, margin 50, leverage 20, and the same three-day window:
2026-10-24 22:53 through 2026-10-27 22:53 Bangkok time.

| Measure | Run #308 | Run #310 |
|---|---:|---:|
| Completed positions | 118 | 111 |
| Net winning / losing positions | 58 / 60 | 77 / 34 |
| Stop-labeled exits | 64 | 25 |
| Stop-labeled exits with a net loss | 58 | 8 |
| Gross PnL | 70.115 | 191.860 |
| Fees | 35.430 | 34.337 |
| Net PnL | 34.685 | 157.524 |
| Worst single-position net loss | -36.487 | -66.213 |
| Mean max adverse move on losing positions | 0.95% | 1.72% |
| Largest max adverse move on a losing position | 3.86% | 8.88% |

The outer-band rule reduced losing stop exits from 58 to 8 in this sample, but
it allowed deeper adverse movement. Closed-position equity drawdown was
181.552 in run #308 and 148.772 in run #310. These figures describe this one
window and do not establish performance outside it.

For the first matched entry, run #308 stopped at the mean line after 6 minutes
at 0.074534. Run #310 kept the position open and exited at its take-profit
after 41 minutes at 0.075620.

## Remaining behavior

Six losing stops in run #310 came from Trending entries and filled at their
recorded `down_big` level. Two of those bars had a low below `down_big` but
closed back above it; the intrabar stop still exited. Waiting for a close below
the band would be a different rule and has not been tested here.

The tested versions are local pre-release versions. No live strategy or bot
configuration was changed.
