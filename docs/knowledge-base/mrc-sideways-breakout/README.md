# MRC sideways break and post-dump transition

**Status:** hypothesis; one initial case recorded.

## Intended pattern

1. Price remains sideways inside the MRC outer lines for a sustained period.
2. A sideways strategy earns from that contained movement.
3. Price breaks an MRC line. Wait for directional confirmation before treating
   the break as a possible large move.
4. Track a downtrend through three time-dependent phases:
   1. **Onset:** price moves down toward `down_small` or `down_big`; the MRC
      mean slope changes from near zero to slightly negative. Record when this
      phase begins.
   2. **Established downtrend:** price keeps setting local lows while the MRC
      mean slope becomes increasingly negative. Track elapsed time from onset.
   3. **Dump exhaustion:** the MRC mean slope starts returning toward zero and
      price stops setting lower lows. This marks the end of the dump.
5. Only after dump exhaustion treat sideways movement or a slow uptrend as
   expected next phases.

The anticipated large move is a hypothesis, not an assumed outcome. A stop on a
position held through a confirmed break may be normal; each case should measure
whether range trading earned enough before that stop.

## Implementation gap

The current local experiment does not implement this three-phase model; the
separate explicit reversal setup remains enabled.

## Record for each case

- How long price stayed inside the outer MRC lines, and which lines it touched
  or broke.
- Break direction, first broken line, and confirmation evidence.
- MRC mean slope and price relative to the mean during the range, dump, and
  recovery phases.
- Sideways positions and their net PnL after fees before the break.
- Stop time and price, then post-break favorable and adverse movement.
- Whether the post-dump phase was sideways, a slow recovery, or another dump.

## Cases

- [2026-10-07 CAP: range before sudden downside break](cases/2026-10-07-cap.md)
