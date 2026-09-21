# Runledger operator TUI

## Purpose
Provide a read-only terminal view of native jobs and workflows.

## Key entrypoints
`src/main.rs` owns startup; `src/app.rs` owns interaction and state.

## Edit here for X
Terminal rendering and operator interactions belong here; durable queue policy
and persistence remain in the native core/postgres packages.

## Invariants
Keep read-only behavior and Unix-only support. The TUI is a separate binary and
must not enter the default Batter foundation dependency graph.

## Common commands
From the root: `cargo test -p runledger-tui --locked` and
`cargo clippy -p runledger-tui --all-targets --locked -- -D warnings`.
