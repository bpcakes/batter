# Fail-fast atomic SQL scopes — batter-wobk

Requested item 3, stacked on failure-policy PR 13 (b4d195c). Deliver this as a
separate PR, leaving native-query helpers for item 4. No migrations or dependency
changes. Generic names only; Unix scope unchanged.

## Progress

- Executed PostgreSQL 18.6 rollback-status counterexamples and selected one
  transaction guard instead of relying solely on transaction-status inspection.
- Implemented low-level fast operations, policy-mode dispatch, shared completion,
  known-rollback evidence, budgeted variants and native Runledger entry point.
- Eight focused failure/statement-count live tests and scoped Clippy pass.
  Native consumer tests passed in both modes. Independent invalid-state review
  found and verified the cancellation repair below; no material findings remain.
  Contracts/examples and full verification are complete; separate PR delivery follows.

## Surprises & Discoveries

A failed transaction cannot run the original-XID validation query. A plain
ROLLBACK can acknowledge a different transaction. On a disposable PostgreSQL18.6
cluster, BEGIN/capture-XID/COMMIT/BEGIN/division-error/ROLLBACK reported original
pg_xact_status = committed. More subtly, BEGIN/capture-XID/ROLLBACK followed by a
committed replacement write and another failed transaction reported original
status = aborted while the replacement write remained committed. Thus original
status alone cannot preserve the existing boundary-loss behavior. PostgreSQL18
references: functions-info.html#FUNCTIONS-PG-SNAPSHOT, sql-rollback-to.html and
sql-release-savepoint.html, read 2026-09-22 UTC. Record these in docs/references.md.

## Decision Log

One private savepoint at transaction birth permits rollback and XID validation
after failed SQL. Successful fast operations have no per-operation savepoint:
body plus validation, two statements for a one-query unprofiled call. Profiled
calls retain external-catalog opening validation and use three statements. Setup
and final completion are excluded. Guard rollback/release plus validated whole
ROLLBACK precedes ordinary rejection. Validation/boundary/transport loss remains
explicit uncertainty; no universal rollback promise is made for broken sessions.

Keep one completion implementation in PgAtomicScope::finish and its existing
Live/InFlight/Poisoned ownership state. Private FastScope wraps that scope with
the birth guard and optional acknowledged rollback evidence. Closed state is
checked before further work or completion; it cannot be interpreted as abandoned
work. Required PgFailurePolicy methods remain unchanged. A fast runner requires
Error: From<PgScopeRolledBack> so a callback that catches a rejection cannot
return success or invoke more work. The original E is returned without Clone;
if the callback deliberately discards it, the library retains rollback evidence,
not a duplicate of that application value. Uncertain outcomes still retain the
body result and first native loss cause. No new variant broadens existing public
uncertainty/error enums.

The selected behavior lives in private policy-scope dispatch; consumers cannot
switch it after construction. Explicit recoverable_sql uses the existing
operation savepoint and cannot resurrect a closed fast scope. Application-created
savepoints may persist between successful fast operations; documentation must
state this difference from recoverable scope confinement. Arbitrary SQL remains
a trusted escape hatch, not a sandbox. Existing enum-returning APIs stay intact.
The newly introduced policy scope types require the actual failure-policy bound
on their declarations, matching their only constructors and native wrappers.

Independent contract review found that a caught acknowledged rollback could be
lost if the callback then stalled until budget cancellation. Retain the opaque
proof in a shared slot before returning the rejection and resolve fallback inside
the existing pre-telemetry boundary. A completed body result retains precedence.
Add a live profiled/unprofiled cancellation regression. The first full verification
was deliberately stopped before completion to repair this confirmed issue.

## Outcomes & Retrospective

Delivered behavior is covered by eight added live regressions, native phase
consumers in both modes and compile-fail examples. On macOS arm64 with PostgreSQL
18.6, Rust 1.98.1 and 1.94.0 each passed the full verify.sh command, exact 113-case
SQLx live inventory and all five rebuilt HTTP smokes. Jig run
run_01M35NN5CDX7YV1Z8EH16Z9FXH passed every required target; api:test receipt is
receipt_01M35P4NHNZAPBC4KN2WZ1AFH2. No dependency, lockfile or migration changes.

The first 1.98.1 live suite reported cleanup failure in the existing
verification_late_ledger_attachment_cannot_supply_snapshot_rows case. The focused
case and full113-case rerun passed unchanged; the original cause was not established
beyond its retained cleanup diagnostic. An initial parallel focused invocation
also missed a tracing statement count; the exact runner serializes these controls,
and the unchanged serial tests passed. Clippy's oversized-test finding was repaired
by extracting the observer assertion, without changing acceptance assertions.
The full verification attempt started before independent review was stopped to
repair the confirmed cancellation-retention defect; only final-tree passes count.

Final logs: /tmp/batter-wobk-verify-{1981,1940}-final.log,
/tmp/batter-wobk-live-{1981,1940}-final.log,
/tmp/batter-wobk-http-{1981,1940}.log and /tmp/batter-wobk-jig.log.

ADR-010 assessment: policy selection and private ownership prevent replacing a
connection, switching execution mode or constructing rollback evidence. A caught
runtime rejection leaves the borrowed callback scope closed; later calls compile
but cannot invoke work or commit. Consuming scope-return tuples would change the
requested fixed Result<T,E> callback shape, so library-owned execution enforces
this dynamic condition. Native phase transitions remain consuming. Deliberately
discarding application errors, external effects and arbitrary SQL meaning remain
application policy, not transaction proofs.

## Work and validation

Source: crates/batter-sqlx/src/atomic/fast.rs, atomic_runner.rs and
atomic_policy/{fast,context}.rs; native phase wrapper in
runledger/runledger-postgres/src/atomic/policy.rs and adapter reexports.

Add live tests for exact success counts, full rollback of prior/current writes,
caught rejection refusing SQL and outer success, explicit recovery, nested user
savepoints, committed/replaced and aborted/replaced boundaries, cleanup failure,
profile drift, cancellation/abandonment and native intent/queue composition.
Add compile-fail evidence for forged rollback evidence and missing conversion;
examples must compile with native question-mark and fixed error inference.
Update exact live inventory and relevant package/guarantee/status/guide contracts.

Run focused tests and clippy before full verification. Then run both verify.sh
commands (Rust1.98.1 and1.94.0), exact SQLx live suite against disposable PG18.6,
build and execute all five HTTP smokes each toolchain, and required Jig gates.
Inspect work evidence/gates first; do not edit tracked inputs during work check.
Record platform/version/counts and any failed attempts without weakening tests.

## Recovery

No persisted changes. Revert this branch to remove the new mode; old runners
remain available. Cancellation and failed cleanup retire the physical connection,
never return it as reusable. Push a separate PR; do not merge or publish crates.
