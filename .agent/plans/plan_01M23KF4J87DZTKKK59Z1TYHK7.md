# Explicit PostgreSQL connection disposition

This living ExecPlan follows `.agent/PLANS.md`; Bead `batter-7r3.2` owns scope.

## Purpose / Big Picture

An optional `batter-sqlx` package lets native SQLx callers retire an unsuccessful
connection without leaving its pool slot waiting behind interrupted SQL. A live
one-slot-pool regression must show independent work and local pool close while
the original backend remains blocked, then observe that backend disappear after
unlocking. This does not acknowledge remote cancellation or prove rollback.

## Progress

- [x] (2026-09-09) Read Bead, repository contracts and SQLx 0.9 pool source.
- [x] (2026-09-09) Implement explicit lease, redacted native errors, bounded probe and cleanup registration.
- [x] (2026-09-09) Add offline and separately invoked ignored live failure-contract tests.
- [x] (2026-09-09) Adopt in the native example and update package/contract/reference documentation.
- [x] (2026-09-09) Execute ten live cases, both Rust verification profiles and all HTTP/PostgreSQL smokes on both toolchains.
- [x] (2026-09-09) Execute final Jig gates: all five targets passed fresh; api:test receipt_01M23MPBGE3RJ254JFNWQQXKHQ.
- [x] (2026-09-09) Audit all Bead acceptance against source and executed two-toolchain/live evidence.
- [x] (2026-09-09) Close Bead after acceptance audit; record final Jig check/finish commands without committing.

## Surprises & Discoveries

SQLx 0.9 PoolConnection Drop spawns return work that pings before releasing the
slot. `detach` releases pool accounting synchronously; `close` retains the permit.
No PostgreSQL connection environment was configured. The existing local PostgreSQL 18.6 server accepts Unix role aa via /var/run/postgresql; explicit SQLx username is required. No database was provisioned. The ordinary-return control needed its cancelled close re-driven after unlock before observing server disappearance; final cases pass.
The worktree initially has an existing `.beads/issues.jsonl` modification.

## Decision Log

Use a default-retiring lease with an explicit success return method. Expose
`&mut PgConnection` only in this optional adapter, so native transactions and
application errors remain visible without a boxed callback or transaction manager.
The caller must await successful query/commit/rollback before opting into pool
return. Unwinding/dropping retires automatically, but no async cleanup is claimed.
Setup database creation is optional in the Bead and is excluded: there is no
hidden provisioning or reconciliation API to validate in this focused extraction.

## Outcomes & Retrospective

The optional adapter, four offline tests, compiling transaction example, existing runnable consumer, and ten ignored live cases are implemented. Final Linux verification and example/live checks pass on Rust 1.98.1 and 1.94.0; docs/validation.md records commands and limits. The complete acceptance audit passed and the Bead is closed. Jig gates passed; recording completion evidence changed their input digests, so refresh the final profile and finish the plan without further source/documentation edits. Jig state retains the authoritative final receipt and closure. macOS, hosted CI and server-side bounds are not established.

## Context and Orientation

The virtual Cargo workspace contains core, Axum and generic test-support
libraries plus `examples/postgres-lifecycle`. New files go under
`crates/batter-sqlx`, including the nearest AGENTS.md. Core and test support must
retain no SQLx dependency. The existing example owns its startup cleanup policy.

## Plan of Work

Add the package and shared workspace dependency. Implement a lease holding an
Option<PoolConnection<Postgres>>; its Drop takes, detaches and drops the native
connection. Only explicit return removes it for ordinary SQLx pool return.
Bound acquisition and SELECT 1 with the supplied OperationContext. Preserve the
native SQLx error under a wrapper with fixed Debug/Display and a conservative
category that grants no replay permission. Register Pool::close through Supervisor.
Adapt the existing example without changing its signal/startup ownership.

Create ignored live tests and `scripts/test_sqlx_live.sh`; missing configuration
must fail the explicit invocation. Use externally supplied DATABASE_URL, a native
observer/blocker connection and advisory locks to avoid schema changes. Capture
each backend PID, wait for pg_stat_activity lock evidence, interrupt the client,
verify replacement and close before releasing the blocker, then wait for every
retired PID to disappear. Cover error, panic, deadline, cancellation, outer drop,
repeated interruptions, success and acknowledged transactions. Always unlock
and observe cleanup on failure before reporting combined errors.

## Concrete Steps

Run from `/home/aa/Documents/batter`: `cargo test -p batter-sqlx --locked`,
`bash scripts/test_sqlx_live.sh`, `bash scripts/verify.sh`, and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`. Build the HTTP example on both
toolchains and execute scripts/smoke_http.py with default, --signal SIGINT,
--deadline, --warn-filter, and --warn-filter --deadline profiles. Run Jig work
check, evidence and gates for this plan; finish only with successful current
api:test evidence and all explicit task contracts proven.

## Validation and Acceptance

Ordinary all-target/all-feature tests compile live cases but leave them ignored.
The explicit runner verifies inventory and executes every live case. One-slot
replacement and bounded local close must precede unlock; independent backend
disappearance must follow unlock. Reuse must retain backend identity. Diagnostics
must hide sentinel secrets while native causes stay inspectable. Inspect Cargo
metadata/tree to establish the optional dependency boundary and locked 0.9 graph.

## Idempotence and Recovery

Use session advisory locks and temporary tables only. The external database is
never created, migrated or dropped. Release locks and await session disappearance
before fixture cleanup; record both test and cleanup failures. Preserve unrelated
worktree changes. Do not commit, publish or deploy.

## Interfaces and Dependencies

Use SQLx 0.9 PostgreSQL/runtime-tokio, Rust 1.94 minimum and existing Batter APIs.
Public PgLease acquisition accepts &PgPool and &OperationContext; connection
borrowing returns &mut PgConnection. Explicit return opts into native pool return;
all other destruction detaches. SqlxFailure owns sqlx::Error, exposes trusted
inspection and conservative classification. probe and register_pool_close compose
the lease and existing operation/lifecycle boundaries.

Revision 2026-09-09: recorded the implemented lease seam, external fixture, corrected control cleanup ordering and completed two-toolchain evidence. Final Jig gating remains.

Revision 2026-09-09: all required source, live PostgreSQL, two-toolchain, example and Jig evidence is complete. Setup creation is deliberately absent; macOS/hosted execution and remote guarantees remain unclaimed.

Final administrative commands: `scripts/jig work check --plan-id plan_01M23KF4J87DZTKKK59Z1TYHK7 --json`, inspect `work evidence` and `work gates`, then `scripts/jig work finish --plan-id plan_01M23KF4J87DZTKKK59Z1TYHK7 --resolution "Completed native PostgreSQL disposition with two-toolchain live evidence" --outcome success`. No source or documentation changes follow the final profile.
