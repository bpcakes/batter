# batter

Batter is the public facade for a native Tokio operational foundation.
`batter-core` owns process lifecycle, startup, finite commands, operation
budgets, retry, admission, health/readiness, cleanup, and telemetry. Optional
adapters own Axum request/server boundaries, PostgreSQL transaction and snapshot
scopes, Runledger lifecycle and atomic enqueue integration, and Runlimit
quota-before-work. Application futures, domain errors, SQLx queries and pools,
and routers remain native.

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

On that canonical path, locally expressible invalid operational states must be
unrepresentable through the public API. A documented ordering, nesting, paired
call, phase transition, nonempty-input rule, cleanup sequence or exhaustive
outcome obligation is not enough when Rust ownership, types or library-owned
assembly can enforce it. Every new or materially changed public API receives the
[ADR-010 invalid-state review](docs/adr/010-agent-only-consumption.md#public-api-invalid-state-review).

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

Version 0.1.0. Publishing is disabled; no registry name has been reserved.
Linux x86_64 and macOS arm64 have execution evidence on Rust 1.94.0 and 1.98.1.
Hosted Linux verification and focused macOS jobs passed on both supported
toolchains in [GitHub Actions run 35580602864](https://github.com/bpcakes/batter/actions/runs/35580602864)
for commit `56814038f2a9cf6a34688ee39cd9f0e433487a1e`. The documentation,
package-description, and rustdoc refresh in this checkout postdates that run.
Live PostgreSQL evidence remains scoped to the separately recorded runs. See
[current status](docs/status.md).

## Platform support

Batter targets Unix backends, including Linux and macOS. **Windows is unsupported,
and there are no plans to support it.** This applies to all workspace packages,
examples, tests and tooling. There are no Windows implementation branches or CI
targets. Other Unix targets remain unverified. See
[ADR-007](docs/adr/007-unix-platform-scope.md).

## Quick start

Network access is required to download dependencies on the first run.

The current unpublished Runledger cutover uses coordinated sibling path
dependencies. Before running a workspace Cargo command in a fresh checkout,
place the pinned Runledger source beside this repository:

```sh
git clone --filter=blob:none --no-checkout https://github.com/bpcakes/runledger.git ../runledger
git -C ../runledger checkout --detach bfc949bbc32fb2cc5731fb743632b2e432d1f5ae
```

This cutover constraint does not make Runledger part of Batter's foundation
dependency graph. Publishing or replacing the paths with immutable package
identities remains a separate decision.

```sh
cargo run -p batter --features axum --example http_service
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
  cargo run -p batter --features sqlx --example owned_pool
cargo run -p batter --features runlimit-memory,runlimit-axum --example quota_service
DATABASE_URL='postgres://user:password@localhost/database' \
  cargo run -p batter-example-postgres-lifecycle --bin postgres_lifecycle
```

The `postgres_lifecycle` example connects to an existing database, probes it
with `SELECT 1`, shows partial-startup cleanup, and registers native pool
closure. It does not create/drop databases or migrate a Runledger schema. Every
`DATABASE_URL` command above must target a local test database; never commit
real connection secrets.

`finite_command` uses `command::Command` to own one native loopback operation and
retain cleanup independently of its waiter.
Service startup is a different ownership path; see
[usage](docs/usage.md#finite-commands-and-owned-cleanup) and the
executable [`Startup` example](crates/batter-core/src/startup.rs).
The eighth facade example, `verification`, requires an explicit migration
manifest and authority policy; see the
[SQLx verifier guide](crates/batter-sqlx/README.md#read-only-schema-and-authority-verification).

## Use as a local dependency

Git or path dependencies only. Every workspace package is `publish = false`. The toml
below assumes this repository is checked out as `batter` beside the consumer:

```toml
[dependencies]
# Public facade; its default graph contains only the native foundation.
batter = { path = "../batter/crates/batter" }
# Select optional toolkit namespaces explicitly, for example:
# batter = { path = "../batter/crates/batter", features = ["axum"] }
# Standalone Rust 1.94 encryption/MAC leaf, also available through feature
# `at-rest` as `batter::at_rest` with identical types.
batter-at-rest = { path = "../batter/crates/batter-at-rest" }
# Direct foundation implementation, when an adapter or focused consumer needs it.
batter-core = { path = "../batter/crates/batter-core" }
# Add this dependency for the HTTP adapter.
batter-axum = { path = "../batter/crates/batter-axum" }
# Add for owned PostgreSQL transaction/snapshot scopes, verification, and
# explicit low-level connection disposition.
batter-sqlx = { path = "../batter/crates/batter-sqlx" }

[dev-dependencies]
batter-test-support = { path = "../batter/crates/batter-test-support" }
```

## Packages

The default `batter` graph does not bring in encryption, Axum, SQLx, or test utilities.
Adapter APIs are also available from their direct packages. The facade exposes
`batter::at_rest`, `batter::axum`, `batter::sqlx`, `batter::runledger`, `batter::runlimit`, and
`batter::test_support` through additive opt-in features; `runlimit-memory`,
`runlimit-postgres`, `runlimit-axum`, and `sqlx-test-support` select only their
documented bridges. Each package declares its own version and Rust
minimum. Runtime/foundation packages currently require Rust 1.94; the standalone
`batter-at-rest` leaf retains an independently verified Rust 1.94 minimum. The
default toolchain is 1.98.1. SQLx 0.9.0 sets the runtime/adapter floor.

| Package | Location | Job |
| --- | --- | --- |
| `batter` | [crates/batter](crates/batter/README.md) | Source-compatible public facade and runnable foundation consumers. |
| `batter-at-rest` | [crates/batter-at-rest](crates/batter-at-rest/README.md) | Synchronous standalone envelope encryption and stable MAC keys; proprietary and not covered by the root MIT license. |
| `batter-core` | [crates/batter-core](crates/batter-core/README.md) | Single native implementation for process ownership, deadlines, retry, admission, cleanup, health/readiness, startup, settings, and telemetry. |
| `batter-axum` | [crates/batter-axum](crates/batter-axum/README.md) | HTTP adapter: request policy, observation, correlation, readiness, browser credential transport, and native serving. |
| `batter-sqlx` | [crates/batter-sqlx](crates/batter-sqlx/README.md) | Owned PostgreSQL transaction and read-only snapshot scopes, schema verification, explicit low-level connection disposition, and opt-in fixtures. |
| `batter-runledger` | [crates/batter-runledger](crates/batter-runledger/README.md) | Native initialization and settlement plus a phase-scoped atomic enqueue runner and schema snapshots built on `batter-sqlx`. |
| `batter-runlimit` | [crates/batter-runlimit](crates/batter-runlimit/README.md) | Optional native atomic quota-before-work execution and protected authenticated HTTP assembly. |
| `batter-test-support` | [crates/batter-test-support](crates/batter-test-support/README.md) | Generic test utilities; independent of the foundation and adapters. |
| `batter-example-postgres-lifecycle` | [examples/postgres-lifecycle](examples/postgres-lifecycle/README.md) | Native SQLx composition; an executable, not a library API. |
| `batter-example-reference-service` | [examples/reference-service](examples/reference-service/README.md) | Atomic authenticated delivery command, provider-effect reconciliation, explicit direct-peer/request correlation, pinned compatibility probes, validated constructors, and an explicit live test target. |

PostgreSQL provisioning stays in the external `postgres-test-harness` repository;
it is not a workspace member. The optional `batter-sqlx/test-support` feature is
selected by reference tests; the default adapter graph excludes the harness.
The optional `batter-runledger` adapter owns native initialization and settlement.
The reference uses its intent-to-queue `run_atomic` API, built on Batter-owned
SQLx scopes, and registers one application-owned provider-effect handler. Its
selected loopback-test protocol uses a stable key, canonical payload matching,
lookup reconciliation and an explicit 24-hour retention boundary; this is not an
exactly-once or arbitrary provider guarantee. `batter-runlimit` preserves native
quota decisions and consumption certainty under an operation budget; optional
HTTP assembly owns auth/quota/body ordering. Its `memory`, `postgres`, and `axum`
features are independent and off by default. It does not own PostgreSQL
initialization or maintenance.
Ownership boundaries
are in [integrations](docs/integrations.md); delivery tasks live in the
[Beads backlog](docs/roadmap.md). The
[compatibility manifest](docs/reference-compatibility.md) records the reference
package's compiled graph and executed live probes. It is not a complete durable
service.

## Non-negotiable limits

- A timeout or cancellation drops a future. It does not roll back an external
  effect or prove a write failed. Nothing here supplies exactly-once effects.
- The supervisor owns registered critical tasks and admitted finite work. A
  finite task initiates drain only by returning `Err(Fatal(error))`; `?` on a
  plain application error does not compile there, and expected business
  rejections belong in the success value. Dropping a receipt does not stop work
  or release its permit.
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
[testing](docs/testing.md) for coverage; [Beads](docs/roadmap.md) for delivery tasks. The
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

MIT; see [LICENSE](LICENSE). Registry publication remains a separate decision.
