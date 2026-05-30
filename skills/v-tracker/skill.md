---
name: v-tracker
description: On-demand V-tracking report for the LTE bot (default group 29 / bot 694). Fetches the live bot debug, extracts the `vt=` field (1m MRC per-line V-movement state), and prints a decoded table — V counts, last penetration power %, and exhaustion per line. V-tracker ONLY — no MRC line prices, no expectation narrative. Trigger when the user asks for "V", "V-tracker", or "V info".
---

# V-Tracker Report

Prints the current V-movement tracker state for the bot. Default target **group 29 / bot 694** (OKX ETH-USDT-SWAP).

## What it is
The V-tracker (`v_lines`, engine `update_v_lines`) records, for each **1m MRC line**, the "V" spikes where price pokes through the line and returns. It is **1m-ONLY** — there is no 5m V-tracker. The bot exposes the state in the `vt=` field of its **sug** debug line.

## Procedure
1. `mcp__claude_ai_LTE__list_bots(bot_group_id=29, status="Active")` → read `context.last_debug`.
2. Find the `vt=` token. **Two sources:**
   - **v0.57+ debug switch (preferred):** if `debug_info` includes bit 0 (the **V** preset, value 1), `vt=` rides the `sw[...]` suffix of **every** line (`init`/`ind`/`sug`) — so a single fetch works regardless of event. If `debug_info=0` and the snapshot is an `ind` line with no `vt=`, the V preset is simply off: push `debug_info=1` via the Start/config action (applies next tick — context accumulation), read the next snapshot, then push `debug_info=0` to restore default. See `docs/debug-info-builder.md`.
   - **Pre-v0.57 (legacy):** `vt=` appears ONLY on **sug** ticks. If `last_debug` is an `ind ...` line, the MCP caches one snapshot ~1/min, so re-fetching rarely catches a `sug` line — this is exactly the gap v0.57 fixes.
3. Take the substring after `vt=` — 6 space-separated line tokens. Also grab `t=` (ticks) and `p=` (price) from the same line for the header.
4. Parse each token `{LINE}{pos}L{lc}({lp}%{le})S{sc}({sp}%{se})` with
   `re.findall(r'([A-Z]{2})([\^v])L(\d+)\((\d+)%(!?)\)S(\d+)\((\d+)%(!?)\)', vt)`:
   - LINE ∈ {UB, US, UI, ML, DI, DB}
   - pos: `^` = price **above** the line, `v` = **below**
   - L = resistance-Vs (price spiked above, fell back): count `lc`, last power `lp`%, `!`=exhausted
   - S = support-Vs (price dipped below, recovered): count `sc`, last power `sp`%, `!`=exhausted

## Output format (print exactly this — nothing more)
```
V-TRACKER (1m MRC lines) — bot 694 · t=<ticks> · price <p>
 Line     pos   Resistance-Vs (L)        Support-Vs (S)
 UpBig     <^/v>  <lc> @<lp>% <exh>       <sc> @<sp>% <exh>
 UpSmall   ...
 UpInner   ...
 MeanLine  ...
 DownInner ...
 DownBig   ...
legend: ^ price above line · v below · L=spiked above & fell back (resistance test) · S=dipped below & recovered (support test) · power=last penetration as % of MRC channel · ! = exhausted (last V < 70% of prior → momentum fading)
read: <ONE sentence — which line is active, exhaustion state, where price sits>
```
(`<exh>` = `(exhausted)` when `!` present, else blank. Show `0` plainly for empty lines.)

## Rules (non-negotiable)
- **V-tracker ONLY.** Do NOT add MRC line prices, the 5m line map, or any movement/price expectation. The user explicitly wants just this (memory `feedback_v_tracker_report`).
- Caveats — mention only if relevant: (a) **1m-only**, no 5m V-tracker; (b) the count increments on **every crossing**, so it inflates on noise (the `power%`/`!exhausted` flags mark the insignificant ones).
- If targeting a different bot, swap the `bot_group_id` / bot label.
