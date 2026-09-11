# Retain fixture failures and unresolved database ownership

This living ExecPlan follows `.agent/PLANS.md`. Bead `batter-kjl` owns scope and acceptance. Baseline: `486e0b0f9f4c4439077418715843b30042205f7e`.

## Purpose

Extend the existing adapter fixture driver so callers can bound observation without dropping database leases, and can require independent PostgreSQL session absence before deletion. Preserve every body, producer, observation, consuming cleanup and deferred-drain cause. No new fixture framework or provisioning engine is needed.

## Progress

- [x] (2026-09-10) Audited Bead, guides, existing runner and native dependency sources; claimed Bead.
- [x] (2026-09-10) Implemented resumable bounded waits, progress, opt-in session observation/retry and retained pool/observation failures.
- [x] (2026-09-10) All 23 live PostgreSQL cases passed, including native detach and adapter retirement, repeated observer failure/recovery, distinct deferred/consuming errors, waiter/runtime loss, handled pool errors and Script/finish failure retention.
- [x] (2026-09-10) Updated contracts/references/status/validation and Bead audit; both toolchain matrices and ten HTTP smokes passed.
- [x] (2026-09-10) Final Jig verify profile passed all five targets; api:test receipt receipt_01M25Z7JQEMS291NY5D39KW613 is fresh.
- [x] (2026-09-10) Bead closed/exported; final metadata verify profile passed all five targets. Final closure is recorded by jig work finish in the append-only plan records.

## Surprises & Discoveries

`batter-4jz` already supplies the owned driver, partial acquisition, independent concurrent cleanup, body panics and dual native body/consuming cleanup errors. SQLx 0.9 Pool::close returns unit and excludes detached connections. The pinned harness 3d525e6 consumes a lease for awaited cleanup, while explicit defer/Drop have separate failure delivery via drain. Dropping the existing FixtureRun detaches its driver; actual runtime/driver destruction can drop leases destructively.

## Decision Log

Reuse FixtureSuite::start. Add an opt-in SessionObserver backed by a caller-budgeted admin pool, configured before acquisition. Query all sessions for the disposable database after all tracked pools close; this includes retired backend identities without PID-reuse false absence. Callers must stop producers of new connections. A failed bounded observation retains its concrete error and parks that database cleanup until explicit observer retry, avoiding unbounded automatic error accumulation. Other databases keep cleaning. A final report remains unsuccessful after a recovered observation error. Ordinary start remains compatible and makes no session-quiescence claim. Manual DatabaseFixture::finish remains caller-driven.

## Outcomes & Retrospective

Implementation and 23-case live execution complete; both full toolchain matrices and all ten HTTP smokes passed; Jig verify profile passed all five targets with fresh evidence. All implementation, documentation, live and offline acceptance criteria are satisfied. The dedicated Docker PostgreSQL 18 endpoint was 127.0.0.1:62202, container batter-kjl-validation, created only for this task and removed after both live runs. No commit, publication or deployment authorized.

## Context and Orientation

`crates/batter-sqlx/src/test_support/runner.rs` retains body and native producers, then concurrently disposes registered fixtures and drains the harness. `report.rs` preserves each failure branch. A lease is the upstream database deletion owner; pool clones are access handles. `examples/reference-service/tests/support/fixture_run.rs` is the minimal consumer and already uses generic finish. `scripts/reference_live.py` enforces live case inventory and an external wall-clock watchdog. Generic batter-test-support remains unchanged and independent.

## Plan of Work

First add a session module with SessionObserver configuration/retry, per-database cleanup phase and retained native observation failures. Register the observer with the suite before running the body. Close all pools before querying pg_stat_activity from the independent admin pool; require the database to be present and observer database distinct. A timeout/error must not consume or drop the lease. Add FixtureRun::wait_for and progress inspection; retain native JoinError separately. Add failures to DatabaseCleanup and report success/source/redaction logic.

Then extend the real reference probes. Hold checked-out connections through bounded waits, and hold detached backend operations blocked by acknowledged advisory locks. Observe their exact backend IDs outside the one-slot closing pool, verify database retention and independent cleanup, release the lock and retry the same observer/driver to finish. Force a real observer failure and repair it. Fault consuming and deferred cleanup using distinct leases under the existing acknowledged catalog lock; recover residual databases via upstream stale cleanup only after resources are released. Test waiter drop with a live runtime and runtime destruction separately, naming which work remains driven; include driver JoinError controls without fabricating SQLx errors. Reuse Script/finish for returned assertion and verification failures.

## Concrete Steps

From repository root, run focused `cargo test -p batter-sqlx --features test-support --locked` and `bash scripts/test_reference_live.sh` with POSTGRES_TEST_ADMIN_URL explicitly selecting an exclusive local disposable PostgreSQL 18 endpoint. Missing prerequisites fail. Keep fault-injection cases serial and under the existing process watchdog. Register every new live case in both Rust and Python inventory.

## Validation and Acceptance

Require the original eight Bead criteria: held checkout retention/resumed wait; distinct timeout/cancel/join outcomes; preserved real independent errors; continued independent cleanup and separate drain; bounded assertion/panic/partial acquisition/owner loss; native detach negative control with separately budgeted observer and retained database on observation failure; shared consumer adoption and generic Script/finish; no reverse dependency or application-router scope creep.

Run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`, then rebuild `cargo build -p batter-axum --example http_service --locked` and run `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` in default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and `--warn-filter --deadline` modes. Record platform, exact toolchains, lock hash, outcomes and limitations in docs/validation.md. Inspect `scripts/jig work evidence` and `scripts/jig work gates`, then run `scripts/jig work check` for a fresh final api:test receipt and applicable gates before finish.

## Idempotence and Recovery

Do not alter shared database roles or existing migrations. Use uniquely named upstream disposable leases. Release injected locks, close native clients and await all available cleanup before checking assertions. Failed consuming cleanup requires owner-aware stale cleanup after harness release; it is not retried through an already consumed lease. A parked observer requires repair and explicit retry while its runtime is live. Runtime destruction has no cleanup completion guarantee. No commits without a user request.

## Interfaces and Dependencies

Keep SQLx 0.9.0 and postgres-test-harness revision 3d525e6 unchanged. Public additions need rustdoc examples. SessionObserver owns caller-supplied observation policy, not provisioning. FixtureRun owns only observation handles for the existing driver. Progress is historical evidence, never clean reuse permission. Update docs/guarantees.md, docs/integrations.md, docs/testing.md, docs/status.md, docs/references.md and package guide as appropriate.

Revision 2026-09-10: audit found handled pool errors were lost. Added shared native cause retention in DatabaseCleanup, updated owned PoolAcquire payload and all workspace source consumers, and documented this explicit source compatibility adjustment. The tests also repair a wrong observer database after native PoolClosed, preserving both attempts.

Verification evidence: each toolchain passed 650 Rust test/doctest executions, 73 summaries and 36 intentional ignores. Live inventory has 23 passing cases on each toolchain; the dedicated server is PostgreSQL 18.4 Linux aarch64 with macOS clients. Exact commands and limitations are recorded in docs/validation.md.


## Completion audit

Held-checkout and cancelled-wait criteria are covered by fixture_close_order_and_resumable_wait: both native pools start closing, the blocked database persists, the independent database disappears, and the same run completes after release. fixture_runtime_loss_exposes_native_drop observes a real cancelled driver JoinError repeatedly, distinct from the bounded wait returning pending. SQLx close returns unit; no close error is fabricated.

Failure retention and independent cleanup are covered by fixture_distinct_deferred_and_consuming_failures and the original consuming failure control. The new control uses distinct native leases, a real catalog lock, SQLSTATE 22012, native consuming error, per-database DeferredCleanup source, external no-op shutdown, explicit drain and tagged-resource recovery. The detached-session control witnesses independent cleanup while another database is parked.

The bounded assertion, panic, partial acquisition and loss cases are present in the 23-case watchdog inventory. fixture_assertion_and_script_failures_retained combines returned assertion and actual Script exhaustion using existing finish. fixture_partial_acquisition_and_panic now enables session observation too. fixture_waiter_loss_keeps_cleanup_driven proves live driver retention; runtime destruction proves native Drop can delete despite a held checkout, without calling that successful fixture cleanup.

fixture_detached_sessions_retained runs native PoolConnection::detach and actual PgLease retirement separately, with acknowledged advisory-lock waits, exact backend IDs and independently budgeted one-slot admin observation after native pool close. fixture_session_observer_failure_resumes retains both real PoolClosed and wrong-target errors and the original Arc through explicit repaired retries. Reports remain unsuccessful after recovered observation errors. The shared minimal fixture uses this adapter-owned support; generic batter-test-support source/manifest are unchanged and no harness/foundation dependency was introduced there. No application-router, provider or external-adoption scope was pulled in.

The remaining governing deliverables are present: public rustdoc/example, contract/status/reference/validation updates, owning Bead audit comment, both complete toolchain scripts, 23 live cases on each toolchain, ten HTTP smokes and fresh Jig api:test/profile evidence. Initial standalone Jig test evidence was invalidated only by a documentation edit during execution; the subsequent unchanged-worktree profile passed. No semantic test or gate was relaxed.

Final metadata refresh: the verify profile passed all five targets again after the completion notes and Bead export, with no code changes, no test failures, and no policy waivers. The task-owned database container is absent.
