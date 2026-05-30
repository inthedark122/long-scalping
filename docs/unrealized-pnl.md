# Self-Computed Unrealized PnL

## Purpose
The platform's `ModulePositionSummary.pnl` field is **realized** PnL, not unrealized — it stays ~0 on an open position until a partial/full close realizes something. The module had been treating it as unrealized, which (a) **blinded the margin/staged SL** (they gate on `total_pnl`, which read ~0 even when an open position was deeply underwater) and (b) **corrupted the `exp_pnl` TP filter**, letting it place reduce-only "take-profits" *below* the entry price (locking losses). Both were observed live on 2026-05-29 and realized ~$0.02 of avoidable loss. This feature computes the true unrealized PnL inside the module.

## Core Concept
`uPnL = (mark − entry_price) · qty` for a long, `(entry_price − mark) · qty` for a short, where `mark = input.price`, and `entry_price`/`qty` come from `ModulePositionSummary` (qty is a positive magnitude). `position.pnl` is left for realized accounting only.

## State
No new state. Helper in `engine.rs`:
```rust
fn upnl(pos: Option<&ModulePositionSummary>, mark: f64, is_long: bool) -> f64 {
    pos.map(|p| if is_long { (mark - p.entry_price) * p.qty } else { (p.entry_price - mark) * p.qty })
       .unwrap_or(0.0)
}
```

## State Transitions
None — pure derivation each call. Applied at every site that needs *current position PnL*:
- `handle_sug_info`: `long_pnl`/`short_pnl`/`total_pnl` → margin SL, staged partial SL, NTPS close, MFE tracking, free-margin display.
- `compute_limit_orders`: `long_pnl`/`short_pnl` → `exp_pnl` TP gate, free-margin gate.
- `compute_auto_open`: `live_pnl` → free-margin estimate.
- `win_mult(pos, mark, is_long)` → win-scaling uses uPnL/notional.

## Signals Derived
| Consumer | Before (realized ~0) | After (true unrealized) |
|----------|----------------------|-------------------------|
| Margin SL / staged SL | never fired on open drawdown | fires correctly |
| `exp_pnl` TP filter | approved below-entry TPs | rejects them (only true-profit TPs) |
| MFE (`*_pnl_mfe`) | meaningless | real peak unrealized |
| free-margin gate | ignored open drawdown | accounts for it |

## History of Changes
### v0.49.0 — 2026-05-29
Initial fix. Root cause confirmed live: sug line showed `pnl=0.0000` for ~2h on an underwater long, then `pnl=−0.0213` exactly equal to the realized loss from two partial closes — proving the field is realized. Self-computed uPnL restores the SL ladder and stops below-entry TP placement. See `analysis_notes.md` 13:54 entry.
