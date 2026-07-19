# 1m recovery direction override

> Historical note: this document describes the former `1m-recovery` behavior. It is kept as a reference in case the idea is reconsidered later.

## Intended purpose

`1m-recovery` was intended to recognize a possible long recovery when the normal decision was sideways, but the 1-minute MRC structure and the timeframe cascade both supported a bullish direction.

The intended inputs were:

- 1m MRC `bullish_structure == true`;
- positive timeframe cascade direction (`cascade_tf.dir > 0`);
- the current decision being `MarketState::Sideways`.

The name is potentially misleading: this logic did not mean that a position had just recovered from a stop-loss, and it did not calculate a recovery entry price.

## Former implementation

The logic was in `module/src/builder/shared.rs`, inside `refine_common`:

```text
if market_state == Sideways && mrc1.bullish_structure && cascade_tf.dir > 0:
    market_state = Long
    direction_source = "1m-recovery"
    append "1m-recovery" to the entry notes
```

It changed the market direction while preserving the existing `EntryGate`:

- an existing `EntryGate::Open` remained open;
- an existing `EntryGate::Wait` remained waiting;
- the logic did not create a dedicated entry plan or price;
- there was only a bullish/long branch, with no equivalent short branch.

## Why it was added

The idea was to use a bullish 1m MRC structure plus positive cascade direction as a recovery confirmation during sideways movement. This was supposed to prevent the strategy from remaining directionless when the faster MRC and broader cascade already suggested a long move.

The signal was therefore meant to be supporting evidence for an independently valid entry decision, not an entry strategy by itself.

## Known problems

1. It had authority over direction. A sideways decision could become Long without a complete long entry plan.

2. It did not validate the entry price. The price came from whatever entry gate had already been produced.

3. It could preserve a structural fallback entry. When no exit plan was available, the prediction code returned an open structural entry at the current price with the note `no-exit-plan`. `1m-recovery` could then relabel that decision as Long and allow the entry to continue.

4. It could cause immediate re-entry after an exit. In bot 1211, the previous long closed at `0.01309`, and about 6.8 seconds later a new long was filled at `0.01311` with notes:

   ```text
   no-exit-plan | 1m-recovery | structural
   ```

5. The name implied position recovery, but the implementation only inspected current indicators. It did not require a prior stop-loss, a reclaim, a cooldown, or improvement over the previous exit price.

6. The behavior was asymmetric. Only bullish 1m MRC plus positive cascade could activate it; there was no matching bearish short rule.

7. The debug reason was incomplete. The old implementation exposed hardcoded labels and only two boolean/direction values, not the full evidence used to determine whether the move was reliable.

## Safe requirements if reconsidered

If this idea is restored, it should be confirmation only:

- it may annotate or confirm an already-valid Long decision;
- it must not change `Sideways` to `Long`;
- it must not change an `EntryGate::Wait` into `Open`;
- it must not preserve or authorize `no-exit-plan` entries;
- it needs a symmetric short-side rule if both directions are supported;
- any recovery behavior must explicitly define the prior exit, cooldown/reclaim condition, and entry-price policy;
- the complete evidence should be recorded without hardcoded explanatory text.
