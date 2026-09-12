# Publish native SQLx pool cleanup before fallible initialization


This living ExecPlan implements Bead `batter-lp2.3` under epic `batter-lp2` and
follows `.agent/PLANS.md`. Beads owns delivery scope and acceptance; this plan
owns execution, decisions and evidence. Maintain the four living sections below.
Plan preparation does not implement the API, provision a database, claim the task
or authorize commits, publication or deployment.

## Purpose / Big Picture


An application agent should create a native PostgreSQL pool only after cleanup
registration has been validated, without manually closing the pool if later
registration fails. After this change it can reserve a cleanup slot and call
`batter_sqlx::pool_in(slot, options, connection)`. That synchronous call returns a
native PgPool only after publishing its awaited close hook. A query, startup
failure, cancellation or panic can then be handled by the existing command or
startup owner, which retains work failures and complete cleanup outcomes together.

## Progress


- [x] (2026-09-12 13:25Z) Prepared plan from current adapter, native-source research
  and the reviewed Bead; no implementation or live validation has started.
- [x] (2026-09-12 14:58Z) Recorded HEAD `39c1b6e`, claimed the Bead, and
  selected the existing external PostgreSQL 18.6 Unix-socket database without
  provisioning or mutation.
- [x] (2026-09-12 15:21Z) Added synchronous `pool_in`, ownership rustdoc, the
  adapter-owned Command example, and offline compatibility/reservation checks.
- [x] (2026-09-12 15:42Z) Added real-query Command and legacy Startup ownership
  cases with exact cleanup records and closed-pool inspection.
- [x] (2026-09-12 15:55Z) Added cancellation, native/application failures,
  panic, LIFO, held-checkout and negative-oracle controls.
- [x] (2026-09-12 16:08Z) Extended the explicit runner to exact ten- and
  fourteen-case targets and added fail-closed Python controls.
- [x] (2026-09-12 16:54Z) Completed docs, both-toolchain verify/live/example
  runs, ten HTTP smokes, and fresh five-target Jig receipt
  `receipt_01M2B3DJPS1R9HBRWMW2J41H5Q`.

## Surprises & Discoveries


SQLx 0.9.0 `connect_lazy_with` may immediately start maintenance/minimum-connection
work. Lazy does not mean inert. The guarantee is publication of cleanup in the
same synchronous call before pool return, not a fence against another runtime
worker acting during construction. Runtime and native option preconditions remain.

The first native maintenance test retained SQLx's default idle/lifetime timers
and saw no minimum connection within its two-second gate. Inspection of pinned
`spawn_maintenance_tasks` showed that either timer selects a delayed reaper loop;
immediate minimum maintenance occurs when both are disabled. The final test sets
both native options to `None`, preserving rather than rewriting the supplied
configuration, and observes the configured callback before explicit checkout.

SQLx marks a pool closed before its close future finishes waiting for checkouts.
The held-checkout prototype produced TimedOut with `is_closed()` already true.
Successful cleanup must therefore inspect the actual hook record and completed
owner report, not just that flag. Closing pooled connections is also not proof
that detached server sessions have terminated or remote transactions rolled back.

The fresh consumer first tried an asynchronous connect helper, then preferred
the synchronous slot-consuming constructor: its required SELECT 1 already checked
connectivity, while the async helper added context plumbing. That was a within-
agent comparison, not a population reliability measurement. Thirteen native
prototype tests passed on Linux/Rust 1.98.1/PostgreSQL 18.6, but they used local
trust/missing-role and synthetic-handshake controls, not full SCRAM/TLS acceptance.
Their source is archived under `docs/evidence/agent-startup-apis-2026-09-12/`.

## Decision Log


Decision: consume the existing CleanupSlot and native option values synchronously.
Rationale: name validation and exclusive cleanup ownership already exist; another
target trait, supervisor wrapper or async resource protocol adds no required
invariant. The helper must not reintroduce rejectable registration after creation.
Date/author: 2026-09-12, reviewed planning decision.

Decision: keep first acquisition/query explicit and preserve native options.
Rationale: lazy creation is not authentication or schema readiness; an acquire
alone does not reproduce all native connect_with warmup semantics. Applications
own SQL and deadlines, while the adapter owns cleanup publication only.
Date/author: 2026-09-12, planning agent.

Decision: retain register_pool_close as an explicitly lower-level compatibility API.
Rationale: externally acquired pools still have callers; changing its signature or
silently claiming their prior acquisition was protected would break the contract.
Date/author: 2026-09-12, planning agent.

Decision: add a separate fourteen-case pool_ownership_live target to the explicit
SQLx runner. Rationale: preserve the original ten disposition cases and give this
task an exact, independently selectable ownership inventory. The runner executes
both targets and rejects missing, skipped, duplicate or zero-test results.
Date/author: 2026-09-12, ExecPlan author, specifying test organization within scope.

## Outcomes & Retrospective


The exact constructor, example, offline checks and fourteen-case owned-pool target
are implemented. On both Rust 1.98.1 and 1.94.0, the explicit runner passed all
ten retained disposition cases and all fourteen ownership cases against external
PostgreSQL 18.6; the standalone example query/cleanup and five HTTP smoke profiles
also passed per toolchain. Both complete verify matrices passed, followed by a
fresh successful Jig profile covering test, Clippy, formatting, contract and file
budget. Protected combined-service adoption remains `batter-lp2.4`; this task
deliberately retains legacy Startup as a compatibility case. Linux is the only
new execution platform, and no TLS/password-method or hosted-CI claim is made.

## Context and Orientation


Preparation HEAD is `39c1b6e3c1c75f808becb5a5e7c33f58001a2ee4`. The workspace has
separate foundation, adapters and unpublished examples. Read root AGENTS.md,
agent-map.md and `crates/batter-sqlx/AGENTS.md`; preserve unrelated documentation,
tracker and append-only Jig state. If independent facade work has moved the
foundation, resolve its current paths but do not make facade delivery a blocker.

`crates/batter-sqlx/src/lib.rs` owns PgLease, probe and register_pool_close.
PgLease retires an unsuccessful client connection; successful reuse is explicit.
`src/failure.rs` retains native SQLx errors behind fixed diagnostics.
`crates/batter/src/cleanup.rs` defines CleanupSlot, CleanupStack and CleanupReport.
A finalizer is an explicitly awaited cleanup function. LIFO means finalizers run
in reverse registration order, so dependents close before dependencies.

`crates/batter/src/command.rs` and command/ drive finite work and cleanup under a
retained owner. `startup.rs` and startup/ drive service initialization, error
cleanup and transfer to a running owner. Their scopes reserve names before
acquisition. On failed registration the old pool helper still leaves ownership
with the caller; the new helper cannot fail that validation after construction.

Existing adapter tests are `tests/offline.rs`, `tests/postgres_live.rs` and fixture
tests. `scripts/sqlx_live.py`, invoked by `scripts/test_sqlx_live.sh`, owns the exact
ten-case external database inventory. Fixture provisioning remains external;
the optional SQLx test-support module and its lease/session protocol are not being
redesigned. Per-pool fixture profiles remain separate task `batter-7r3.8`.

Native source checked during research is SQLx 0.9.0 pool options.rs, inner.rs and
mod.rs. connect_lazy_with creates PoolInner and starts native maintenance; close
marks closure then waits for accounted resources. Native constructor assertions
or runtime absence may panic before a pool is returned. This helper introduces
neither panic conversion nor rollback of partial native construction. A preceding
resource already registered with Command/startup still follows its existing panic
cleanup contract. Do not promise observation of arbitrary native background panics.

## Interfaces and Dependencies


Add this public crate-root function in `crates/batter-sqlx/src/lib.rs`:

    pub fn pool_in(
        slot: batter::cleanup::CleanupSlot<'_>,
        pool_options: sqlx::postgres::PgPoolOptions,
        connect_options: sqlx::postgres::PgConnectOptions,
    ) -> sqlx::PgPool;

The complete ownership operation is deliberately small:

    let pool = pool_options.connect_lazy_with(connect_options);
    let closing = pool.clone();
    slot.register(move || async move {
        closing.close().await;
        Ok(())
    });
    pool

Do not call register_pool_close inside this function. Do not add a context,
Supervisor, URL, environment reader, extra name, Result or future to its signature.
The slot already validated the name. No additional synchronous application
callback, await or fallible registration may intervene between successful native
construction and publication. Native maintenance callbacks can run concurrently.
Do not alter min_connections, callbacks, native timeouts or session settings.

Preserve the exact old register_pool_close signature and disclose its existing
rejection obligations. Keep PgLease/probe behavior and SqlxFailure unchanged.
Connectivity is established by an explicit native query or existing bounded
probe after creation. Application schemas, migrations, replay and transaction
commit ambiguity remain outside the constructor. No dependency upgrade is needed.

## Plan of Work


### Milestone 1: publish the ownership primitive and its public composition


Implement pool_in and rustdoc with the signature above. Explain the live Tokio
runtime/native-option preconditions, lazy return, synchronous registration and
server-session limits. A pool clone retained by a caller does not block close,
but an outstanding checked-out connection can. The owner must join dependent
work and release checkouts before expecting successful finalization.

Add adapter-owned `crates/batter-sqlx/examples/owned_pool.rs` using
Command::new, a concrete error type, direct reserve_cleanup and pool_in followed
by a bounded query. This standalone executable requires explicit DATABASE_URL and
must fail with sanitized diagnostics when it is absent; it must not provision a
database or mutate the environment globally. Keep its query/cleanup observations
available before checking success. Do not add SQLx to the foundation for an example.

In offline tests prove old function-pointer compatibility and construction under
a live runtime using lazy options without claiming database success. Test invalid
and duplicate reservation before entering native construction, with an independent
constructor-call counter. Fulfill the first slot before expecting a duplicate;
dropping an unused reservation does not permanently claim the cleanup name.
An unstarted Command must invoke neither its factory nor pool construction.

### Milestone 2: independent native ownership oracles and normal operation


Create `crates/batter-sqlx/tests/pool_ownership_live.rs` and suite-owned support
under `tests/pool_ownership/`. Mark each external case ignored in ordinary tests
with a reason pointing to the explicit runner. Use only authorized disposable
PostgreSQL selected through DATABASE_URL; do not create a cluster inside the
adapter. Keep test-owned diagnostic connections outside the closing pool's capacity.

Define one shared assertion that requires the expected named cleanup record,
Succeeded outcome, no skipped hooks, correct count, complete owning report,
`size() == 0` after closure and PoolClosed on later acquire from a retained clone.
The report check must reject an empty successful CleanupReport. An independent
query must return the integer 1 before any successful-database claim. Do not
derive expected results from the implementation's own registration counters.

Run finite Command and existing Startup::new paths. The latter reserves through
legacy scope.supervisor().reserve_cleanup and registers a real acknowledged
component; drain must join it before pool close. This deliberate compatibility
fixture does not introduce a dependency on the future protected scope.

Set distinct native option values and observe them through SQLx behavior. Preserve
after_connect session policy with a native SHOW/query. For positive min_connections,
acknowledge background connection creation before any explicit checkout; use a
bounded gate, not an instantaneous count at function return. Do not assert absence
of native work during construction or secretly set minimum capacity to zero.

### Milestone 3: failed work, cancellation, destruction and close completion


For a native query failure, use a real PostgreSQL error such as division by zero
and inspect SQLSTATE 22012 through the concrete SQLx error. Retain it in
SqlxFailure/OperationError or an explicit application enum, not a string. A
separate failed/timed-out finalizer must coexist with that error in the report.
Authentication acceptance must exercise server rejection of invalid credentials
on a disposable SCRAM/password endpoint; local trust, refused sockets and missing
configuration are not substitutes. Record the original returned native category.

For acquisition cancellation use an acknowledged stalled native handshake or
held acquisition gate, then cancel the operation/owner through its existing API.
Require that the registered pool finalizer survives even though no successful
connection was delivered. Release test-owned sockets during recovery. This is a
cancellation case, not successful SQL/authentication evidence.

After a successful query, return a typed later initialization failure; separately
panic in the application initializer after registration. Retain original error
or panic alongside successful cleanup. Do not claim the owner catches SQLx
maintenance tasks' independent panics or suppresses the default process panic hook.

Register pool A, pool B, then a dependent non-pool hook. Hold the dependent hook
and prove earlier hooks have not completed; release it and require records in
exact dependent/B/A order and completed native close for both pools. Keep work
owners and observations outside any timeout future that is intentionally dropped.

Hold a real checkout outside the closing callback. In one variant observe pending
close, release it before budget expiry, and require success. In another retain it
through the budget and require the actual TimedOut/incomplete cleanup result, not
success from is_closed. Do not demand Unjoined when the engine cooperatively
aborted and joined the close future. Release the checkout and await native close
under a separate recovery bound; that recovery never changes the original report.

### Milestone 4: freeze live discovery and prove the tests reject false success


Add exactly these fourteen external test names to the new target:

    command_query_closes_owned_pool
    startup_query_joins_before_pool_close
    invalid_slot_prevents_pool_construction
    duplicate_slot_prevents_second_pool
    native_options_and_maintenance_are_preserved
    authentication_error_keeps_pool_cleanup
    cancelled_acquisition_keeps_pool_cleanup
    later_application_error_keeps_pool_cleanup
    application_panic_keeps_pool_cleanup
    two_pools_close_after_dependents_in_lifo_order
    held_checkout_delays_successful_close
    held_checkout_timeout_is_not_success
    work_and_cleanup_failures_are_both_retained
    ownership_oracles_reject_missing_and_premature_cleanup

The last case runs isolated test-only negative subjects: one omits close
registration and one reports success as soon as is_closed is true. The same
canonical oracle must reject both for the intended reason. After observation,
repair their resources explicitly under a bound. Do not ship a runtime flag that
disables production cleanup and do not mutate the oracle between subjects.
Keep corresponding invalid/duplicate offline controls for default gates too.

Extend scripts/sqlx_live.py to describe two named targets rather than replace
its current inventory. Keep the original ten names unchanged:

    blocked_cancellation_releases_capacity
    blocked_deadline_releases_capacity
    blocked_error_releases_capacity
    blocked_panic_releases_capacity
    blocked_outer_drop_releases_capacity
    repeated_interruptions_leave_independent_residual_sessions
    success_and_acknowledged_transactions_reuse
    native_database_failure_retires_and_preserves_cause
    rejected_commit_preserves_native_cause_and_retires
    ordinary_return_control_retains_blocked_capacity

Discover each target with --ignored --list --format terse, compare exact names,
then execute all of them with --ignored --test-threads=1 --nocapture. Validate
per-name success as well as each summary: ten original and fourteen new, twenty-
four total at this baseline. Add focused Python runner controls in
scripts/test_sqlx_live.py for missing, zero, duplicate, skipped and failing cases.
Wire that control into scripts/test_matrix.py's appropriate prerequisite batch
and update its matrix tests. Use the existing bounded process runner, with output
overflow and watchdog failures treated as failures, not lost diagnostics.

### Milestone 5: document the contract and hand off exact delivered evidence


Update adapter README/rustdoc, docs/integrations.md, guarantees, testing, status,
references and validation. Explain native runtime/precondition behavior and the
distinction between lazy ownership and connection/query readiness. Show concrete
work and cleanup error inspection without printing secrets. Preserve native
options selected by reference RootSettings; exercise that existing settings
boundary without introducing a settings dependency into the adapter.

Do not migrate all canonical consumers here; `.4` consumes this API and inventory.
Do not change optional fixture profiles or lease/session cleanup. Check new source,
runner and fixture roots against both .jig.toml and .agent/jig-contract.json,
updating them together where needed. Record exact twenty-four-case evidence for
the successor, or an explained preserved-predecessor inventory if independent
authorized work legitimately added cases before implementation.

## Concrete Steps


Work from `/home/aa/Documents/batter`, or its current workspace location. Inspect
git status, `br show batter-lp2.3 --json`, exact Cargo.lock and toolchain versions.
Start `scripts/jig work start --title 'Implement batter-lp2.3' --body-file
.agent/plans/batter-lp2.3-owned-sqlx-pool.md --json` and record the returned ID.
Do not print DATABASE_URL or credentials when checking fixture availability.

After adding the named targets, run:

    cargo test -p batter-sqlx --features test-support --locked
    cargo test -p batter-sqlx --doc --locked
    cargo check -p batter-sqlx --all-targets --all-features --locked
    python3 scripts/test_sqlx_live.py
    python3 scripts/test_parallel_process.py
    bash scripts/test_sqlx_live.sh

Run the explicit live runner on Rust 1.98.1 and 1.94.0 with an authorized disposable
DATABASE_URL in the process environment. It must fail, not skip, if prerequisites
are absent. Keep credentials out of logs and source snapshots. Then execute:

    RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh

For each toolchain rebuild the HTTP example and run these five modes with the
same selected environment. Ordinary workspace verification does not run them:

    cargo build -p batter-axum --example http_service --locked
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline

Inspect implementation-plan Jig work evidence and gates, run missing required
checks with `scripts/jig work check --plan-id <id>`, and read back a fresh passing
api:test receipt and sibling policy gates. Reuse only with unchanged source,
configuration, commands, toolchain, environment and prerequisites and no later
unresolved failure. A collection_limit response requires an evidence/gates read
with --freshness-timeout-ms 30000, not blind re-execution. Close the Bead and
finish its Jig plan only after the required evidence is complete.

## Validation and Acceptance


The public constructor is synchronous, native and slot-consuming, with no
post-construction validation failure. Both finite and existing service owners
retain awaited closure through success, cancellation, native/application failure
and application panic. Names validate before pool creation; callbacks/options
retain native behavior. LIFO, held-checkout delay/timeout and combined-error tests
inspect complete reports, with negative subjects proving the oracle can fail.

All ten existing disposition cases remain exercised. Every new named case must
run once in its explicit target; compilation or a total count alone is not proof.
No provisioning/harness/settings dependency is added to default adapter consumers.
Both Rust toolchains are required. Label Linux and macOS evidence separately;
do not infer macOS, TLS, SCRAM or hosted CI results from unrelated runs. Native
construction panic, runtime death, detached sessions and remote commit ambiguity
retain their existing limits.

## Idempotence and Recovery


Use task-owned disposable schemas/databases and existing external provisioning
authority. Do not use or reset a production endpoint. Tests must release held
checkouts, blockers, sockets and diagnostic pools even when assertions fail.
Retain the original owner report before recovery. The independent recovery bound
must not inherit an already-cancelled operation token. Never drop fixture leases
or abort unaccounted native work to manufacture a successful cleanup report.

Rerunning tests must not reuse stale source/binary identity or silently substitute
ambient configuration. Keep generated Cargo locks generated by Cargo. Preserve
unrelated worktree changes and append-only state; undo only task-owned edits if
needed. Repeated confirmed example failures trigger the ADR-010 assessment and
explicit report before dependent repairs, not an unbounded patch loop.

## Artifacts and Notes


Record fixture authentication mode and server version without credentials, native
source/lock versions, actual commands, expected/discovered per-target names,
individual results and final Jig receipts. Distinguish successful SQL, native
authentication rejection and synthetic cancellation evidence. Record the complete
pool_ownership_live inventory on `.3` so `.4` preserves it rather than guessing
a future count. No experiment result is promoted to production acceptance here.

Revision note, 2026-09-12: initial execution plan specifies the already-reviewed
constructor and makes test placement, the fourteen-case ownership inventory,
runner rejection controls and predecessor-independent validation explicit.
