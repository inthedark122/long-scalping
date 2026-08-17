# MRC pattern history

This directory is the external, Git-tracked learning dataset for MRC movement
patterns. It is intentionally outside `module/src`: strategy code defines the
classification vocabulary, while this directory records historical evidence
and later outcomes.

## Files

- `schema.json` describes one future per-observation JSONL record.
- `YYYY-MM-DD.json` stores a daily aggregate snapshot.
- Future raw observation exports can use `YYYY-MM-DD.jsonl`; one line is one
  local movement observation.

The first snapshot, `2026-08-17.json`, was aggregated from the one-week CAP and
BEAT research run `mrc-crossing-local-08`. It is a baseline, not a trained
model. Its movement statistics come from completed MRC movements; pattern-level
trade outcomes are `null` until the research runner labels trades against the
same pattern observation.

## Required provenance

Every daily file must retain:

- the source research run identifier;
- the source symbol;
- the observation window in milliseconds and UTC text;
- the schema version;
- whether values are movement outcomes or closed-trade outcomes.

Do not overwrite an old daily file. Add a new date or a new explicitly named
research run and commit it to Git. This keeps changes reviewable and gives a
future training process reproducible snapshots.

## Learning workflow

1. Run the research tool for the same window on every configured symbol.
2. Export local movement observations using the stable pattern identifiers.
3. Append the daily aggregate and, when available, the raw JSONL observations.
4. Commit the dataset separately from strategy source changes.
5. Evaluate predictions walk-forward: only history before an observation may
   contribute to that observation's prediction.
6. Add `filled`, `trade_win`, `net_pnl_pct`, and exit metadata only after the
   simulated/live trade has closed.

The future AI training input should use the raw observation features and the
later outcome labels. It must not train on the full rare crossing path as one
categorical value.
