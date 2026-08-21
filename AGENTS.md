# Repository Agent Guidelines

## Workspace and temporary files

- Use the repository-local `tmp/` directory at `/home/dev/strategies/long-scalping/tmp/` for temporary files, caches, reports, generated HTML, command output, and build artifacts.
- Never use the system `/tmp`, `module/tmp`, or another module-local temporary directory for these files.
- Run repository commands from `/home/dev/strategies/long-scalping` unless a command explicitly requires another working directory.
- Run module binaries from the repository root using `--manifest-path module/Cargo.toml`; do not `cd` into `module/` for the standard workflow.

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
    --output tmp/reports/ \
    --html tmp/reports/
  ```

- Include the common statistics available from the run: crossings,
  predictions/forecasts, entries, completed positions, wins/losses, win rate,
  gross PnL, fees, net PnL, max drawdown, and active/pending/open trade counts.
- Compare each report's loaded history with the requested `--history`. If a
  cache is shorter than requested, rerun that symbol with `--refresh-cache`
  before accepting the report; record any remaining data shortfall explicitly.
