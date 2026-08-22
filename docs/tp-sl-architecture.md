# TP/SL and active MRC line plan

This is the design contract for MRC staircase TP/SL calculation.

## 1. Data sources

| Source | Responsibility |
|---|---|
| Current `1m` indicator snapshot | MRC lines, MRC signal, candle, volume and other indicator values. The core MRC plan always uses the current 1m structure. |
| `sug_info` | Event time, NTPS, fast-time data, cascade timing and other non-indicator/live context. |
| Cascade state | Direction, movement phase, elapsed movement time, speed, price range/potential and adverse movement. |
| Historical MRC movement records | Per-route favorable %, adverse %, duration and sample count. |
| Execution state | Actual filled entry and current position price/quantity. |

The cascade is updated from chronological 1m movement data using the time from
`sug_info`. It measures current price/time interest in the movement. It is
context for the MRC prediction; it does not independently choose the trade
direction.

## 2. Active MRC line plan

The strategy builds a plan for all nine current 1m MRC lines, not only for one
selected target line. Each line plan contains:

- line index and current line price;
- route role: current, forward route, planned target, covered, beyond, or
  opposite direction;
- direction-adjusted movement percentage from the filled entry;
- absolute distance percentage from the filled entry;
- structural stop distance percentage;
- historical duration and sample count for that route;
- historical favorable and adverse movement percentages for that route;
- cascade-derived duration from current movement speed;
- selected approximate duration and its source;
- NTPS, fast-time, cascade direction, phase, speed, potential and adverse
  movement.

For direction `d` (`+1` long, `-1` short), filled entry `E`, and line price
`L_i`:

```text
line_move_pct_i = d × (L_i - E) / E × 100
line_distance_pct_i = abs((L_i - E) / E × 100)
```

For a valid cascade speed `S` in percent per minute:

```text
cascade_time_i = line_distance_pct_i / S × 60,000 ms
```

The approximate time for a line is selected from the available sources:

1. cascade speed when valid and active;
2. exact historical duration for that source/line route;
3. scaled historical duration from a nearby route;
4. unavailable when neither source exists.

The nine plans form the active route map. A staircase can progress through
several forward lines; each line has its own movement percentage and time
estimate. The selected target is a model decision, not a reason to discard the
other line plans.

## 3. Imperative TP model

TP selection is a stateful procedure, not a closed-form mathematical model.
The formulas above only describe line distances, time estimates, and the final
conversion of a selected movement into an order price.

The live state keeps:

- filled entry and position direction;
- the current nine-line plan;
- the current route line and staircase state;
- the best reachable profit candidate found on the last update;
- candidate ETA, favorable/adverse movement, and cascade interest;
- the currently published TP.

## 4. Maximum-profit procedure

Run this procedure on entry and on every new valid 1m indicator / `sug_info`
update:

1. Refresh the current 1m MRC and indicator snapshot.
2. Update cascade state from the chronological 1m movement and `sug_info`
   time. Record its direction, phase, speed, elapsed time, potential price
   movement and adverse movement.
3. Build the complete plan for all nine current MRC lines.
4. Walk through every forward line in staircase order. For each line:
   - estimate the favorable movement around that line from exact route history;
     otherwise use current staircase/prediction movement evidence;
   - estimate the time to the line from cascade speed, otherwise historical
     exact/scaled duration;
   - check that the direction, staircase lifecycle, cascade phase, price
     potential, adverse movement and stop room support the route;
   - create a candidate only when the line is still reachable.
5. Continue scanning while the route remains valid. Do not stop at the first
   valid line.
6. Select the candidate with the maximum reachable profit in the position
   direction. Use the shortest ETA and strongest current cascade interest as
   tie-breakers.
7. If a candidate exists, publish its predicted exit as the primary TP. The
   MRC line supplies the route location; the movement prediction supplies the
   approximate exit around that line.
8. If no candidate exists, keep the last valid TP while the route is merely
   waiting. If the movement is stalled or invalidated, execute the separate
   stalled/invalidation policy.
9. When the selected candidate changes, amend the same reduce-only TP order.

The final price conversion is only the last step:

```text
long:  TP = filled_entry × (1 + selected_move_pct / 100)
short: TP = filled_entry × (1 - selected_move_pct / 100)
```

The important operation is selecting `selected_move_pct` by scanning and
comparing the reachable route candidates. A fixed percentage, the first valid
line, or the current MRC line price is not a TP strategy.

Example for a long filled at `100`:

```text
line 5: predicted exit around 101.4, ETA 90s, interest strong
line 6: predicted exit around 102.8, ETA 210s, interest adequate
line 7: predicted exit around 104.1, ETA 360s, interest insufficient
```

The procedure rejects line 7, compares lines 5 and 6, selects line 6 as the
maximum currently reachable profit, and publishes approximately `102.8` as
the TP. If cascade interest later supports line 7, the procedure selects line
7 and amends the same TP order. If line 6 also loses support, the procedure
reselects the best remaining candidate or enters the stall policy.

## 5. Stop-loss calculation

SL is calculated separately from TP using the current 1m MRC structure:

1. start from the MRC line that generated the setup;
2. walk to the first valid adverse line beyond the filled entry;
3. use that line as the protective stop;
4. recalculate when the ordered 1m MRC structure changes;
5. move SL only protectively: up for long positions and down for short
   positions;
6. apply direction and minimum-distance validation.

The source line is not a valid stop when it is already on the wrong side of
the fill. Cascade time/price interest can classify continuation or stall, but
it does not turn the cascade into an independent SL price.

## 6. Runtime states

### Primary movement

Use the active nine-line plan, prediction-based TP and structural 1m MRC SL.
Track filled-entry age, favorable extreme, cascade activity and route progress.

### Stall or invalidation

If expected time passes without favorable progress, or the cascade/staircase
loses continuation interest, mark the route stalled or invalidated. Apply the
defined partial/fallback exit policy; do not extend TP automatically.

## 7. Non-negotiable invariants

- Core indicator input is the current 1m snapshot.
- `sug_info` supplies time and non-indicator/live context used by cascade and
  activity measurement.
- All nine MRC line plans remain available for route and prediction decisions.
- Every forward MRC line has movement-% and approximate-time information when
  the required data exists.
- TP selection scans all reachable forward lines and chooses the maximum
  currently reachable profit.
- TP uses filled entry plus predicted movement percentage.
- MRC line price alone is never the TP algorithm.
- SL uses current 1m MRC structure and only moves protectively.
- A TP reprice updates the existing marked reduce-only order.
- A stored percentage that never changes is not dynamic behavior.

## 8. Required tests

Tests must prove that:

1. the plan is built from current 1m MRC lines;
2. `sug_info` time changes cascade elapsed time and speed context;
3. all nine line plans contain movement and time fields;
4. historical and cascade time sources are selected correctly;
5. the model scans all valid forward lines instead of stopping at the first;
6. the maximum reachable-profit candidate is selected;
7. changed movement/cascade interest can change the selected TP;
8. TP remains anchored to the filled entry;
9. changing an MRC target line does not blindly set TP to that line;
10. MRC structure can move SL only protectively;
11. a reprice amends the same TP order mark;
12. stall and invalidation use their defined exit policy.
