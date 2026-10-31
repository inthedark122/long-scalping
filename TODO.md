# TODO

- [ ] Implement and validate actual MRC trend-following selection. Removing the post-selection countertrend veto was not sufficient: Run 251 made 97 positions (59 short, 38 long) and finished at -$15.44 net, versus Run 249's 59 positions and -$5.21. Inspect why selected staircase directions produce opposite-side MRC entries, then rerun the same window and sizing.
  - [ ] Keep the direction field unset for two-way strategy backtests. Run 250 was accidentally long-only (49 long positions) and is not a valid comparison.
  - [ ] Compare against Run 238 only with matching `cap_init`: Run 251 used 20%; Run 238 order notes show 25%.
- [ ] Commit only after the full-window behavior is accepted.
- [ ] Refactor after behavior is verified and committed, keeping each file responsible for one domain implementation.
- [ ] Keep the separate exit follow-up: inspect Run 238's emergency no-exit-plan closes and same-side staircase invalidation exits. Run 249 had no emergency no-exit-plan close.
- [ ] Keep the separate execution follow-up: inspect Run 237's 14,521-contract order against the 12,000-contract symbol maximum.
