# batter

`batter` is the public facade for the native Tokio operational toolkit. It
keeps the established `batter::...` foundation paths while optional Axum,
SQLx, Runledger, Runlimit, and test-support namespaces are selected through
additive facade features.

The implementation of the foundation lives in [`batter-core`](../batter-core).
The facade only re-exports that implementation and owns the runnable foundation
examples, so applications and adapters share one concrete set of types.

The default feature set is empty. The available feature/module pairs are:

| Feature | Namespace or capability |
| --- | --- |
| `axum` | `batter::axum` |
| `sqlx` | `batter::sqlx` |
| `runledger` | `batter::runledger` plus the required `batter::sqlx` capability |
| `runlimit` | `batter::runlimit` |
| `runlimit-memory` | native memory error bridge through `batter::runlimit` |
| `runlimit-postgres` | native PostgreSQL error bridge through `batter::runlimit` |
| `runlimit-axum` | `batter::runlimit::http` and `batter::axum` |
| `test-support` | `batter::test_support` |
| `sqlx-test-support` | `batter::sqlx::test_support` plus generic support |

For example:

```toml
[dependencies]
batter = { path = "../batter", features = ["axum", "runlimit-axum"] }
```

The facade selects namespaces; native Axum, SQLx, Runledger, Runlimit and the
external PostgreSQL harness remain direct ecosystem dependencies with their
existing ownership and configuration.

This crate follows the workspace's [Unix-only platform policy](../../README.md#platform-support).
Windows is unsupported and not planned.

From the workspace root:

```sh
cargo test -p batter --locked
cargo run -p batter --example worker
cargo run -p batter --example process_owned
cargo run -p batter --example operation_budget
cargo run -p batter --example finite_command
```

Version 0.1.0; Rust 1.94 minimum; publishing disabled. MIT licensed.
