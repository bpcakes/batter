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
- `Cargo.toml` owns the feature contract, including the bridges that make every
  native Runledger and Runlimit library package reachable through one dependency.
- `examples/` contains all facade-owned runnable consumers and their support,
  including the optional Axum, SQLx and Runlimit demonstrations.
- `tests/native_transport_consumer.rs` executes the native Runlimit transport
  packages through facade paths and asserts their identity.
- `../../consumers/` holds the external single-dependency consumer sources; see
  the [single-dependency recipe](../../docs/reference-compatibility.md#single-dependency-recipe).

## Edit here for X

Change foundation behavior in `crates/batter-core`. Add facade feature wiring,
adapter namespaces, and isolated consumer checks here as their Beads tasks land.
Keep facade examples on public `batter::...` paths.

## Invariants

The facade must not duplicate runtime or leaf code or create a second type identity.
The default normal dependency graph contains only `batter-core`; adapters must
depend on core rather than this facade. Preserve concrete errors, native futures,
explicit cleanup, tracing target `batter`, and the Unix-only scope.
A native namespace re-exports the native package itself, never a wrapper, and
every one is feature-gated: no feature may silently select another backend or
transport, and `default` stays empty. Reachability moves no ownership — keep
`runledger::register_in` and `runlimit::http` the documented protected paths, and
keep the low-level namespaces named and documented as native with their caller
obligations. A new feature needs a `check_facade_features.py` graph expectation,
a reachability import, an identity assertion and a negative case; the runner
fails on a declared feature with no expectation.

## Common commands

```sh
cargo test -p batter --all-targets --locked
cargo test -p batter --doc --locked
cargo clippy -p batter --all-targets --locked -- -D warnings
python3 scripts/check_facade_features.py
python3 scripts/check_single_facade_consumer.py
```

The last two run from the repository root. The single-dependency consumer needs
Docker and PostgreSQL 18.
