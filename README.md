# batter

A small operational foundation for Rust backends built on Tokio, with a separate
Axum adapter and optional SQLx PostgreSQL adapter. Keep your normal futures, application error enums, SQLx pools,
transactions, and routers. Standardize how work is owned, bounded, observed, and
stopped—not how every business operation is written.

**Status: 0.1.0 MVP, locally validated; not a production-validated release.**
The workspace includes failure-contract tests and five runnable demonstrations. Dependencies were upgraded to
the latest stable direct releases on 2026-09-07, including SQLx 0.9.0, with a
Cargo-generated lockfile. See [validation](docs/validation.md) for executed
checks and remaining gaps, and [current status](docs/status.md).

## Platform support

Batter targets Unix backends, including Linux and macOS. **Windows is unsupported,
and there are no plans to support it.** This applies to all workspace packages,
examples, tests and tooling. There are no Windows implementation branches or CI targets.

Linux x86_64 and macOS arm64 have execution evidence on Rust 1.94.0 and 1.98.1.
The updated macOS CI job has not run yet. Other Unix targets remain unverified.
See [ADR-007](docs/adr/007-unix-platform-scope.md) and
[validation](docs/validation.md) for the policy and its tested scope.

## What is implemented

| Module | Responsibility |
| --- | --- |
| `health` | Owned non-overlapping dependency probes and fresh, read-only observations; no probe per request. |
| `startup` | Owned native initialization, cleanup reservations, retained failures and running-driver handoff. |
| `lifecycle` | Startup-acknowledged critical tasks, bounded finite process work, owned shutdown driver, retained reports, drain/cancel/abort/reap and cleanup. |
| `cleanup` | Explicit asynchronous LIFO finalizers, shared/per-hook budgets, panic observation, retained errors, and reported skipped work. |
| `operation` | Total deadlines, explicit finalization reserves, one-way cancellation, typed application failures, child cancellation on scope completion/drop. |
| `retry` | Fresh attempts, explicit replay authorization, capped backoff with optional injected jitter, provider delay lower bounds, retained last error. |
| `admission` | Native Tokio semaphore permits with reject-or-wait policy; waiting uses the existing deadline. |
| `telemetry` | Stable operation outcomes, elapsed time, attempt/task/cleanup events through `tracing`; no global subscriber installation. |
| `batter-axum` | Readiness gate, request deadline/context, configurable sanitized error rendering, independent HTTP observation covering probes/fallback with explicit response severity, combined compatibility middleware. |
| `batter-sqlx` | Default-retiring PostgreSQL leases, bounded probe, redacted native errors and explicit pool-close registration. |
| `batter-test-support` | Scripted dependency results and preservation of both test-body and cleanup errors. |

The virtual workspace has four library packages and two unpublished examples:

| Package | Location | Adoption boundary |
| --- | --- | --- |
| `batter` | [crates/batter](crates/batter/README.md) | Tokio lifecycle, operations, retry, admission, cleanup, and telemetry. |
| `batter-axum` | [crates/batter-axum](crates/batter-axum/README.md) | Add alongside `batter` when using the HTTP adapter. |
| `batter-sqlx` | [crates/batter-sqlx](crates/batter-sqlx/README.md) | Independently selected native PostgreSQL connection disposition. |
| `batter-test-support` | [crates/batter-test-support](crates/batter-test-support/README.md) | Generic test utilities; independent of the foundation and adapters. |
| `batter-example-postgres-lifecycle` | [examples/postgres-lifecycle](examples/postgres-lifecycle/README.md) | Native SQLx composition; an executable, not a library API. |
| `batter-example-reference-service` | [examples/reference-service](examples/reference-service/README.md) | Pinned native SQLx/Runledger/harness compatibility probes; explicit live test target. |

Depending on `batter` does not bring in Axum, SQLx, or test utilities. HTTP APIs
are imported from `batter_axum`; there is no `batter::http`, `axum` feature, or
`postgres-example` feature. Package versions and Rust minimums are declared
individually. PostgreSQL provisioning remains in the external
`postgres-test-harness` repository; it is an external-only test dependency of the
reference example, not a workspace member.
Runlimit, Runledger, and postgres-test-harness adapters are **not implemented**.
Their ownership boundaries are documented in [integrations](docs/integrations.md);
delivery tasks live in the [Beads backlog](docs/roadmap.md). The
[compatibility manifest](docs/reference-compatibility.md) records the reference
package's compiled graph and executed live probes; it does not establish a
complete durable service or reusable adapter.

## Verification

The default toolchain is pinned to Rust 1.98.1. All six packages retain Rust
1.94 as their minimum. SQLx 0.9.0 requires it in the adapter and examples; extracting
that dependency does not establish a lower minimum for the libraries.
The Linux CI definition covers 1.94.0, 1.98.1, and current stable; the focused
macOS job compiles all workspace targets and runs the subprocess tests on both
1.94.0 and 1.98.1.
Network access is required to download dependencies on the first run.

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
```

Checks preserve the resolved dependency graph in `Cargo.lock`. Use
`bash scripts/verify.sh --bootstrap` only when formatting sources and generating
a missing lockfile is intended. Compiler/lint/test failures must be fixed
without weakening the documented contracts.

## Run the examples

```sh
cargo run -p batter-axum --example http_service
# In a second terminal:
curl -i http://127.0.0.1:3000/live
curl -i http://127.0.0.1:3000/ready
curl -i http://127.0.0.1:3000/work
```

`BATTER_BIND` defaults to `127.0.0.1:3000`; `BATTER_REQUEST_TIMEOUT_MS` defaults to
2000. `/work` performs a simulated 25 ms read under a concurrency bound. This is
an integration example, not a business API. `/fail` demonstrates the same application
error envelope used by middleware failures, with a server-generated request ID.
SIGINT and SIGTERM trigger shutdown through native Unix signal listeners.

```sh
cargo run -p batter --example worker
cargo run -p batter --example process_owned
cargo run -p batter --example operation_budget
DATABASE_URL='postgres://user:password@localhost/database' \
  cargo run -p batter-example-postgres-lifecycle --bin postgres_lifecycle
```

The PostgreSQL example connects to an existing database, probes it with `SELECT
1`, demonstrates partial-startup cleanup, and registers native pool closure.
It does not create/drop databases or migrate a Runledger schema. Use only a local
test database for initial verification; never commit real connection secrets.

## Use as a local dependency

Every workspace package is intentionally `publish = false`; no registry name has
been reserved and no publishing action has been taken.

```toml
[dependencies]
batter = { path = "../batter/crates/batter" }
# Add this dependency for the HTTP adapter.
batter-axum = { path = "../batter/crates/batter-axum" }
# Add for explicit SQLx PostgreSQL connection disposition.
batter-sqlx = { path = "../batter/crates/batter-sqlx" }

[dev-dependencies]
batter-test-support = { path = "../batter/crates/batter-test-support" }
```

### Bound an operation without erasing its application error

```rust
use batter::operation::OperationContext;
use std::time::Duration;

async fn read_count() -> Result<u64, std::io::Error> {
    Ok(42) // Replace with your dependency; preserve its concrete error type.
}

async fn example() -> Result<(), Box<dyn std::error::Error>> {
    let request = OperationContext::new(Duration::from_secs(2))?;
    let count = request.run("accounts.count", |_scope| read_count()).await?;
    assert_eq!(count, 42);
    Ok(())
}
```

`run` receives a **factory**, creates a child context, and cancels that child when
execution completes or its future is dropped. It does not join tasks spawned by
the callback. Compose request-local futures normally; do not turn them into
unowned `tokio::spawn` calls. See [usage](docs/usage.md) for retries and ownership.

## Non-negotiable limits

A timeout does not prove a write failed. Cancellation drops a future and may
leave an external outcome unknown. Nothing here supplies exactly-once effects.

The supervisor owns registered critical tasks and explicitly admitted finite work.
Finite task `Err(E)` initiates drain; put expected business rejections in the
success value instead. Dropping a receipt does not stop work or release its permit.
The owned driver continues shutdown when a waiter is cancelled, provided the
runtime remains alive. Task abortion is not preemption,
and joining a server wrapper does not prove secretly detached children stopped.
After a panic, requested abort, or unjoined direct task, dependent finalizers are
conservatively skipped and the report is unsuccessful.

Cleanup is explicitly driven—not asynchronous `Drop`, not general cancellation shielding, and
not a promise to survive SIGKILL. The Axum boundary ends when a response is
constructed: streaming bodies and WebSockets require a separate lifetime design.

These are API contracts and limitations, not footnotes. Read
[guarantees](docs/guarantees.md) before putting side effects behind a boundary.

## Documentation and handoff

Start with [status](docs/status.md), [architecture](docs/architecture.md), and
[guarantees](docs/guarantees.md). The [Effect v4 brief](docs/effect-v4-brief.md)
preserves the design rationale, including topics not implemented here.
The [reconciliation](docs/effect-v4-reconciliation.md) maps the broader conventions
proposal to implemented APIs, deliberate differences, and remaining delivery work.

[AGENTS.md](AGENTS.md) gives the next agent a reading order, exact verification
commands, dependency boundaries, and change rules. The [backlog navigation page](docs/roadmap.md)
points to Beads, the sole owner of delivery tasks and their dependencies. See also
[testing](docs/testing.md), [operations](docs/operations.md),
[security](SECURITY.md), and [primary references](docs/references.md).

## License

MIT; see [LICENSE](LICENSE). Review ownership and publication policy before
turning this internal MVP into a public release.
