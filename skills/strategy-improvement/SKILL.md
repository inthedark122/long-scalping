---
name: strategy-improvement
description: Debug and improve this repository's LTE strategy by tracing workflow-to-bot decisions, editing source, building and uploading a candidate WASM artifact, backtesting it, and reporting the result. Use for missed-entry investigations and strategy changes; communicate in Russian when the user prefers.
---

# Strategy improvement

Use this skill for investigation and validation of strategy behavior in this repository. Keep the skill instructions and technical notes in English, but conduct discussion and write the final report in the user's preferred language, including Russian.

Read the project [AGENTS.md](../../AGENTS.md) and the user-facing [Russian guide](../../docs/strategy-debugging.ru.md) when this workflow applies. The workflow graph and live configuration are managed in LTE and are not defined by this source tree. Inspect the exact workflow/version/run and module build involved; do not assume a bot, group, strategy, module ID, baseline, or version from an old note.

The user may provide only a screenshot and an optional backtest run ID. Treat those as starting pointers. Retrieve the available run metadata, workflow execution details, strategy/module versions, parameters, debug, orders, and positions from the connected LTE tools and repository. Do not ask the user to trace code, collect debug lines, look up settings, or calculate metrics. If connected tools cannot reach the relevant data, explain the missing access and ask only for the specific information needed to continue.

## Lifecycle

### 1. Discuss and diagnose

Begin with the screenshot and optional run ID. Identify the symbol and time from the image and run context when possible. Retrieve the exact backtest/workflow run, strategy and module build, start/action parameters, direction, margin, leverage, debug, orders, and positions yourself. If multiple runs match, narrow them using available timestamps and ask the user only when the run remains ambiguous.

Trace the handoff in order:

1. **Workflow:** inspect its settings and, when available, its actual execution history to determine whether the trigger and graph path ran and which action, direction, and values it produced. Use LTE workflow run, event, cursor, node, and context diagnostics when available. Do not expect a bot backtest to replay this workflow.
2. **Bot signal:** inspect what the module received. In `module/src/engine.rs`, `Signal(start)` configures direction and parameters and resets analysis state; it does not itself open a position.
3. **Detection and entry decision:** follow the current event path in `engine.rs` into `strategies::evaluate_entry`. Inspect the selected strategy and its exact wait/entry reason in `module/src/strategies/`.
4. **Execution and fill:** inspect `module/src/strategies/execution.rs` for direction, existing-position, sizing, and minimum-notional checks; then inspect order state to distinguish a rejected plan, a pending limit order, and a filled position.

Use the debug output around the timestamp. Fields such as `routed`, `entry-plan-direction`, `allowed-direction`, `entry=`, `constraint=`, `movement-window`, and `primary-trace` help identify the stopping point. The module's once-per-minute `workflow_report` is a module payload, not a complete copy of the LTE workflow graph; inspect the specific graph mapping to see whether it consumes those fields.

State the evidence, the exact failed condition, and any uncertainty in plain language. Keep observation separate from inference. If the user asked only for discussion or explanation, do not edit source. For an improvement request, treat it as authorization to make the necessary focused source change and complete candidate validation; do not stop for approval of individual code edits. If the expected behavior is materially underspecified, explain the ambiguity and ask only the question needed to resolve it, while continuing independent investigation.

For Russian users, a useful first response is: “Сам проверю скриншот и запуск, прослежу путь от workflow до заявки и найду причину в данных и коде. Если ожидаемое поведение понятно, внесу минимальное изменение, соберу кандидат и проверю его в backtest. Результаты объясню по-русски.”

### 2. Discuss the cause and change source

Explain the smallest rule change that would admit the intended case, the additional cases it could admit, and how the backtest will measure both. When the user requests an improvement and the expected behavior is clear, proceed with the focused source change after explaining the diagnosis; do not turn the discussion into a required approval gate. Keep decision logic in its owning feature module and orchestration in `engine.rs`; do not bundle unrelated cleanup or behavior changes.

Keep the source change in the repository. Do not upload source archives or submit server-side source builds. Do not commit unless asked.

### 3. Build and upload a candidate WASM

Use Cargo's default target directory and run commands from the repository root:

```bash
cargo check --manifest-path module/Cargo.toml --locked --bin module_entry
cargo build --manifest-path module/Cargo.toml --locked --bin module_entry --target wasm32-wasip1 --release
```

Upload `module/target/wasm32-wasip1/release/module_entry.wasm` using the connected LTE WASM upload flow: request the module WASM upload URL, upload the built artifact, submit it with `submit_module_wasm`, and poll `get_module_build_status` until success. Keep the returned build ID and pin the backtest candidate to that exact build. Use an unreleased/pre-release strategy version or another isolated candidate path supported by the selected LTE backtest surface. Publishing this candidate build does not activate it on a live bot.

If a candidate cannot be tested without changing live configuration, stop before that mutation and explain the exact isolation problem. A live release is a separate request; when requested, use the dedicated [LTE module release skill](../lte-module-release/SKILL.md).

### 4. Backtest

Use the target and backtest surface selected by the user or established project configuration. Never infer a target from old IDs or a similar symbol. Verify the strategy version, module build ID, workflow version, and action parameters attached to the run.

Compare the candidate with the current baseline on the same symbol, time window, side/direction, margin, leverage, and strategy parameters. When the reported problem concerns a workflow condition, verify both the workflow run path and the bot's decision path. Retrieve debug lines around the missed timestamp when available; if debug data is unavailable for the selected period, say so and base the conclusion on the available orders and positions.

The historical backtest tests the bot/module only; it does not execute the external LTE workflow. Diagnose workflow behavior from its settings and actual execution history separately. If workflow rules changed, state clearly that the bot backtest does not validate that workflow change; it validates only the bot's behavior under the test inputs.

Do not judge an improvement from one screenshot or headline PnL alone. Review whether expected examples became eligible, whether unintended entries appeared, the filled and completed positions, fees, and the effect on exits and drawdown. Use the run's documented gross, fee, and net metrics; reconcile them with position data when the API exposes enough detail.

### 5. Report

Report in the user's preferred language. Include:

- the original hypothesis and the confirmed reason the example did or did not enter;
- the source files and behavior changed;
- module/strategy version, successful WASM build ID, and bot backtest run ID; when relevant, separately identify the workflow version and actual workflow run inspected;
- symbol, period, timezone, direction, margin, leverage, and action parameters;
- baseline versus candidate: crossings and predictions/forecasts when available, entries/orders, filled and completed positions, wins/losses, win rate, gross PnL, fees, net PnL, maximum drawdown, and active/pending/open trade counts when available;
- the missed-entry examples reviewed, new unwanted cases found, missing data, and remaining uncertainty;
- a clear conclusion about whether the change is supported by the evidence and what to investigate next.

After any source or analysis change that can affect analysis results, follow the report requirements in [AGENTS.md](../../AGENTS.md): generate both JSON and HTML research reports directly under `tmp/reports/`, include the available common statistics, compare loaded history with the requested history, refresh a short cache, and record any remaining shortfall. Never use system `/tmp`.

## Russian report shape

When responding in Russian, use a concise structure such as:

1. **Причина пропущенного входа** — exact workflow/module/execution condition and evidence.
2. **Изменение** — what changed and which cases it is intended to admit.
3. **Проверка** — versions, build, settings, symbol, period, and baseline/candidate metrics.
4. **Вывод и ограничения** — what the backtest supports, what it does not establish, and the next useful investigation.
