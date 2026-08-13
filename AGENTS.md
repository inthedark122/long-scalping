# Repository Agent Guidelines

## Rust tests

- Keep new Rust tests in the crate's separate `tests/` directory, not in
  `#[cfg(test)]` modules inside `src/` files.
- Mirror the source layout when moving tests: `module/src/engine.rs` maps to
  `module/tests/engine.rs`, and `module/src/execution.rs` maps to
  `module/tests/execution.rs`.
- Prefer testing behavior through the crate's public API and the same event
  flow used in production. Run the focused tests and the full `cargo test`
  suite after moving or adding tests.
