# Virtual stop-loss spike investigation plan

## Purpose

The current `MrcReclaim` exit uses a virtual stop-loss. When the observed
price crosses that stop, the active reduce-only limit exit is repriced to a
recovery target two ticks away. This has produced good results, but a fast
price spike can cross the virtual stop before the limit exit is filled.

This document is an investigation plan for Option A only: adding diagnostic
information to exit-limit order notes. It does not change exit logic, order
types, thresholds, or live behavior.

## Investigation questions

1. When a virtual stop is crossed, was there enough observable movement before
   the crossing to determine whether the existing exit-limit price was
   reachable?
2. Was the existing exit limit theoretically executable before the spike, and
   at what observed price?
3. Which observed events should be classified as normal virtual-SL crosses and
   which should be classified as dangerous spikes?
4. Can the available strategy and order data distinguish a possible exit from
   a confirmed exchange fill?

## Phase 0: define the baseline and event vocabulary

Record the current behavior before evaluating the telemetry:

- The strategy, side, entry price, active virtual SL, and stop-loss reason.
- The active exit-limit price before the virtual-SL crossing.
- The last observed price and timestamp before the virtual-SL crossing.
- The first observed price and timestamp at or beyond the virtual SL.
- The recovery limit price generated after the crossing and whether that order
  was filled, cancelled, replaced, or remained unfilled.
- Whether the position eventually closed through TP, stop-loss handling,
  another close path, or remained open.
- The confirmed fill price and filled quantity when order data is available.

Use these terms consistently:

- **Virtual-SL cross:** the strategy's observed price satisfies the existing
  long/short crossing rule.
- **Spike:** an adverse move that crosses the virtual SL within a short
  observed time/tick window. The window must be measured from data rather than
  chosen in advance.
- **Possible exit:** an analytical conclusion that the observed market path
  reached or passed an exit price before the virtual-SL cross. It is not proof
  that the exchange would have filled the order.
- **Possible exit price:** the best price that can be inferred from the
  available observed prices, rounded to the symbol tick size. If the data is
  insufficient, record `unknown` rather than inferring a fill.

The baseline report should include both long and short positions and should
separate ordinary virtual-SL crosses from suspected spikes.

## Option A: add investigation telemetry to exit-limit order notes

### Objective

Make each relevant exit-limit order traceable enough to reconstruct what the
strategy knew at order placement, at repricing, and at virtual-SL crossing.

The telemetry must separate observed market evidence from order state and
confirmed exchange execution. It must not overstate what can be concluded
from last price or mark price alone.

### Information to capture

The future note format should be compact, stable, and key/value based. It
should include, where available:

- `exit_kind` and `exit_mode`;
- `side`, `entry_price`, `virtual_sl`, and `sl_reason`;
- `active_tp_before` and `recovery_ticks`;
- `observed_price` and `observed_price_prev`;
- `observed_at_ms`, `previous_at_ms`, and elapsed time;
- adverse movement in ticks and price units;
- `spike_detected` and the detection reason;
- `sl_crossed` and `sl_overshoot_ticks`;
- `possible_exit` with values such as `yes`, `no`, or `unknown`;
- `possible_exit_price` and the source observation used to derive it;
- `exit_order_price` and order age if available;
- whether the order was a normal TP, a repriced recovery exit, or another
  reduce-only exit;
- order status, filled quantity, and actual fill price when available.

Keep these concepts separate in the note or in the joined analysis:

- `observed_path` — what the strategy's market observations showed;
- `order_state` — what order-status data showed;
- `actual_fill` — confirmed execution data.

If bid/ask, trade direction, or order-book data is available through the
bridge, record it separately from last/mark price. If it is not available,
explicitly label the result as last-price-only evidence.

### How to determine the analytical fields

For each virtual-SL event, compare the previous observed tick, current
observed tick, timestamps, active limit price, and the known order lifecycle.
Classify the event as follows:

- `possible_exit=yes` only when the observed path reached a price at which the
  active reduce-only limit could have executed before the crossing;
- `possible_exit=no` when the observed path shows that price was never
  reached;
- `possible_exit=unknown` when tick gaps, missing order status, unavailable
  bid/ask data, or insufficient timestamps prevent a reliable conclusion;
- `possible_exit_price` is the inferred observed price, not a guarantee of
  fill, and remains unknown when the preceding rule cannot be satisfied.

Do not infer an intratick path that is not supported by the data. If the
available observations do not establish whether the limit was reached, record
`unknown` instead of assuming a fill opportunity.

### Collection and review

1. Add the telemetry in a future instrumentation change without changing
   decisions, exit prices, or order types.
2. Collect a representative sample of virtual-SL crosses, including ordinary
   crosses, sharp spikes, and cases where the recovery limit filled.
3. Include unfilled, cancelled, replaced, and partially filled exits where the
   order lifecycle is available.
4. Join order notes with order-status/fill records and market timestamps.
5. Manually review representative samples to verify that the notes describe
   the actual state transition and that long/short signs are correct.
6. Check that `possible_exit=yes`, `no`, and `unknown` can each be explained
   from the captured fields.
7. Produce a baseline table containing:
   - virtual-SL crossing frequency;
   - normal-cross versus suspected-spike counts;
   - SL overshoot in price units and ticks;
   - possible-exit counts by classification;
   - recovery-limit fill rate;
   - realized exit-price distribution;
   - order-status completeness and unknown-evidence rate.

The instrumentation phase is successful only if every classification can be
explained from the captured fields. A high number of `unknown` events means
the data source must be improved before drawing conclusions.

The final analysis must not treat `possible_exit=yes` as proof that the
exchange would have filled the order. Confirmed fills must come from
order-status or fill records.

## Deliverables

1. Data-availability and timestamp audit.
2. Stable exit-note telemetry specification with example annotated events.
3. Baseline virtual-SL spike report.
4. Joined note/order-status sample with confirmed fills separated from inferred
   possible exits.

## Explicit non-goals

- No early-exit detection or implementation.
- No market-order fallback.
- No change to the virtual-SL or two-tick recovery behavior.
- No change to the continuously recalculated `VirtualTrailingExit` stop-loss.
