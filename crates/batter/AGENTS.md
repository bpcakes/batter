# Batter facade guide

## Purpose

Own the public `batter` namespace and its compatibility examples. The native
foundation implementation is in [`batter-core`](../batter-core); adapter
implementations remain in their own packages and are exposed here only through
explicit optional features.

Follow the root [Unix-only platform policy](../../AGENTS.md#platform-scope).
Windows support and non-Unix fallbacks are out of scope.

## Key entrypoints

- `src/lib.rs` re-exports the core implementation and owns facade doctests.
- `examples/` contains all facade-owned runnable consumers and their support,
  including the optional Axum, SQLx and Runlimit demonstrations.

## Edit here for X

Change foundation behavior in `crates/batter-core`. Add facade feature wiring,
adapter namespaces, and isolated consumer checks here as their Beads tasks land.
Keep facade examples on public `batter::...` paths.

## Invariants

The facade must not duplicate runtime or leaf code or create a second type identity.
The default normal dependency graph contains only `batter-core`; adapters must
depend on core rather than this facade. Preserve concrete errors, native futures,
explicit cleanup, tracing target `batter`, and the Unix-only scope.

## Common commands

```sh
cargo test -p batter --all-targets --locked
cargo test -p batter --doc --locked
cargo clippy -p batter --all-targets --locked -- -D warnings
```
