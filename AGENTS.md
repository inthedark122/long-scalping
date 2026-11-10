# Repository Agent Guidelines

## Workspace and temporary files

- Use the repository-local `tmp/` directory at `/home/dev/strategies/long-scalping/tmp/` for temporary files, caches, reports, generated HTML, and command output.
- Use Cargo's default target directory for build artifacts. Do not set `CARGO_TARGET_DIR` or pass `--target-dir`.
- For LTE module releases, build and upload WASM only. Use the `wasm32-wasip1` artifact with LTE's WASM upload and submit tools. Never upload Rust source archives or submit server-side source builds.
- Never use the system `/tmp`, `module/tmp`, or another module-local temporary directory for temporary files.
- Run repository commands from `/home/dev/strategies/long-scalping` unless a command explicitly requires another working directory.
- Run module binaries from the repository root using `--manifest-path module/Cargo.toml`; do not `cd` into `module/` for the standard workflow.

## Code organization and implementation scope

- Follow the user's explicit request exactly. Do not replace the requested
  behavior with a workaround, shortcut, blanket wait/block, disable switch, or
  another behavior of your own choosing.
- Do not guess, infer extra requirements, or broaden the scope. When a detail
  needed to implement the request is unclear, inspect the repository and
  conversation for an answer; if it remains unclear, ask before changing that
  behavior.
- Treat user corrections as authoritative. Do not repeat an approach the user
  has rejected or reinterpret a correction to justify it.
- When the user asks for an explanation, explain the current behavior without
  changing code or documents unless they also ask for a change.
- Organize implementation by domain and feature. Keep each feature's types,
  rules, and helpers in its own focused module or module directory instead of
  expanding broad files such as `engine.rs` with feature logic. For example,
  MRC line and level behavior belongs under
  `module/src/detection/mrc_levels/`.
- Keep orchestration code focused on routing events and coordinating feature
  modules. Put domain decisions and calculations inside the module that owns
  that behavior, with explicit inputs and outputs between modules.
- Scope each implementation to one requested feature or one clearly defined
  solution. Do not bundle unrelated fixes, cleanup, or behavior changes into
  the same implementation.
- Prefer the simplest solution that fixes the identified cause. Remove
  unnecessary or conflicting behavior instead of layering new code around it.
  Overly complex solutions, kludges, and makeshift fixes are unacceptable.
- Prioritize clear structure, readable code, and well-defined ownership over
  adding or changing strategy behavior. Implement strategy behavior exactly as
  specified; do not infer, broaden, or silently alter requirements.
- Minimize side effects and hidden state changes. Keep mutations local to the
  feature that owns the state and make state changes explicit at module
  boundaries.
- Keep comments sparse. Use them to explain non-obvious rationale or
  constraints, not to restate what the code already expresses.

## Rust tests

- Keep new Rust tests in the crate's separate `tests/` directory, not in
  `#[cfg(test)]` modules inside `src/` files.
- Mirror the source layout when moving tests: `module/src/engine.rs` maps to
  `module/tests/engine.rs`, and `module/src/execution.rs` maps to
  `module/tests/execution.rs`.
- Prefer testing behavior through the crate's public API and the same event
  flow used in production. Run the focused tests and the full `cargo test`
  suite after moving or adding tests.

## Analysis reports

- After any analysis change or code change that can affect analysis results,
  generate a new report using the standard report workflow and include the
  common statistics in that report.
- Store reports directly under `tmp/reports/`. Do not create `html/` or `json/`
  subdirectories under it.
- Every analysis run must produce both JSON and HTML reports directly in
  `tmp/reports/`. For multiple symbols, the report tool generates one JSON and
  one HTML file per symbol. Verify that all expected files exist after the run.
- Use the repository-root command pattern below, replacing `<run-name>` and
  the analysis arguments as needed:

  ```text
  cargo run --manifest-path module/Cargo.toml --locked \
    --features mrc-research,native-tools \
    --bin mrc_crossing_research -- \
    --symbol-key 'OKX#CAP-USDT-SWAP#SWAP' \
    --symbol-key 'OKX#BEAT-USDT-SWAP#SWAP' \
    --symbol-key 'OKX#ETH-USDT-SWAP#SWAP' \
    --history 10000 \
    --output tmp/reports/ \
    --html tmp/reports/
  ```

- Include the common statistics available from the run: crossings,
  predictions/forecasts, entries, completed positions, wins/losses, win rate,
  gross PnL, fees, net PnL, max drawdown, and active/pending/open trade counts.
- Compare each report's loaded history with the requested `--history`. If a
  cache is shorter than requested, rerun that symbol with `--refresh-cache`
  before accepting the report; record any remaining data shortfall explicitly.
