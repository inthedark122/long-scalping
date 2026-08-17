# MRC movement pattern classification

This document defines the seven MRC movement classifications used by research.
They describe local, reusable behavior around MRC lines. They are not a
complete historical crossing path. The active MRC crossing strategy uses the
matched classification as its direction prediction and opens a trade only when
the pattern has supporting movement evidence and valid MRC target/stop levels.

The line order is fixed:

```text
0 down_big
1 down_outer
2 down_small
3 down_inner
4 mean
5 up_inner
6 up_small
7 up_outer
8 up_big
```

An observation records the local movement that just finished:

- source and destination MRC line;
- movement direction;
- elapsed time;
- favorable and adverse percentage outside the source line;
- furthest line reached before returning to the source line;
- whether the source line was revisited;
- price drift and MRC range movement;
- later, the filled trade result and net PnL.

The same movement can develop through several classifications. For example,
an `OUTWARD_CONTINUATION` can be followed by `STAIRS_UP`, while a rejected
next line can instead produce `FAST_BREAK_RETURN`. The live decision uses the
most specific currently recognized classification, with this precedence:

```text
FAST_BREAK_RETURN
PULLBACK_TO_INNER
STAIRS_UP / STAIRS_DOWN
RANGE_CYCLE_SIDEWAY / TRENDING_CYCLE
OUTWARD_CONTINUATION
```

The direction is taken from that classification. A route probability is only
supporting evidence for the selected MRC destination; it is not used to flip
the direction or to trade an unclassified event.

## 1. `STAIRS_UP`

### Meaning

Price crosses successive MRC lines upward without a meaningful downward
interruption. A typical example is:

```text
down_inner -> mean -> up_inner -> up_small -> up_outer -> up_big
```

### Detection

- the line index keeps increasing across local transitions;
- the movement does not return through a previously crossed line before the
  next step is confirmed;
- record the number of steps, total line span, elapsed time, favorable move,
  adverse move, and highest line reached.

There is no fixed full-path length. The pattern becomes stronger as additional
local steps are observed and ends when the sequence reverses, stalls, or
reaches the available outer line.

### Prediction

Predict the next unbroken upper MRC line. The learned target is the next
destination; the actual trade target and stop must be calculated from the
current MRC lines at execution time.

## 2. `STAIRS_DOWN`

### Meaning

The mirror of `STAIRS_UP`. Price crosses successive MRC lines downward:

```text
up_inner -> mean -> down_inner -> down_small -> down_outer -> down_big
```

### Detection

- the line index keeps decreasing across local transitions;
- the movement does not return through a previously crossed line before the
  next step is confirmed;
- record the number of steps, total line span, elapsed time, favorable move,
  adverse move, and lowest line reached.

### Prediction

Predict the next unbroken lower MRC line and update the expected movement after
each new step.

## 3. `FAST_BREAK_RETURN`

### Meaning

Price crosses one adjacent line and quickly returns to the exact source line:

```text
A -> adjacent B -> A
```

This means that `B` may be acting as resistance or support rather than as a
continuation level.

### Detection

Record:

- source line `A`;
- crossed adjacent line `B`;
- time from `A` to `B` and from `B` back to `A`;
- maximum percentage excursion beyond `A`;
- whether the return occurred before another line was reached.

The research reports currently contain a useful 5–10 minute bucket, but the
time interval remains a learned feature rather than a permanent strategy
constant.

### Prediction

After the return, treat `B` as a possible barrier. A failed upward break is
evidence for resistance; a failed downward break is evidence for support.

## 4. `RANGE_CYCLE_SIDEWAY`

### Meaning

Price repeatedly travels between lower and upper MRC zones while its net price
drift remains small compared with the total distance travelled.

The zones are currently described by:

```text
lower zone: down_inner or lower
upper zone: up_inner or higher
```

### Detection

- compress repeated visits into alternating lower/upper zone visits;
- measure the number of alternations and the time between them;
- measure total price range and net price drift from the beginning to the end;
- classify as sideways only when drift is small relative to total range.

This prevents every repeated crossing from being called sideways. A market can
cross both zones repeatedly while still drifting strongly in one direction.

### Prediction

Prefer the opposite edge of the current range as the next destination. A
lower-edge observation favors movement toward the upper edge; an upper-edge
observation favors movement toward the lower edge.

## 5. `TRENDING_CYCLE`

### Meaning

Price also alternates between lower and upper zones, but the entire range is
moving directionally. This is a cycle inside a trend, not a neutral range.

### Detection

Use the same alternating-zone observations as `RANGE_CYCLE_SIDEWAY`, but
classify the block as trending when net price drift is large compared with the
total travelled range.

### Prediction

Prefer the drift direction. Treat movement toward the opposite edge as a
pullback candidate rather than immediately reversing the strategy.

## 6. `PULLBACK_TO_INNER`

### Meaning

Price first moves outward and then crosses back toward an inner MRC line before
the expected continuation is confirmed:

```text
up_inner -> up_small -> up_inner
down_inner -> down_small -> down_inner
```

### Detection

Record the outward line, returned inner line, elapsed time, favorable excursion
of the outward leg, and adverse excursion during the return.

### Prediction

Reduce continuation confidence. The returned inner line can become a reversal
or re-entry area. A later outward break should be treated as a new observation,
not silently combined with the failed attempt.

## 7. `OUTWARD_CONTINUATION`

### Meaning

Price crosses the adjacent MRC line in the current movement direction and its
favorable excursion is stronger than its adverse excursion. This is the small
building block from which a staircase is learned.

### Detection

For every local transition, record:

- source line and adjacent destination line;
- direction;
- elapsed time;
- favorable and adverse percentage movement;
- movement success;
- return-to-source status.

Examples from the current reports include `up_inner -> up_small`,
`up_small -> up_outer`, `down_inner -> down_small`, and
`down_small -> down_outer`.

### Prediction

Use the next line as the immediate destination. Repeated successful outward
continuations increase the evidence for `STAIRS_UP` or `STAIRS_DOWN`.

## Learning and outcome labels

The classifier and the outcome ledger are separate:

- the classifier describes what happened;
- the daily history stores how often it happened and what followed;
- the virtual/live trade result later labels whether a trade based on that
  observation won or lost.

Movement success is not trade win rate. A movement can reach a favorable price
but still produce a losing trade because of entry fill, stop placement, fees,
or the order of intrabar barriers.

The research exporter groups virtual predictions by the stable classification
name. The detailed crossing sequence remains diagnostic data and is not used
as a rare, exact pattern key.

The external history format is documented in
[`data/mrc-pattern-history/README.md`](../data/mrc-pattern-history/README.md).
The Rust vocabulary is in
[`mrc_pattern_classification.rs`](../module/src/detection/mrc_pattern_classification.rs).
