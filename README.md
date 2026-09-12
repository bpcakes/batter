# batter

A small operational foundation for Rust backends built on Tokio, with a separate
Axum adapter and optional SQLx PostgreSQL adapter. Keep your normal futures,
application error enums, SQLx pools, transactions, and routers. Standardize how
work is owned, bounded, observed, and stopped—not how every business operation
is written.

It is for Tokio services that need owned shutdown, deadlines, and cleanup. It is
not a web framework, DI container, or a replacement for application types.

## For coding-agent consumers

Batter's integration APIs are designed for autonomous coding agents. Prefer the
canonical, library-driven path so operational invariants follow from ownership,
constrained interfaces, validated configuration, and executable checks. Repeated
caller obligations to coordinate cancellation, joining, cleanup, deadline
relationships, registration, or error retention are design debt, even when
documented. Keep application-specific protocols in the application or a
supported adapter; agent-only consumption does not call for an opaque DSL,
extra abstraction layers, or claims that types prove arbitrary remote effects.

Examples are consumer contracts. When a lower-level escape hatch is necessary,
its documentation must state the obligations it leaves with the caller and must
not present it as equivalent to the protected path. Assess proposed changes with
independent failure scenarios and fresh-agent implementation or modification
tasks; clean reviews or test volume alone do not establish agent usability.
Those evaluations are proposed and unexecuted unless this repository records
specific evidence. See [ADR-010](docs/adr/010-agent-only-consumption.md),
[architecture](docs/architecture.md#agent-only-consumption), and
[usage](docs/usage.md#canonical-agent-consumer-path).

Recurring invariant failures during example review require the implementation
agent to [assess the consumed API](docs/adr/010-agent-only-consumption.md#recurring-example-review-defects)
and report the design concern before continuing dependent repairs.

## Status

**0.1.0 MVP, locally validated; not a production-validated release.** Publishing
is disabled; no registry name has been reserved. Linux x86_64 and macOS arm64
have execution evidence on Rust 1.94.0 and 1.98.1. The updated macOS CI job has
not run. See [current status](docs/status.md) and [validation](docs/validation.md).

## Platform support

Batter targets Unix backends, including Linux and macOS. **Windows is unsupported,
and there are no plans to support it.** This applies to all workspace packages,
examples, tests and tooling. There are no Windows implementation branches or CI
targets. Other Unix targets remain unverified. See
[ADR-007](docs/adr/007-unix-platform-scope.md).

## Quick start

Network access is required to download dependencies on the first run.

```sh
cargo run -p batter-axum --example http_service
# In a second terminal:
curl -i http://127.0.0.1:3000/live
curl -i http://127.0.0.1:3000/ready
curl -i http://127.0.0.1:3000/work
```

`BATTER_BIND` defaults to `127.0.0.1:3000`; `BATTER_REQUEST_TIMEOUT_MS` defaults to
2000; `BATTER_BULKHEAD_CAPACITY` defaults to 32. `BATTER_ENV_FILE` may name one
literal dotenv file; there is no default `.env` search. Invalid values fail before
bind. See [operations](docs/operations.md#loading-example-settings). This is an
integration example, not a business API: `/work` simulates a 25 ms read under a
concurrency bound, and `/fail` uses the same application error envelope as
middleware failures. SIGINT and SIGTERM trigger shutdown through native Unix
signal listeners.

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

### Other examples

```sh
cargo run -p batter --example worker
cargo run -p batter --example process_owned
cargo run -p batter --example operation_budget
cargo run -p batter --example finite_command
DATABASE_URL='postgres://user:password@localhost/database' \
  cargo run -p batter-example-postgres-lifecycle --bin postgres_lifecycle
```

The PostgreSQL example connects to an existing database, probes it with `SELECT
1`, demonstrates partial-startup cleanup, and registers native pool closure. It
does not create/drop databases or migrate a Runledger schema. Use only a local
test database; never commit real connection secrets.

`finite_command` uses `command::Command` to own one native loopback operation and
retain cleanup independently of its waiter.
Service startup is a different ownership path; see
[usage](docs/usage.md#finite-commands-and-owned-cleanup) and the
executable [`Startup` example](crates/batter/src/startup.rs).

## Use as a local dependency

Path dependencies only. Every workspace package is `publish = false`. The toml
below assumes this repository is checked out as `batter` beside the consumer:

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

## Packages

Depending on `batter` does not bring in Axum, SQLx, or test utilities. HTTP APIs
are imported from `batter_axum`; there is no `batter::http`, `axum` feature, or
`postgres-example` feature. Each package declares its own version and Rust
minimum (currently 0.1.0 and 1.94). The default toolchain is 1.98.1. SQLx 0.9.0
sets that floor in the adapter and examples; extracting it does not establish a
lower library minimum.

| Package | Location | Job |
| --- | --- | --- |
| `batter` | [crates/batter](crates/batter/README.md) | Process ownership, deadlines, retry, admission, cleanup, health, startup, settings, and telemetry. |
| `batter-axum` | [crates/batter-axum](crates/batter-axum/README.md) | HTTP adapter: request policy, observation, correlation, readiness, and native serving. |
| `batter-sqlx` | [crates/batter-sqlx](crates/batter-sqlx/README.md) | Optional native PostgreSQL connection disposition. |
| `batter-runledger` | [crates/batter-runledger](crates/batter-runledger/README.md) | Optional native initialization, stop-clock and settlement integration. |
| `batter-test-support` | [crates/batter-test-support](crates/batter-test-support/README.md) | Generic test utilities; independent of the foundation and adapters. |
| `batter-example-postgres-lifecycle` | [examples/postgres-lifecycle](examples/postgres-lifecycle/README.md) | Native SQLx composition; an executable, not a library API. |
| `batter-example-reference-service` | [examples/reference-service](examples/reference-service/README.md) | Atomic authenticated delivery command, pinned compatibility probes, validated constructors, and an explicit live test target. |

PostgreSQL provisioning stays in the external `postgres-test-harness` repository;
it is not a workspace member. The optional `batter-sqlx/test-support` feature is
selected by reference tests; the default adapter graph excludes the harness.
The optional `batter-runledger` adapter owns native initialization and settlement.
The reference starts the native runtime with an empty handler registry and uses
Runledger's transactional producer API. The delivery handler/provider and a
Runlimit adapter remain unimplemented; application readiness stays unapproved.
Ownership boundaries
are in [integrations](docs/integrations.md); delivery tasks live in the
[Beads backlog](docs/roadmap.md). The
[compatibility manifest](docs/reference-compatibility.md) records the reference
package's compiled graph and executed live probes. It is not a complete durable
service.

## Non-negotiable limits

- A timeout or cancellation drops a future. It does not roll back an external
  effect or prove a write failed. Nothing here supplies exactly-once effects.
- The supervisor owns registered critical tasks and admitted finite work. Task
  `Err(E)` initiates drain; put expected business rejections in the success
  value. Dropping a receipt does not stop work or release its permit.
- Task abortion is not preemption. Joining a server wrapper does not prove
  detached children stopped. After a panic, requested abort, or unjoined direct
  task, dependent finalizers are skipped and the report is unsuccessful.
- Cleanup is explicitly awaited. It is not asynchronous `Drop`, general
  cancellation shielding, or a promise to survive SIGKILL. The owned driver
  continues shutdown when a waiter is cancelled, provided the runtime remains
  alive.
- The Axum boundary ends when a response is constructed. Streaming bodies and
  WebSockets require a separate lifetime design.

These are API contracts and limitations, not footnotes. Read
[guarantees](docs/guarantees.md) before putting side effects behind a boundary.

## Documentation

To use batter: [usage](docs/usage.md), [operations](docs/operations.md),
[architecture](docs/architecture.md), [guarantees](docs/guarantees.md), and
[security](SECURITY.md). Crate READMEs in the table above own adapter-level
detail.

To work in this repository: [AGENTS.md](AGENTS.md) for reading order,
verification, and change rules; [status](docs/status.md) and
[testing](docs/testing.md) for coverage; [validation](docs/validation.md) for
executed evidence; [Beads](docs/roadmap.md) for delivery tasks. The
[Effect v4 brief](docs/effect-v4-brief.md) and
[reconciliation](docs/effect-v4-reconciliation.md) record design rationale.
[Primary references](docs/references.md) record upstream checks.

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
```

Checks preserve `Cargo.lock`. Use `bash scripts/verify.sh --bootstrap` only when
formatting sources and generating a missing lockfile is intended. Repair
compiler, lint, and test failures without weakening the documented contracts.

## License

MIT; see [LICENSE](LICENSE). Review ownership and publication policy before
turning this internal MVP into a public release.
