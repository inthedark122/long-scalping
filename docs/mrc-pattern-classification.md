# MRC movement pattern classification

This document defines the five MRC movement classifications used by research.
They describe local, reusable behavior around MRC lines. They are not a
complete historical crossing path. The pattern engine runs every matcher on
each observation, keeps overlapping matches, and evaluates each match as an
independent candidate.

The live and research architecture has three separate stages:

1. `classify_all` detects every currently matching pattern;
2. the predictor builds one candidate and score per matched pattern;
3. the selector keeps the best pattern active until a challenger beats it by
   the switch margin.

`REVERSAL` is a transition on a staircase candidate, not a separate pattern
or performance ledger.

Each pattern has its own performance ledger. A ledger starts with movement
evidence, then learns from net realized trade PnL using an EMA. Research runs
one virtual account per pattern in parallel, while live execution trades only
the currently selected candidate. This prevents a profitable staircase from
being diluted by results from a different pattern and makes pattern selection
adaptive over time.

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

Every plan still requires a directionally valid target and stop. The general
reward-versus-risk floor is retained for barrier-driven patterns, but adjacent
`STAIRS_UP` and `STAIRS_DOWN` legs may
continue when their next MRC target is valid. This keeps an MRC stop
from suppressing the stair sequence itself.

An observation records the local movement that just finished:

- source and destination MRC line;
- movement direction;
- elapsed time;
- favorable and adverse percentage outside the source line;
- furthest line reached before returning to the source line;
- whether the source line was revisited;
- price drift and MRC range movement;
- later, the filled trade result and net PnL.

The same movement can develop through several classifications. A staircase can
overlap with a pullback or a rejected next line can instead produce
`FAST_BREAK_RETURN`. These are parallel candidates, not a hard-coded precedence
chain. The score combines the
pattern's learned route value with realized net PnL; a small switch margin
prevents noisy one-tick oscillation between patterns.

The direction is taken from that classification. The exact outcome counts are
supporting evidence; they are not a percentage or the trade win rate.

## 1. `STAIRS_UP`

### Meaning

Price forms an upward zigzag. The upward legs are separated by pullbacks, and
each new upward leg reaches farther than the previous one. A typical path is:

```text
2 -> 3 -> 2 -> 3 -> 2 -> 3 -> 4 -> 3 -> 4 -> 5
```

### Detection

- at least two upward legs exist;
- upward legs are separated by downward pullbacks;
- the newest upward leg reaches above the previous upward leg;
- a pullback after a successful leg keeps the staircase active for a possible
  support-assisted entry;
- an attempted upward leg that fails to exceed the previous peak stalls the
  current staircase lifecycle but does not finish the structure;
- a downward leg that breaks the protected swing low invalidates the
  staircase and requires two new upward legs before it can trade again.

There is no fixed full-path length. The pattern becomes stronger as additional
local steps are observed. A missed extreme is informational; the protected
swing is the structural finish condition.

### Prediction

Predict the next unbroken upper MRC line. The learned target is the next
destination; the actual trade target and stop must be calculated from the
current MRC lines at execution time.

## 2. `STAIRS_DOWN`

### Meaning

The mirror of `STAIRS_UP`. Price forms a downward zigzag with progressively
deeper downward legs:

```text
6 -> 5 -> 6 -> 5 -> 6 -> 5 -> 4 -> 5 -> 4
```

### Detection

- at least two downward legs exist;
- downward legs are separated by upward pullbacks;
- the newest downward leg reaches below the previous downward leg;
- an attempted downward leg that fails to exceed the previous trough stalls
  the current staircase lifecycle but does not finish the structure;
- an upward leg that breaks the protected swing high invalidates the staircase
  and requires two new downward legs before it can trade again.

### Prediction

Predict the next unbroken lower MRC line and update the expected movement after
each new step.

### Staircase lifecycle

The staircase detector reports a lifecycle transition with every staircase
candidate:

- `progressing`: a directional leg reached a new extreme;
- `pullback`: the opposing leg remains above the protected swing low for
  `STAIRS_UP`, or below the protected swing high for `STAIRS_DOWN`;
- `stalled`: a continuation attempt did not update the extreme. This remains
  directional information but is not a fresh trade entry;
- `invalidated`: the opposing leg broke the protected swing. The candidate is
  retained for diagnostics, but it is non-tradeable until two new directional
  legs rebuild a staircase segment.

A missed higher high or lower low is therefore not, by itself, a reversal.
The protected swing is the structural finish condition.

### Staircase transition: `REVERSAL`

The staircase remains useful as a continuation route until one of two warning
conditions appears:

- the favorable extreme reaches the outer/big MRC line, where the next
  retracement can span a much larger part of the range;
- the favorable high or low stops updating, or the expected next line is not
  reached during a continuation attempt.

These conditions produce a `REVERSAL` transition alongside the normal
staircase route. `REVERSAL` is not a third pattern: the candidate keeps its
underlying `STAIRS_UP` or `STAIRS_DOWN` identity and is attributed to that
pattern's performance ledger. A potential transition is emitted on the first
counter-leg after the warning. The opposite route becomes tradeable after
price crosses the staircase's protected swing; that transition is marked
confirmed, with the crossed protected line as the confirmation barrier and the
next MRC line as the first target. The normal staircase route is not filtered
just because transition evidence exists.

## 3. `FAST_BREAK_RETURN`

### Meaning

Price crosses one adjacent line and quickly returns to the exact source line:

```text
A -> adjacent B -> A
```

This makes `B` a candidate resistance/support line, but it does not decide the
trade direction by itself. A return to `A` can still be followed by a reclaim
of `B`.

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

The predictor searches historical instances with the exact same source line
`A`, crossed line `B`, and original break direction. It then labels the next
movement as one of:

- `rejection`: movement away from `B`;
- `continuation`: reclaim toward and through `B`;
- `neutral`: no directional movement.

The crossed line `B` is the actual support/resistance barrier in the analysis.
The strategy follows only a strict count majority of the exact outcomes. A
tie, neutral majority, or absence of an exact historical outcome produces no
directional prediction. Therefore `A -> B -> A` is not an automatic
counter-trend entry.

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

## Learning and outcome labels

The classifier and the outcome ledger are separate:

- the classifier describes what happened;
- the daily history stores how often it happened and what followed;
- the virtual/live trade result later labels whether a trade based on that
  observation won or lost;
- a target hit advances the continuation leg to the next unbroken MRC line,
  allowing the selected staircase pattern to keep trading the trend;
- staircase pullback lifecycle data remains analysis evidence and never takes
  entry authority.

Movement success is not trade win rate. A movement can reach a favorable price
but still produce a losing trade because of entry fill, stop placement, fees,
or the order of intrabar barriers.

## Class-specific line sequence for historical matching

Each `MrcPatternSignal` exports `line_sequence`, copied to the crossing
prediction and research event as `pattern_sequence`. Values are MRC line
indices from 0 (`down_big`) to 8 (`up_big`). The sequence is a matching key
within one class; it does not replace the classifier's label, candidate score,
or trade outcome.

The sequence follows the class's structure:

- staircases use the line path supporting the staircase classification;
- `FAST_BREAK_RETURN` uses source, crossed, and returned-to-source lines;
- cycle classes use the alternating lower/upper extreme visits;
- a staircase `REVERSAL` transition retains the underlying staircase
  sequence, because it is a transition on that pattern rather than a new class.

The real-kline research matcher first requires the same `pattern_kind` and
`pattern_direction`, then ranks earlier events by normalized dynamic-time-
warping distance between these line-index sequences. Ties use sequence-length
difference, elapsed-duration difference, then recency. The displayed top-five
examples have non-overlapping pattern-plus-outcome windows, and their full
four-hour outcomes end before the query pattern begins. This makes the chart's
best-match selection reproducible from the classifier output while keeping
its small-sample direction vote explicitly descriptive. The matcher and chart
are research-only; adding `line_sequence` does not change classification or
trading behavior.

The research exporter groups virtual predictions by the stable classification
name. The full crossing-event history remains diagnostic data. `pattern_sequence`
is the shorter, class-specific MRC path used for approximate DTW matching; the
matcher does not require an exact sequence equality.

The external history format is documented in
[`data/mrc-pattern-history/README.md`](../data/mrc-pattern-history/README.md).
The Rust vocabulary is in
[`mrc_pattern_classification.rs`](../module/src/detection/mrc_pattern_classification.rs).

For a real-kline best-match comparison and the broader research questions,
see the [MRC movement pattern-match solution](mrc-movement-heatmap-pattern-solution.md).
