# Scalp / Micro-Grid Volume Experiment (v0.61-scalp)

## Purpose
The user wanted **~10× more trading VOLUME** than v0.61's ~6,900/month — "like a grid that
trades all the time to take a small profit very often." This records the maker micro-scalp
built to chase that, its backtest result, why it failed, and the key constraint it revealed.
Kept as institutional knowledge so we don't re-run this dead end.

## Design (branch `v0.61-scalp`, module version `0.61-scalp`)
Added a maker micro-scalp block to `compute_limit_orders`, const-gated (`SCALP_ON`):
- On the **controlling (direction) side**: a ladder of `SCALP_RUNGS = 3` MAKER entry limits — sell
  ABOVE price / buy BELOW price (non-crossing = maker) — `SCALP_STEP_PCT = 0.06%` apart, each
  `SCALP_RUNG_FRAC = 10%` of the per-side risk budget, total capped by `r_side`.
- One **reduce-only TP** 0.06% inside the avg entry. 0.06% was picked to clear an assumed
  ~0.04% maker round-trip fee (≈ +0.02% net/cycle). User spec: *"limit in and out, enter/exit
  multiple times inside one candle, ≥0.06% to cover 0.04% fee."*
- **Additive** on top of v0.61 — the tight TP also fast-cycles the trend positions → max turnover.

## Backtest result (Run 94, version 76, m10, Apr30–May28)
| | v0.61-scalp | baseline |
|---|--:|--:|
| volume | **30,219 (4.4×)** | 6,903 |
| gross profit | 3.21 | 3.32 |
| fees | **9.70** | 1.72 |
| **NET** | **−$6.49** | +$1.60 |

It produced 4.4× the volume but **lost $6.49 — fees were 3× the gross profit.**

## Why it failed — THE KEY LEARNING
- **The backtest charges a flat ~0.025–0.032% PER SIDE regardless of maker/taker** (baseline
  0.0249%/vol, scalp 0.0321%/vol). It does **not** model the maker-fee discount.
- So every round-trip pays ~0.06% in fees, and a 0.06% TP barely covers that → ~zero edge per
  cycle, pushed negative by imperfect fills.
- The tight TP also **hijacks the trend positions** (closes them at +0.06% instead of riding to
  the MRC targets), destroying the trend edge that gives v0.61 its +$1.60.
- **Therefore: ~10× volume AND net-positive is NOT reachable in the backtest at this fee rate.**
  The maker advantage you'd rely on (0.02%/side) is real *live* but invisible to the backtest —
  so a tight maker scalp can only be validated LIVE, not by backtest.

## Related finding (same session)
`action_config.action_params` are **NOT honored** by the backtest — it uses the group's
`signal_actions[].default` values. So params are effectively fixed at defaults, and **every
improvement must be a CODE change + redeploy** (a new module version), not a param tweak.

## Forward options (none pursued — awaiting user)
1. **Wider TP (~0.10–0.15%)** — clears the backtest fee with margin, can show net-positive, but
   fewer cycles → volume only ~2–3×, not 10×.
2. **Small LIVE maker scalp test** — the only way to prove the maker edge the backtest can't model.
3. **Drop the volume push** — the strategy's edge is thin/fee-dominated (fees already 52–88% of
   gross over 1–6 months), so pure volume-via-scalping is fee-negative by construction.

## History of Changes
### v0.61-scalp — 2026-05-31 (built + backtested → REJECTED)
Maker micro-scalp (0.06% TP, const `SCALP_ON`). Run 94: **4.4× volume, NET −$6.49.** Rejected.
Branch `v0.61-scalp` kept for the record. Discovered the backtest fee model has **no maker/taker
distinction**, which makes a tight-TP volume scalp un-validatable as profitable in backtest.
