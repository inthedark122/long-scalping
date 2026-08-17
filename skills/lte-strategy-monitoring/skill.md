---
name: lte-strategy-monitoring
description: Read-only, self-rescheduling monitoring for any LTE bot group. Fetch bot state, positions, orders, balances, market data, and the target strategy's declared diagnostics in parallel; produce concise reports, flag escalations, and recommend actions without opening, closing, stopping, or deploying unless the user explicitly authorizes it.
---

# LTE strategy monitoring

Monitor a configured LTE bot group as a read-only watch loop. The workflow is
platform-aware but strategy- and bot-agnostic: all bot IDs, symbols, module
IDs, indicator feeds, action capabilities, thresholds, and deployment fields
come from the target profile or the current LTE configuration.

Use `ScheduleWakeup` for a recurring loop. Every wake-up must carry a
self-contained prompt containing the current state, the parallel fetch plan,
the escalation rules, the action authorization rule, and the next interval.

## Target profile

Resolve the target before the first check. Never infer a bot from a default
group or from a similar symbol. Keep this profile in the monitoring state:

```text
group_id, bot_id, symbol_key, exchange, account_id
module_id, live_module_version, strategy_name
declared_signal_inputs, optional_observability_inputs
balance_floor_warn, balance_floor_stop
normal_interval, decision_interval, idle_interval
notes_path, deployment_capabilities, supported_actions
```

If a field cannot be discovered, report the missing field and continue only
with checks that are safe and meaningful. Do not silently substitute another
bot, account, symbol, or strategy.

## Check sequence

Run the same sequence on every tick:

1. confirm target identity and source freshness;
2. fetch independent sources in parallel;
3. normalize positions, orders, prices, balances, and strategy state by
   timestamp;
4. compare with the previous tick to detect transitions;
5. apply the target strategy adapter and escalation thresholds;
6. produce the one-line report and only the relevant evidence;
7. gate any requested operation through the safety contract;
8. append notable events and schedule exactly one next wake-up.

If a source is stale, partial, or contradictory, downgrade the conclusion and
say what is missing. Never fill a missing value from an earlier target or from
memory without labeling it as stale.

## Safety contract — recommend, do not execute

The default operation is read-only. Never open, close, bank, add, hedge, stop,
restart, change configuration, upload a module, or deploy based only on a
market read.

Recommend the next action and wait for an explicit user keyword in the current
turn. An action may be taken only when all of these are true:

1. the user explicitly requested that action now, or a previously documented
   hard-stop policy applies;
2. the target profile supports the action;
3. the exact target, side, size, and expected state transition are known;
4. the action's effect on open positions and live configuration has been
   stated;
5. the result is verified after the action.

For a hard-stop floor, prefer raising the alert first. If the profile
explicitly authorizes automatic protection, close only the configured target,
verify the position is flat, and report the realized result and balance.

### User action keywords

Map these words to the target connector's operations only when the target
supports them. The mapping is intentionally generic; never embed a bot ID in
the skill.

- `bank`, `close`, `close long`, `close short` → force-close the requested side;
  in a connector that supports it this maps to `run_bot_action(target.bot_id,
  force_close_long|force_close_short)`; report fill, realized PnL, fees, and
  balance.
- `long`, `short`, `force long`, `force short`, `add`, `stair`, `risk N` →
  request the configured `force_open_long|force_open_short` operation after
  calculating and confirming size and stop risk.
- `flat`, `close all` → close all supported sides, stop only if requested, and
  conclude after flatness is verified.
- `hold`, `resume`, `wait` → keep the current live state and continue watching.
- `stop` → stop the configured bot only after stating that the operation may
  flatten positions or cancel orders according to the connector.
- `slow`, `stretch` → lengthen the monitoring interval.
- `deploy`, `deploy a` → follow the deployment flow below; never deploy an
  untested version implicitly.

Ambiguous words are not authorization. Ask for the missing side, target, or
size rather than guessing.

## Fetch plan — parallel on every check

Fetch independent sources in parallel, then reconcile them by timestamp and
target identity:

1. **Bot/group state:** status, last debug, market mode, module state, live
   module version, realized PnL, and the latest strategy diagnostics.
2. **Open positions:** side, signed quantity, entry price, mark price, fees,
   stop/target, and position status.
3. **Orders:** recent entries, exits, partial fills, cancellations, notes, and
   order timestamps.
4. **Account:** available balance, total/equity balance when available, frozen
   margin, and account currency.
5. **Market feed:** current price and only the feeds declared by the target
   profile. This can include candles, order book, open interest, flow, or
   activity, but undeclared fields must not become strategy conclusions.
6. **Strategy indicators:** the target's declared timeframe snapshots,
   pattern state, target/stop, risk state, and selector state.

Use the LTE connector's equivalent operations for these logical requests. An
external feed is optional and must be labeled with its source and timestamp.
If sources disagree, report the disagreement instead of creating a synthetic
value.

Typical LTE operation mappings are `list_bots(group_id)`,
`list_bot_positions(bot_id, status=InProgress)`,
`list_bot_orders(group_id, limit=5)`, and
`get_account_balances(account_id)`. Use the connector's actual names and
schemas, and verify that the returned group, bot, account, and symbol match the
target profile.

For a target configured with a Projection WS-style feed, the optional adapter
can expose current price, activity/NTPS, 1h/4h/24h price context, 1m/1h/4h
klines, order-book walls with resistance/support and buy/sell amounts, open
interest and change, and net flow. A configured feed script may support
snapshot, stream, and raw modes. These fields are monitoring context unless
the strategy profile explicitly declares them as signal inputs.

For a target configured with an MRC indicator endpoint, the optional adapter
can expose 1m and 5m MRC lines, ML, DownInner, DownBig, CurrentCross, candle
Close/High, and any documented reversal fields. Record the endpoint's symbol,
timeframe, limit, and timestamp with the snapshot; never hardcode them in the
monitoring loop.

## Reconciliation and PnL

Use the position snapshot for live state and the order stream for transitions.
Do not call a position realized merely because its mark moved.

For a signed quantity:

```text
uPnL = (mark_price - entry_price) × signed_quantity
```

Negative quantity represents a short. Include estimated exit fees when making
a recommendation, and distinguish estimated unrealized PnL from realized
PnL. A field named `position.pnl` may be realized; verify its semantics before
using it as live PnL.

Available balance may equal total equity minus frozen margin from open
positions and pending limit orders. It can fall while orders are open and jump
when the account becomes flat. Judge a balance floor against total/equity or a
profile-defined when-flat value, not available balance alone.

The historical test profile used a warning floor below `0.55` and a stop floor
below `0.50`, with restarts costing approximately `0.50`. These values are
examples to preserve when that profile is selected, not defaults for another
account or currency.

## Strategy adapter

The monitoring core does not assume how a strategy makes decisions. The target
profile must define an adapter with:

- signal and pattern names;
- valid timeframes and fields;
- active candidate/selector state;
- target, stop, continuation, and invalidation fields;
- evidence required to call a signal strong, weak, or ending;
- strategy-specific escalation rules;
- fields that are diagnostic only and must not affect the conclusion.

The adapter must state whether a field is an input to the strategy, an
observability signal, or an unavailable legacy field. Never resurrect a deleted
strategy dependency merely because an old debug field still exists.

### MRC/volume adapter retained from the original monitor

For a target that declares the MRC/volume adapter, preserve these concepts:

- **MRC mean line — three states:** a soft touch uses the ML angle (`state.angle`
  when documented) to project drift; a hard break and hold below/above the ML
  indicates continuation; a breakout outside the MRC Big can be a flight move
  where a line-based TP is inappropriate.
- **Higher-timeframe structure:** the higher-timeframe ML provides room/target
  when price is far from it; higher-timeframe down/up lines can be stair or
  target levels; the fast-timeframe ML can be a re-entry area. A documented
  reversal direction plus a high retracement/Fibo value can identify an
  exhausted counter-move, but it is supporting evidence only.
- **Activity:** an activity/transaction-rate feed can flag momentum or
  exhaustion. The historical MRC monitor used a spike above 2× average as a
  momentum/fly warning, `>400` as warning, and `>600` as danger. These are
  profile values, not universal defaults.
- **Debug fields:** when documented by the module version, preserve fields such
  as `ntps=` (activity), `ss=` (stair zone), `b5=` (higher-timeframe hard
  break), `run=` (price run), `inv=` (invalid side), and `negL`/`negS`
  (negative exits). Unknown or stale debug fields are not evidence.

For an MRC pattern-engine target, the strategy conclusion is based on MRC
crossings, all-pattern classification, selector state, and volume when those
are the declared inputs. ZigZag, SMC, fast-candle, cascade, order-book, OI,
flow, and higher-timeframe fields may be reported as optional telemetry only if
the target profile declares them; they must not silently become trading inputs.

## Sizing and risk recommendations

Only calculate a force-entry size when the user requests an entry or the
profile explicitly requires a pre-authorized protection action. Resolve all
units from the connector before sending an operation.

For a risk-sized notional, the reusable formula is:

```text
notional = min((max_amount / leverage × risk_fraction) / stop_distance,
               max_amount)
```

Here `stop_distance` is a decimal price percentage, `risk_fraction` is the
configured fraction of the allowed loss, and `max_amount` is the target's
notional cap unless the connector documents another unit. State the resulting
notional, approximate quantity, stop distance, leverage, fees, and worst-case
risk before acting. Reject a size that violates exchange minimums or the
configured risk cap.

For reference, the historical example used `max_amount=50`, `leverage=100`,
`risk_fraction=0.40`, and `stop_distance=0.007`, producing roughly `0.014`
units at the example price. It also treated a force-entry `amount` as notional
when that was the connector contract. Reuse those numbers only when the target
profile explicitly declares the same configuration.

## One-line report and full report

Lead every check with one stable line:

```text
price | market mode | position(side, signed qty, uPnL vs entry) | balance/equity | strategy state | primary activity
```

For an activity-enabled target, the final fields can be rendered as the
historical compact form `price | mkt_mode | position | balance | activity`.
Keep the stable one-liner even when a richer full report follows.

Then expand only changed or actionable items:

- position opened, closed, resized, or partially filled;
- order created, filled, canceled, expired, or replaced;
- target, stop, invalidation, or continuation state changed;
- strategy evidence strengthened or weakened;
- balance or risk-floor escalation;
- connector/module/version mismatch;
- data freshness or source disagreement.

For a no-change tick, keep the report to the one-line summary and one sentence
confirming that no escalation fired. Always report while a position is open or
an escalation is active.

## Escalation rules

Use the target profile's thresholds and classify each alert:

- **INFO:** routine fill, state change, or a new candidate;
- **WARN:** thesis weakening, target/stop proximity, stale data, partial fill,
  activity anomaly, or balance below the warning floor;
- **URGENT:** stop/target hit, position unexpectedly changed, account below the
  stop floor, bot status mismatch, or a failed safety operation.

For MRC/volume targets, the original escalation cases remain available:

- a higher-timeframe ML repair/reclaim after a hard break;
- price reaching a down/up line or stair/target zone;
- a fast-timeframe Big breakout or flight move;
- a market-read weakening or declared reversal flip;
- balance crossing the configured warning/stop floors;
- activity crossing configured warning/danger thresholds;
- buy/sell gain exceeding the configured absolute threshold; the historical
  example used `|BSG| > 2`;
- a single net-flow flush that does not break price structure, which is chop
  evidence rather than confirmation. Require a price-structure break before
  treating that flush as a real move.

Every escalation must include evidence, timestamp, impact, recommended action,
and whether user authorization is required. Do not convert an escalation into
an operation automatically.

## Deployment flow — only on explicit `deploy`

Deployment is a separate, high-impact operation. Follow the target's release
skill and profile. The generic sequence is:

1. compile/check the requested version for the target runtime with zero
   warnings (for WASM targets this may be `cargo check --target
   wasm32-wasip1`);
2. package only the declared module source and manifest, for example
   `Cargo.toml` and `src/`;
3. obtain an upload URL for the configured module ID and version;
4. upload and verify the HTTP result, normally HTTP 200, before the URL
   expires;
5. submit the server build and poll until `Success`; on `Failed`, report the
   compiler output;
6. if backtesting, patch the configured test group's module-version paths,
   including both `strategy.module_version` and
   `strategy.professional.dynamic_module.module_version` when both exist;
7. run with an explicit action configuration containing the signal action,
   margin, leverage, trigger, and strategy parameters;
8. poll until the backtest is `FINISHED` and self-calculate net PnL from
   positions as `Σ total_profit − Σ fee`;
9. attribute results by order note/entry type, compare against the configured
   baseline on the same window and parameters, and revert test configuration
   immediately;
10. deploy live only after explicit approval, flattening and confirming open
    positions if the stop/start flow requires it; patch every live version path,
    rename if required by the release policy, stop and verify the bot, then
    start it with the configured symbol, live price, and parameters;
11. verify the running version, status, positions, orders, and first debug
    snapshot after restart.

The corresponding LTE operations are commonly named
`get_module_upload_url`, `submit_module_build`, `get_module_build_status`,
`update_bot_group`, `run_bot_action`, and `run_bot_group_action`. A stop/start
deployment can close open positions, so flatten and confirm first whenever the
target connector has that behavior.

Never leave a live group pointing at an untested version. A configuration
change may not affect an in-memory bot until restart; verify both facts rather
than assuming either one.

## Analysis notes

Append only notable events to the profile's configured notes file:

```text
[HH:MM] <what changed> — <evidence> — <why it matters> — <action/status>
```

Record entries, closes, partial fills, bugs, new market behavior, deployment
results, and design insights. A legacy configuration may use a file named
`analysis_notes.md`; keep that path in the target profile rather than baking it
into this skill. Skip no-change ticks. Do not write credentials, private keys,
or unredacted account secrets.

## Cadence and self-rescheduling

Use the profile's cadence as the default. A reasonable starting policy is:

- **90 seconds** during normal monitoring;
- **60–75 seconds** at a live decision point near a target, reclaim, break, or
  position transition;
- **about 240 seconds** during dead chop or after `slow`.

Keep the interval below the prompt-cache boundary when freshness matters; a
longer idle interval may cause a cache miss, so choose it deliberately. Do not
schedule multiple overlapping wake-ups. Pass the same self-contained prompt
back on every tick and carry forward the latest target profile, state, source
timestamps, open-position context, and unresolved escalations.

## Starting and stopping

On `/lte-strategy-monitoring`:

1. resolve and confirm the target profile;
2. run the first full parallel check immediately;
3. report the one-line state and any escalations;
4. schedule the next wake-up and tell the user monitoring is running.

On `stop monitoring`, do not schedule another wake-up. Confirm that the loop
has stopped; do not stop the trading bot unless the user separately requested
that action.
