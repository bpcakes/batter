# batter

`batter` is the public facade for the native Tokio operational toolkit. It
keeps the established `batter::...` foundation paths while optional at-rest,
outbound HTTP, Axum, SQLx, Runledger, Runlimit, flags, and test-support namespaces are
selected through additive facade features.

The implementation of the foundation lives in [`batter-core`](../batter-core).
The facade only re-exports that implementation and owns the runnable foundation
examples, so applications and adapters share one concrete set of types.
Owned service roots await `wait_checked().await?` or request
`shutdown_checked().await?`; successful completion retains its report, and
unsuccessful completion keeps the original report or coordinator error.

The default feature set is empty. The available feature/module pairs are:

| Feature | Namespace or capability |
| --- | --- |
| `at-rest` | `batter::at_rest`, preserving direct `batter_at_rest` type identity; [MIT-licensed dependency](../batter-at-rest/LICENSE) |
| `axum` | `batter::axum` |
| `metrics` | `batter::telemetry::metrics`: bounded foundation outcome metrics through the `metrics` facade; no recorder or exporter is installed |
| `outbound-http` | `batter::outbound_http`, preserving direct `batter_outbound_http` type identity, with its `operation` bridge selected so `PreparedRequest::send_in` takes `batter::operation::OperationContext` |
| `sqlx` | `batter::sqlx` |
| `runledger` | `batter::runledger` and `batter::runledger::native::{core, postgres, runtime}`, plus the required `batter::sqlx` capability |
| `runledger-test-support` | `batter::runledger::test_support`: drive one job handler without a worker, queue or database; and `batter::runledger::native::test_support`: test-only reachability for Runledger's own PostgreSQL support |
| `runlimit` | `batter::runlimit` and `batter::runlimit::native` |
| `runlimit-memory` | native memory error bridge plus `batter::runlimit::memory` |
| `runlimit-postgres` | native PostgreSQL error bridge plus `batter::runlimit::postgres` |
| `runlimit-axum` | `batter::runlimit::http` and `batter::axum` |
| `runlimit-native-http` | `batter::runlimit::native_transport::http`: native response metadata, no transport framework |
| `runlimit-native-axum` | `batter::runlimit::native_transport::axum`: the native caller-controlled layer, without `batter::axum` |
| `flags` | `batter::flags` and `batter::flags::core` (experimental, [ADR-016](../../docs/adr/016-typed-flags-control-plane.md)): typed flags and kill switches declared once in code, `declared::resolve` from the application's settings source with its startup report and the periodic gate; `test::TestFlags` needs `flags-test-support` |
| `flags-postgres` | `batter::flags::postgres`, `batter::flags::control_plane` and `batter::flags::grants` (experimental): the audited store and the protected registration of its refresh, heartbeat and prune, plus the required `batter::sqlx` capability |
| `flags-axum` | `batter::flags::http` (experimental): route-group gates and per-request flag views, plus `batter::axum` |
| `flags-runledger` | `batter::flags::jobs` (experimental): job-handler gates and pinned decisions, plus `batter::runledger` |
| `flags-test-support` | `batter::flags::test::TestFlags` (experimental): injected test flags serving `Source::Test`, for test builds only |
| `test-support` | `batter::test_support` |
| `sqlx-test-support` | `batter::sqlx::test_support` plus generic support |

The five `flags*` features are experimental. `flags` alone is declared mode,
with no database; the [`rollout` example](examples/rollout.rs) runs in both
declared and control-plane mode. The `batter-flags` operator command is the
unpublished [`batter-flags-cli`](../batter-flags-cli/README.md) package.

For example:

```toml
[dependencies]
batter = { path = "../batter", features = ["axum", "runlimit-axum"] }
```

The facade selects namespaces; `batter-at-rest`, `batter-outbound-http`, native Axum, SQLx,
Runledger, Runlimit and the external PostgreSQL harness remain direct ecosystem
dependencies with their existing ownership and configuration.

The `native`, `memory`, `postgres` and `native_transport` namespaces are the
native packages themselves, so one `batter` dependency needs no direct
native-package declaration and no `[patch]`; see the
[single-dependency recipe](../../docs/reference-compatibility.md#single-dependency-recipe).
They are deliberately low-level and do not move ownership:
`batter::runledger::register_in` remains the protected registration path and
`batter::runlimit::http` the protected quota-before-body assembly, while each
native module documents the obligations its callers take on.

The standalone encryption package can be selected directly as `batter-at-rest`,
and the outbound exchange as `batter-outbound-http`, which depends on no Batter
package unless its `operation` feature is selected; the facade always retains
its normal `batter-core` dependency. Cargo unifies
dependency features, so audit the final application's resolved graph. Feature
isolation in this facade is not a security boundary. The outbound client uses
its own *ring* rustls configuration and never a process-default crypto
provider; if the application's graph also compiles aws-lc-rs into rustls, for
example through reqwest's `rustls` feature, crates that build a rustls
configuration without naming a provider need the application root to install a
process default first.

This crate follows the workspace's [Unix-only platform policy](../../README.md#platform-support).
Windows is unsupported and not planned.

From the workspace root:

```sh
cargo test -p batter --locked
cargo run -p batter --example worker
cargo run -p batter --example process_owned
cargo run -p batter --example operation_budget
cargo run -p batter --example finite_command
cargo run -p batter --features outbound-http --example outbound_exchange
```

Version 0.0.1; Rust 1.98.1 minimum; the manifest targets crates.io. MIT licensed,
including the optional `batter-at-rest` dependency.

Select `otlp` for `batter::otlp`: explicit bounded metrics export through
`batter::service::start`. Core owns protected startup through diagnostic completion;
application configuration and exit policy remain at the root. See the
[adapter guide](../batter-otlp/README.md).
