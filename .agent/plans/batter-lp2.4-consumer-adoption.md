# Adopt protected startup and pool ownership in real consumers


This living ExecPlan implements Bead `batter-lp2.4` under epic `batter-lp2`, in
accordance with `.agent/PLANS.md`. Beads owns scope, dependencies and acceptance;
this file owns execution and evidence. Keep Progress, Surprises & Discoveries,
Decision Log and Outcomes & Retrospective current. This is an implementation
handoff, not permission to implement, provision infrastructure or commit now.

## Purpose / Big Picture


Library ownership improvements are incomplete until applications use them. After
this task the reference service, PostgreSQL lifecycle demo and HTTP example
select protected startup and Unix policy without writing signal handoff logic.
The reference and PostgreSQL demo, plus the finite retirement command, use
slot-owned native pool creation without a manual close-registration recovery path.
Real requests, native queries and signal-triggered cleanup must retain existing
application behavior. A fresh consumer agent must implement and modify a service
from public guidance with independently checked failure and cleanup outcomes.

## Progress


- [x] (2026-09-12 13:25Z) Prepared this plan from current roots, reviewed task,
  failure fixtures and exact live inventories; no consumer has been migrated.
- [ ] Verify completed .1/.2/.3 revisions, interfaces, receipts and fixture access.
- [ ] Migrate reference startup, registration and error-wrapper boundary.
- [ ] Migrate PostgreSQL/HTTP service examples and finite retirement pool creation.
- [ ] Execute eight frozen combined process cases and preserve all live inventories.
- [ ] Complete bounded fresh-agent implementation/modification and independent review.
- [ ] Update canonical guidance and run complete both-toolchain/offline/live/smoke gates.
- [ ] Record all outcomes, finish the implementation plan and close only this Bead.

## Surprises & Discoveries


Current reference startup manually installs listeners, races application work
against reception, requests drain and transfers listeners. Its pool helper already
uses the correct reserve/create/register sequence, demonstrating the reusable
adapter gap rather than missing application awareness. The PostgreSQL demo
connects and probes before it installs listeners, leaving that interval outside
its graceful signal path. These are the concrete substitutions this task makes.

`RuntimeStartupFailure::startup` publicly exposes a legacy typed startup error.
Changing it in place would break source consumers. Keeping it but moving run to
a new protected wrapper still changes the boxed error's downcast target. The
reviewed plan explicitly accepts that reference-root behavioral cutover and
requires its tests/documentation, rather than claiming full behavioral compatibility.

The finite retirement pool's after_connect guard prevents a replacement physical
connection from silently continuing maintenance. That native callback and its
one-slot/session policy must survive the helper substitution unchanged.

The existing reference signal fixture calls runtime::run in a test process and
exits zero after asserting a startup failure. That does not prove the actual
executable's sanitized nonzero exit. Separate tests below cover both contracts.

Historical fresh-agent registration trials exposed missing module paths, an
error conversion mismatch and confusion about check_shutdown returning unit.
Another parent rerun accidentally reused a shared Cargo output binary; its result
was rejected and independently rebuilt. Future evaluation must bind every source
variant to its actual executable. These are historical observations, not current
production acceptance; see docs/evidence/agent-startup-apis-2026-09-12/README.md.

## Decision Log


Decision: depend on batter-lp2.1, .2 and .3, but not independent facade work.
Rationale: actual protected interfaces must exist before migration; facade imports
can be adopted if already delivered without forcing another delivery chain.
Date/author: 2026-09-12, reviewed planning decision.

Decision: add ProtectedRuntimeStartupFailure while preserving the old type/accessor.
Rationale: represent real protected application and signal failures without
fabricating an application cause or changing the old accessor's return type.
runtime::run's new boxed-error downcast behavior is explicit and tested.
Date/author: 2026-09-12, third planning review refinement.

Decision: preserve schemas, transaction/replay behavior, native callbacks and
withheld application readiness. Rationale: this is an ownership-path migration,
not authorization to add a provider handler, startup witness jobs or new workflows.
Date/author: 2026-09-12, planning agent.

Decision: freeze eight added reference cases and a bounded public-agent evaluation.
Rationale: distinguish real executable behavior from harness evidence and prevent
success from being manufactured by skipped cases or unlimited unreported repairs.
Date/author: 2026-09-12, standalone task review refinement.

## Outcomes & Retrospective


No migration is implemented yet. This task remains blocked on `.1`, `.2` and `.3`.
Its plan is ready to execute after those deliveries and authorized fixture access
are verified. The parent epic closes only when all four tasks satisfy their own
acceptance; preparing or completing this plan does not close the predecessors,
the broader operational-adoption task or any publication/deployment decision.

## Context and Orientation


Preparation HEAD is `39c1b6e3c1c75f808becb5a5e7c33f58001a2ee4`. Preserve the closed
batter-fbd documentation convergence in that commit and all subsequent relevant
changes. Begin with root AGENTS.md, agent-map.md and the guides in
examples/reference-service, examples/postgres-lifecycle, crates/batter-axum,
crates/batter-sqlx and crates/batter-runledger. Scenario labels remain generic.

`examples/reference-service/src/runtime.rs` owns service initialization, health,
HTTP and native Runledger registration. `src/main.rs` maps errors to fixed
executable diagnostics. `src/config/` owns validated settings and native option
construction; `schema.rs`, delivery.rs and retirement code own durable protocols.
A composition root is this application-owned place where native resources,
settings and adapters are assembled; it is not a new framework abstraction.

`examples/postgres-lifecycle/src/main.rs` owns run, serve and report_exit.
Its src/tests.rs and src/tests/live.rs test those real paths. It deliberately
registers a long-running application component, so it remains a service example,
not a finite command merely because its only visible work is database startup.

`crates/batter-axum/examples/http_service.rs` owns configuration, health helper,
router, bound listener, startup and sanitized exit. It needs no PostgreSQL.
`examples/reference-service/src/retirement/session.rs::Session::new` is the finite
native pool consumer; it uses CommandScope, not service startup or Unix policy.

`examples/reference-service/tests/support/startup_process.rs` provides private
child launch and typed startup report checks, currently with a TERM-only sender.
startup_signals.rs and related native fixture helpers own suite-specific tests.
`scripts/reference_live.py` is the exact-inventory runner, separate from the
SQLx adapter runner and the PostgreSQL lifecycle executable smokes.

## Interfaces and Dependencies


Before claiming implementation, inspect these exact predecessor Beads:

    br show batter-lp2.1 batter-lp2.2 batter-lp2.3 --json

Record their completed revisions, public signatures, exact test inventories and
required receipts in this plan. Future revision IDs cannot be supplied now. If
delivery disagrees with the following contract, report it to its owning Bead
and block this migration; do not implement missing predecessor APIs inside `.4`.

The protected API is `Startup::scoped(supervisor, context, cleanup, initialize)`
returning `ScopedStartup<F>`, with `.with_unix_signals(name)` and
`.without_readiness_approval()`. Start returns
`StartingSupervisor<InitializationError<E>>`. The scope type is
`batter::startup::ProtectedStartupScope`, exposing stage, direct reserve_cleanup
and registration only. `batter::registration::{Registration, RegistrationTarget}`
provides the concrete view and sealed target. Native adapter entrypoints are
`batter_axum::register_http_in` and `batter_runledger::register_in`; old names stay.

The SQLx constructor is:

    pub fn pool_in(
        slot: batter::cleanup::CleanupSlot<'_>,
        options: sqlx::postgres::PgPoolOptions,
        connection: sqlx::postgres::PgConnectOptions,
    ) -> sqlx::PgPool;

It synchronously publishes native close after successful lazy construction and
before return, not connectivity, schema readiness or remote-session termination.
Native work may start during construction. Existing probes/queries retain their
budgets and original concrete failures. No migration, retry or environment policy
belongs inside pool_in. Command still uses its existing direct cleanup reservation.

InitializationError is non-exhaustive, with Application(E),
Signals(SignalRegistrationError), and SignalPolicyAlreadySelected. Received
signals use StartupCause::Draining, not a fabricated application error. Defaults
are redacted; native cause and full cleanup reports remain inspectable explicitly.
check_shutdown consumes a shutdown outcome and returns unit on success; retain
the actual outcome/observer before invoking it when a test needs detailed reports.

Add this reference wrapper without altering RuntimeStartupFailure's old accessor:

    pub struct ProtectedRuntimeStartupFailure(/* private retained error */);
    impl ProtectedRuntimeStartupFailure {
        pub fn startup(&self) -> &StartupError<InitializationError<InitializationFailure>>;
    }

Implement fixed Display/Debug and an Error source exposing the protected startup
error. runtime::run still returns Result<(), BoxError>, but its startup-error
downcast target deliberately becomes ProtectedRuntimeStartupFailure. Update
startup_process.rs and all intended canonical-root downcasts; retain a compiling
old accessor fixture. Never stringify or discard the original application/IO cause.

Facade task batter-tmx is related, not blocking. If absent, use current crate paths
and mark facade feature/re-export checks N/A. If present, consume its delivered
imports and identity fixtures without duplicating foundation types or implementing
the facade here. Broad adoption batter-7r3.6 and fixture profiles batter-7r3.8 remain
independent; do not alter their scope or close them for this task.

## Plan of Work


### Milestone 1: migrate the reference root and keep errors observable


Change runtime::run to Startup::scoped(...).with_unix_signals("signals") and retain
without_readiness_approval. Capture the readiness handle and native initialization
OperationContext at the root before moving the Supervisor into startup. The
20-second startup allowance still includes native initialization; no new full
allowance begins at a later phase. Do not add a context accessor to the scope.

Remove manual install_signals, the application tokio::select, readiness.request
and InstalledSignals::register transfer. Remove the application signals.install
stage; policy failures occur before the callback at stage startup. Preserve
postgres.acquire, postgres.schema, http.bind and worker.register stages and their
application meaning. Do not claim no initializer can enter after kernel signal
arrival; the library's cooperative observation boundary is the guarantee.

Change register_pool to accept ProtectedStartupScope, reserve postgres.pool,
obtain the root's validated native options and call pool_in. Preserve
RootSettings::connect_options_from_process and pool_options, including PG*/passfile
rejection; never parse the validated reference connection policy again through
native from_env or a fallback URL parser. Keep the explicit acquisition and schema
initialization after pool ownership has been published.

Change register_health to accept a concrete Registration view or mutable reborrow
of it, not &mut Supervisor. Register HTTP with register_http_in and owned native
Runledger preparation with register_in. Preserve jobs_config, empty JobRegistry,
native prepare(), all stop/settlement translation and absence of startup witness
jobs. Do not enqueue work or add a provider handler to obtain readiness.

Add ProtectedRuntimeStartupFailure and switch only the canonical run mapping.
Tests must inspect Application(InitializationFailure), a signal-policy failure,
and Draining with their full cleanup records. Preserve old accessor source
compatibility and fixed main.rs stderr. A failed diagnostic write must not turn
failure into success. Run reference configuration/diagnostic tests and compile
all targets before continuing to the next consumer.

### Milestone 2: migrate examples and preserve native retirement policy


In postgres-lifecycle, change serve's initializer bound to ProtectedStartupScope
and select signals on the builder before start. Reserve the pool slot, parse the
application-owned DATABASE_URL into native PgConnectOptions at the application
boundary, and call pool_in with the existing max-connections/acquire-timeout
options. Parsing belongs before native creation and inside retained startup
where appropriate. Keep the explicit bounded probe after lazy creation; it now
establishes connectivity rather than relying on an eager constructor.

Create the 15-second startup context at the owning root and derive the probe's
maximum-five-second child through OperationContext::child before/pursuant to
use; pass the root context into private serve as needed. This keeps the probe's
deadline within its parent without broadening the public scope. Preserve the
long-running application component, actual mark_started, ready marker, fixed
ProcessFailure diagnostics and complete startup/shutdown report source chains.
Update private serve callers in tests and all three existing live cases.

In http_service, capture the shutdown/readiness handle before moving Supervisor,
use protected startup, change the health helper to registration authority and
use register_http_in. Select Unix policy on the builder; remove only obsolete
example-local signal calls. Keep routes, middleware ordering, request correlation,
logging selection, capacity, deadlines and all five smoke behaviors. Do not add
SQLx to this HTTP-only example.

In retirement/session.rs, replace only connect_lazy_with plus manual clone/hook
publication with pool_in(slot, options, connection). Preserve max_connections=1,
min_connections=0, no idle/max lifetime, the two-second acquisition timeout,
search_path=public and the entire atomic after_connect replacement guard. Its
fixed error text is intentionally safe for native SQLx logging. Keep finish,
verify, transactions, original-error retention and separate readback ownership
unchanged. The ignored physical-session replacement regression must still run
through the reference live runner. No durable migration is edited or added.

### Milestone 3: add eight causal process cases with independent observations


Add the following eight named wrappers to reference_live and its explicit runner.
Reuse the existing child_fixture dispatch, not another discoverable child entry:

    protected_startup_acquisition_sigterm
    protected_startup_acquisition_sigint
    protected_startup_schema_sigterm
    protected_startup_schema_sigint
    protected_startup_waiter_loss
    protected_startup_owner_loss
    protected_startup_executable_sigterm
    protected_startup_executable_sigint

For the acquisition pair, the existing child harness invokes actual runtime::run
against a held native handshake. The parent waits for native acquisition
acknowledgement, sends the selected signal and bounds reaping. The child exits
zero only after inspecting ProtectedRuntimeStartupFailure: stage postgres.acquire,
cause Draining, no Ready, and one successful postgres.pool cleanup record with
no skipped hooks. A held synthetic handshake is cancellation evidence, not a
successful database query. Preserve existing suite-owned native observation.

For the schema pair, actual runtime::run targets the authorized disposable database.
An independent PostgreSQL observer must confirm the blocked schema SQL/lock before
TERM or INT is sent. Require the same successful assertion-child exit and report
checks, but stage postgres.schema. Release blockers safely after retaining the
result. Timing proximity alone does not establish that initialization reached
either phase.

The waiter-loss case is a public protected-composition fixture, not a new control
API on runtime::run. It successfully queries the real pool, registers a dependent
finalizer, sets test.hold and parks initialization. Cancel only a borrowed wait,
keep the owner alive, send TERM, then resume/observe the same retained Draining
report. Require the dependent hook before one successful pool close and no Ready.
The owner-loss counterpart drops the owner after the same native query and stage,
sends no OS signal and retains only the observer. It must complete cleanup on the
live runtime with the same report fields. Both assertion harnesses exit zero.

The executable pair launches the actual built reference-service binary at held
native acquisition, then sends TERM or INT. Require process exit code 1, not
Unix signal termination, and stderr exactly `Error: reference service failed`
followed by newline. The paired typed harness cases prove internal report details;
do not infer those fields solely from the executable's exit. Keep stdout/stderr
separate, retain bounded capture and never print endpoints, credentials, native
errors or private markers. Assertion children have empty application stderr;
distinguish Rust test-harness banners from application messages.

Extend existing production_root_withholds_readiness_without_control_jobs to run
both TERM and INT after /live returns 200 and /ready repeatedly returns 503.
Require ordinary exit zero, observed native joins/cleanup, no private diagnostics
and unchanged absence of durable startup witnesses. HTTP status alone cannot
prove pool closure or native settlement. Preserve `.2`'s separate post-start,
pre-coordinator-poll coverage; these new cases prove actual root composition.

### Milestone 4: preserve exact inventories and trustworthy execution accounting


The recorded reference inventory contains 58 entries. Preserve these exact names
and add the eight above, giving 66 at this baseline:

    production_root_withholds_readiness_without_control_jobs
    retirement_preserves_history_and_disables_old_catalog
    retirement_rejects_wrong_identity
    retirement_rejects_hidden_sessions
    retirement_observes_late_enqueue
    retirement_rejects_prepared_enqueue
    retirement_retains_commit_error_through_readback_cancellation
    retirement_retains_lost_commit_acknowledgement
    child_fixture
    startup_signal_during_pool_acquisition
    native_initialization_without_queue_writes
    native_in_flight_finishes_after_drain
    native_owner_drop_retains_settlement
    native_business_failure_preserves_process
    native_unjoined_callback_blocks_dependency_cleanup
    startup_signal_during_schema_initialization
    configured_command_root_bounds
    configured_pool_capacity_and_acquire_timeout
    configured_startup_pool_close_before_lease
    configured_worker_concurrency
    fixture_abandoned_producer_failure_retained
    fixture_assertion_and_script_failures_retained
    fixture_autovacuum_retains_lease
    fixture_body_and_cleanup_failures_retained
    fixture_cancelled_creation_retains_producers
    fixture_close_order_and_resumable_wait
    fixture_detached_sessions_retained
    fixture_distinct_deferred_and_consuming_failures
    fixture_driver_error_closes_admin_pools
    fixture_finish_preserves_body_failure
    fixture_foreign_template_rejected
    fixture_handled_pool_error_retained
    fixture_observer_failure_preserves_body
    fixture_one_slot_lock_operation
    fixture_partial_acquisition_and_panic
    fixture_pending_body_keeps_diagnostics
    fixture_pending_completion_recovers
    fixture_pool_error_is_pending_cleanup
    fixture_premature_shared_close_recovers
    fixture_replacement_before_first_attempt
    fixture_restricted_observer_sees_other_role
    fixture_retry_during_active_attempts
    fixture_retry_during_active_attempts_multithread
    fixture_role_cleanup_after_assertion
    fixture_runtime_loss_exposes_native_drop
    fixture_session_observer_failure_resumes
    fixture_shared_harness_admission
    fixture_shared_retry_and_close
    fixture_startup_is_not_a_connection_fence
    fixture_template_observation_recovers
    fixture_template_reuse_and_isolation
    fixture_waiter_loss_keeps_cleanup_driven
    fixture_wrong_server_retains_lease
    initialized_schema_upgrade
    lease_cleanup_defer_and_drop
    migrations_and_transactional_enqueue
    reference_delivery_command_and_reconciliation
    isolated_durable_execution_and_shutdown

Preserve the separate library test
retirement::session::tests::maintenance_session_replacement_is_refused. If other
authorized work added predecessor cases, preserve that inventory plus this exact
eight-case delta and explain the total. Never delete cases merely to reach 66.
Fix the runner's current len(CASES)-2 live-probe announcement: new synthetic
acquisition and executable controls must be classified explicitly, not counted
as successful database probes. Keep exact per-name discovery/execution checks.

Read `.3`'s delivered SQLx inventory rather than rewriting its tests. Its plan
delivers ten existing disposition cases and fourteen pool-ownership cases in two
targets. Preserve its actual completed inventory and receipts; do not reopen that
task's implementation checklist. Keep PostgreSQL lifecycle's three live cases:
success_closes_the_live_pool_before_exit,
startup_database_error_retains_failed_cleanup_and_closes_the_pool, and
task_database_error_survives_shutdown_and_pool_close, under tests::live::.

Update scripts/test_reference_live.py, test_smoke_postgres.py and any affected
runner controls so missing, zero, skipped, duplicate, wrong-status and timed-out
execution still fail. Python owns scheduling/inventory, while existing native
preflight owns credential/cluster policy. Extend exhaustive Jig source scopes
in both .jig.toml and .agent/jig-contract.json when needed; keep required policy
checks as siblings rather than dependencies of Rust execution.

### Milestone 5: evaluate public guidance with a bounded fresh-agent task


Use two fresh consumer-agent identities and three prompts: agent one writes an
initial service and then modifies it; agent two independently reviews the public
consumer packet. A fresh identity means new context containing only the frozen
public packet, not this plan, internal tests, prototype repairs or another agent's
transcript. Record actual model/session IDs and all allowed inputs. The executing
root agent is the evaluator, separate from the consumer author.

Before delegation, freeze the packet's behavioral requirements and independent
runtime oracle. Supply public crate docs/manifests and authorized disposable
fixture access, but do not prescribe ownership function names. Require a native
query, real HTTP response, startup TERM cleanup, later application/native failure,
fixed diagnostics and complete named cleanup records. Then request a second
dependency and fallible startup stage, requiring LIFO without new signal routing.

The initial authoring phase is explicitly execution-free: no compiler, tests or
evaluator feedback until first source submission. Reading public docs is allowed.
Create a task-owned directory with mktemp -d in a writable cache; snapshot the
whole submitted source/manifests with a SHA-256 manifest before evaluator compile.
Use distinct Cargo target directories for variants and verify invoked binary
identity. Keep first source, each diagnostic, abandoned approach, repair and first
modification source separately. The author cannot edit the independent oracle.

Allow at most two repair attempts after each first submission in this evaluation
batch. Both final variants must compile and pass every independent semantic
oracle. Retain failures as first-attempt failures even after repair; report no
population reliability percentage. The independent packet reviewer must find it
implementable or have concrete omissions corrected and checked again. A failing
batch leaves `.4` open with its design assessment; do not silently keep sampling
until success. ADR-010's repeated-defect trigger applies before the numeric limit.

Archive source-bound packet, oracle, submissions, repair history and sanitized
results in a task-specific docs/evidence directory, linked from validation and
the Bead. Exclude credentials, database data and build products. This evaluation
is new production-path acceptance; earlier prototypes are not substitutes.

### Milestone 6: canonical contracts and complete integration evidence


Update docs/usage.md's service/command examples, docs/integrations.md's SQLx and
Runledger sections, docs/guarantees.md's startup/signal/cleanup contracts, and
docs/architecture.md, testing.md, validation.md, references.md and
reference-compatibility.md. Update status rows Service startup versus finite
commands, Runledger host/trace adapter and Live PostgreSQL integration tests,
plus the protected-startup/pool rows added by predecessors. Preserve historical
evidence and append exact new outcomes. Owning guides route to contracts rather
than duplicating giant consumer manuals.

Combined service examples show protected startup, policy, pool_in and applicable
native adapters. The HTTP-only example adds no SQLx and finite Command adds no
service signals. Old raw scope access, explicit signals and register_pool_close
remain labeled compatibility/escape paths with accurate obligations. Preserve
all settings, authentication, transaction boundaries, histories, sequences and
retirement/reconciliation behavior. No library type proves remote effects.

## Concrete Steps


Work from `/home/aa/Documents/batter`, or its current workspace root. Record git
status, actual predecessor revisions, source/toolchain identities and fixture
authority. Start `scripts/jig work start --title 'Implement batter-lp2.4'
--body-file .agent/plans/batter-lp2.4-consumer-adoption.md --json` and save its ID.
Missing predecessor interfaces or external endpoints block complete execution,
not permission to invent APIs or provision infrastructure silently.

The SQLx and PostgreSQL example runners need an explicit disposable DATABASE_URL.
The reference requires POSTGRES_TEST_ADMIN_URL and POSTGRES_TEST_OBSERVER_URL for
two genuinely distinct local PostgreSQL 18 clusters. The primary needs superuser,
SCRAM host authentication, max_prepared_transactions > 0, track_counts, and
autovacuum with naptime <= 5 seconds. Both endpoints must permit native cluster
identity inspection. Keep unrelated producers off the dedicated serial primary.
Use existing native PG*/passfile/endpoint checks, not shell-parsed credentials.

Check fixture readiness without printing credentials by running:

    cargo run --quiet -p batter-example-reference-service --example reference_preflight --locked

Its expected success marker is reference-preflight:ok. Failure is incomplete
prerequisite evidence; do not claim live acceptance or weaken server policy.
Offline milestone commands are:

    cargo test -p batter-example-reference-service --test configuration --locked
    cargo test -p batter-example-reference-service --test fixture_diagnostics --locked
    cargo test -p batter-example-postgres-lifecycle --locked
    cargo test -p batter-axum --example http_service --locked
    cargo check -p batter-example-reference-service --all-targets --all-features --locked
    python3 scripts/test_reference_live.py
    python3 scripts/test_smoke_postgres.py -v

Execute each of the following on Rust 1.98.1 and Rust 1.94.0 by setting
RUSTUP_TOOLCHAIN for every build and runner. Keep the exact locked graph:

    bash scripts/verify.sh
    bash scripts/test_sqlx_live.sh
    cargo build -p batter-example-reference-service --bin batter-example-reference-service --locked
    bash scripts/test_reference_live.sh
    cargo test -p batter-example-postgres-lifecycle --bin postgres_lifecycle --locked tests::live:: -- --ignored
    cargo build -p batter-example-postgres-lifecycle --bin postgres_lifecycle --locked
    python3 scripts/smoke_postgres.py --binary target/debug/postgres_lifecycle
    python3 scripts/smoke_postgres.py --binary target/debug/postgres_lifecycle --signal SIGINT
    cargo build -p batter-axum --example http_service --locked
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline

The new executable signal tests must receive that toolchain's actual reference
binary, built before they run. Wire explicit binary selection/build into the
runner before reference_live execution; do not let a test discover a stale binary
left by another toolchain. Run the fresh-agent evaluation against the delivered
public source and its separately identified binaries, not the research archive.

Finally inspect `scripts/jig work evidence --plan-id <id>` and `work gates`, run
missing required checks using `scripts/jig work check --plan-id <id>`, and read
back fresh successful api:test and required siblings. Reuse only with unchanged
inputs, commands/configuration, toolchain, environment and prerequisites and no
later unresolved failure. Retry collection-limited evidence reads with
--freshness-timeout-ms 30000 instead of blindly rerunning Rust. Update Bead and
plan outcomes, finish the task-local Jig session and close `.4` only after all
required acceptance. Closing the epic requires independent predecessor completion.

## Validation and Acceptance


All four actual consumers use the appropriate protected path; application signal
handoff and manual pool publication no longer appear in their canonical startup.
Source-compatible old wrapper accessors remain, while canonical boxed-error
downcasts intentionally change and are documented. Native settings/callbacks,
schema/durable behavior and withheld readiness remain unchanged. Actual startup
signals, complete reports, executable exit rendering, native settlement and pool
cleanup are independently observed, not inferred from method names or HTTP status.

The complete predecessor inventories plus eight named reference cases execute,
with exact per-name accounting and preserved runner controls. Both supported
toolchains and all HTTP/PostgreSQL smoke modes are required. Record platform
execution individually; Linux does not imply macOS or hosted CI. The Unix contract
remains Linux/macOS only, with unexecuted platform checks explicitly labeled.
The bounded fresh-author/modification evaluation and independent packet check
must pass without discarding failed attempts. No general agent reliability rate
or remote-session/commit certainty is inferred from these results.

## Idempotence and Recovery


Retain owners and reports outside cancelled waiters. On failure release
acknowledged locks, held checkouts and raw SCRAM sockets before awaiting dependent
DROP-barrier/fixture completion. Preserve pending fixture owners and explicit
retry semantics; do not abort supervision or drop leases to manufacture success.
Diagnostic observer pools need independent capacity. Successful recovery does
not remove original failure records or change an earlier timeout into success.

Use unique task-owned files and externally authorized disposable endpoints. Never
overwrite existing migrations, reset production data, reintroduce retired jobs,
copy historical source over newer facade changes, or modify process-global
environment from parallel Rust tests. Preserve unrelated worktree edits and
append-only state. No automatic commit, push, publication or deployment follows.

When a repair repeats a confirmed invariant failure or breaks a coupled lifecycle
phase, apply ADR-010 immediately: classify implementation mistake versus library
gap versus application/native complexity, retain evidence and report the concern
before dependent repairs. Out-of-scope redesign needs a separate decision. Keep
the independent ready work moving without hiding unresolved acceptance.

## Artifacts and Notes


Record predecessor revisions/receipts, all commands and case inventories,
toolchains/platforms, fixture preflight and source hashes without credentials.
Retain separate harness and executable statuses, typed cause/cleanup observations,
native settlement and bounded recovery records. Public packet, evaluator source,
first submissions, repairs and final binaries' identities must be auditable.
Expected HTTP 200/503 responses and fixed exit strings above are specifications,
not executed results. Append actual outcomes as milestones finish.

Revision note, 2026-09-12: initial ExecPlan translates the reviewed migration
contract into six ordered milestones, concrete root edits, explicit predecessor
interfaces, exact process/live inventories, bounded fresh-agent acceptance and
safe verification/recovery steps. No production scope is implemented here.
