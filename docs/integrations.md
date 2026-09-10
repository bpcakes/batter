# Integration ownership contracts

The `batter-axum` and `batter-sqlx` adapters, native SQLx lifecycle example and
unpublished reference compatibility package exist in this snapshot. The latter
composes pinned native upstream APIs in explicit live probes; see the
[compatibility manifest](reference-compatibility.md).
Runlimit, Runledger, and postgres-test-harness are not dependencies of the
library/test-support crate. The contracts below govern composition; they do not advertise unimplemented
APIs. Delivery scope, acceptance tests and dependencies live in [Beads](roadmap.md).

## Axum: implemented, with a deliberately small boundary

Import `RequestPolicy`, `request_admission` and `observe_http` from `batter_axum`.
Apply `middleware::from_fn_with_state(policy, request_admission)` with
`route_layer` to guarded business routes, merge unguarded liveness/readiness and
fallback, then apply `middleware::from_fn(observe_http)` using `Router::layer`.
Install trusted server request identity outermost. Observation now covers probes,
fallback and rejection responses without imposing admission on them. Domain
services receive their own dependencies through State/FromRef/constructors;
request operation context arrives through Extension<OperationContext>.

Apply observation after every route/fallback is assembled: Axum's router layer
runs after routing and only covers existing routes. A later-added route bypasses
it; a service wrapper outside routing has no matched route template at entry.
Place rejecting/status-changing middleware inside observation so its response
is covered. See the [compiling composition example](../crates/batter-axum/src/lib.rs)
and [HTTP example](../crates/batter-axum/examples/http_service.rs).

`with_failure_renderer` maps middleware failures into an application-owned
envelope using a snapshot of request parts. Trusted correlation middleware must
run outside it; never assume raw inbound headers are trusted. Application
handlers must use their own renderer too. The HTTP example demonstrates both
paths with matching generated request-ID headers/body fields. Readiness requires
all registered components to acknowledge startup before traffic is admitted.

`RequestPolicy` retains one combined readiness/deadline policy. The independent
observer requires neither that policy nor lifecycle state. `request_scope` keeps
the combined observation/admission behavior for compatibility; replace it with
`request_admission` when installing outer observation. There is no automatic
observer deduplication. All entry points use
`batter::telemetry::with_current_dispatch` inside their async bodies to preserve
first-poll capture and full future/span destruction under the captured dispatcher.
The response-construction boundary excludes later body polling and destruction.
Observation also retains the enabled HTTP span or available application span at
first poll. It reuses that context for execution and completion parenting even
when later polling/destruction occurs under another request. For example,
`RUST_LOG=info,batter=warn` retains identity from an enabled outer application INFO
span while disabling the HTTP INFO span. Sink-specific filters still control
visible context; HTTP field recording never modifies application span fields.

Severity policy is also independent of admission. Return an Axum
`Extension(HttpObservationLevel(tracing::Level::INFO))` alongside a response to
select INFO for an application-identified expected readiness failure. Inner
middleware can insert, replace or remove this response extension. Keep the actual
503 and `server_error` outcome; alert rules must distinguish probe traffic if
those fields drive alerts. Built-in probes and unannotated responses retain the
WARN-for-5xx/INFO-otherwise defaults. No response means no override: dropped
futures retain WARN. The HTTP example selects INFO for Starting/Draining probe
responses, leaving stopped-process probes and application errors at defaults.

Keep authentication, authorization, request body limits, CORS, TLS, proxy trust,
trace-header validation, tenant resolution, and user admission policy external.
Choose middleware order deliberately: Batter's timer starts inside its middleware,
not before an outer queue. The example is GET-only and not an upload/streaming
security template. See [guarantees](guarantees.md).

The supervised Axum server uses with_graceful_shutdown and waits for it to finish
while dependencies remain alive. Aborting that wrapper is not accepted as proof
of transitive child termination; resource finalizers are conservatively skipped.
The existing smoke test does not establish full connection/body lifetime behavior.

## SQLx: keep transactions visible

`batter-sqlx` is independently selected. `PgLease::acquire` bounds acquisition
under an existing OperationContext. Move the lease into the operation future;
native queries and transactions borrow `lease.connection()`. Explicitly call
`return_to_pool` only after fully awaited successful query, commit or rollback.
Dropping a transaction/stream does not acknowledge completion. Every other lease
drop detaches and drops the native client, including errors, unwinding and
interruption when the operation owns the lease. A lease retained outside a
cancelled future remains the caller's responsibility.

SQLx 0.9 ordinary pool return pings before releasing capacity and can wait behind
interrupted SQL. Retirement releases that local accounting immediately, but the
server can keep executing or waiting on a lock. Detached sessions can exceed
`max_connections` and survive completed `Pool::close`; neither is a remote
concurrency bound or termination witness. Reconcile uncertain writes and manage
server-side resource policy in the application. The ignored live regressions
hold an advisory lock, prove replacement work and local close before unlocking,
then independently observe every retired backend disappear.

`SqlxFailure` keeps native causes under fixed Debug/Display. Its conservative
`FailureClass` uses native variants and never authorizes replay. Interruption
remains `OperationError::Interrupted`, separate from native pool timeout or commit
errors. Trusted source-chain inspection and SQLx's own logging can expose native
contents. This adapter adds no database creation/recheck, migration, transaction
manager or retry policy. `register_pool_close` retains caller ownership on
registration failure; the caller must explicitly close that pool.

The [example](../examples/postgres-lifecycle/src/main.rs), packaged as
`batter-example-postgres-lifecycle`, uses native PgPoolOptions and the optional
`batter-sqlx` bounded probe and Pool::close registration. It registers close as a dependency finalizer and
uses `Startup` for owned initialization and startup-error cleanup. It reserves the
pool finalizer name before connecting and registers closure immediately after
acquisition. `register_signals` and `check_shutdown` are shared with the HTTP
composition; pool sizing, probe and cleanup budgets remain application choices. Its fixed process diagnostic retains concrete early
errors, the startup cleanup report, or the complete failed shutdown report in its
source chain. No database abstraction or generic transaction retry is introduced.
The example uses SQLx 0.9.0, which requires Rust 1.94 or newer. Portable checks
and separately provisioned live executions have distinct evidence in
[validation](validation.md); compiling the example does not establish pool closure.

Application-owned migration/schema checks happen before readiness. There is no
automatic migration during a health probe. Size pool capacity alongside admitted
HTTP work, Runledger concurrency, harness test admission, and maintenance tasks.
A semaphore bound is not a database-capacity calculation by itself.

For the durable reference path, application writes and Runledger submission must
use the SAME native SQLx transaction. Preserve upstream error distinctions,
especially outcomes around commit. A caller deadline or lost connection does not
prove rollback. Do not turn an ambiguous outcome into a generic "retryable"
Batter error. Implement application idempotency/reconciliation separately.

## Runledger: future host, not another worker runtime

Reviewed baseline from the preceding brief: runledger-core, runledger-postgres,
and runledger-runtime 0.12.0, with a PostgreSQL 18 requirement in its published
README. The reference compatibility package now compiles and exercises the Git
revision selected in the [manifest](reference-compatibility.md). The registry
release uses a different SQLx version and is not interchangeable with that pin.

Runledger retains internal supervision, heartbeat/lease logic, claim behavior,
schedules, workflows, retries, durable intents, schema compatibility, and
operator interfaces. Batter should host the upstream supervisor as one critical
component and preserve its shutdown result. Do not spawn each of its internal
loops under a second independent policy.

The host must align budgets: request upstream stop-claiming during drain, allow
its full cleanup allowance, and observe its returned error. An already in-flight
claim may complete after the stop request. The inspected candidate offers no
linearized stop-claim barrier, and its builder spawns loops without a readiness
acknowledgement. The host needs an application initialization witness and must
state what it proves. Upstream join returns its first failure; later internal
errors are not automatically available to Batter. Dropping the supervisor is
not join evidence. See the dated source inspection in [references](references.md).
Do not assume an adapter can mechanically map every inner signal to Batter's
forced token; inspect upstream behavior and document that translation.

Application writes and dependent job submission share a native transaction.
Rollback, idempotency and payload-conflict semantics belong to the selected
upstream protocol and application. External effects remain at least once unless
the external protocol independently protects them.

Copy correlation metadata into durable payload/envelope fields only through a
versioned, validated representation. Never persist a CancellationToken or Tokio
Instant, and never keep a job subordinate to an already-finished HTTP scope.
The worker creates a new execution context with its own deadline/retry policy.

## Runlimit: future thin policy/telemetry connection

Reviewed baseline: the Runlimit workspace has separate core, memory, PostgreSQL,
HTTP, and Axum responsibilities. Re-check current APIs before coding.

Prefer the upstream runlimit-axum layer rather than reimplementing its admission
path in Batter. The application must establish proxy trust and normalize
identities before generating opaque subject keys. Batter must not interpret
Forwarded, raw emails, session IDs, or tenant policy on the user's behalf.

Translate observations into stable telemetry without raw subjects or backend
error strings as labels. Preserve normal quota denial vs capacity denial vs
backend failure, shadow decisions, retry timing, and consumption certainty. Do
not charge a new user quota automatically for each internal retry attempt.

Layer ordering depends on trusted metadata, authentication and which identity
is being limited. No single order is a universal security claim.

## postgres-test-harness: optional native fixtures

Reviewed baseline: postgres-test-harness 0.2.0 provisions/connects to PostgreSQL
18, caches fingerprinted templates, and clones isolated test databases. Its
public API is independent of SQLx. The reference probes select the exact Git
revision in the [manifest](reference-compatibility.md), with containers disabled.

`batter-sqlx` exposes `test_support` through its opt-in `test-support` feature.
`FixtureSuite` owns the configured harness and retains upstream template handles
inside one Tokio runtime; `DatabaseFixture` owns every declared native pool and
one disposable database lease. Reference tests enable this as a development
dependency and keep application SQL and migration execution local. The default
adapter graph excludes the harness. Generic `batter-test-support` stays a leaf.

`template_spec` frames ordered bundle identity, migration identity/type and exact
SQL bytes plus a setup revision using upstream `FingerprintBuilder`. Initializers
are trusted to match the inputs and close their pools before returning. No static
SQLx pool is shared across destroyed runtimes. Empty-database probes stay separate.

`ConnectionPlan` rejects empty, overflowing and over-budget declarations before
lease/pool acquisition. Pool maxima plus reserved standalone connections must fit
the selected per-database limit; per-lease admin observers count too. Suite
initializer/catalog pools are separately bounded in the consumer (normally one
connection each; the creation-cancellation observer uses two to hold and inspect
a catalog lock concurrently); upstream owner/template/lifecycle sessions are additional server usage.
Independent harnesses do not share a server-wide budget. The lock probe declares
three one-slot pools (operation, blocker, observer), acknowledges an advisory
lock, observes the exact `pg_blocking_pids` relation, then explicitly releases and
joins the transaction before independent readback and finish.

`FixtureSuite::start` owns native lease/template producers before their delivery
waiters can be cancelled. Producers publish acquired leases/templates into the
run registry before delivering access handles. After body exit the driver joins
all producers, closes each database's pools before consuming its lease, then
drains deferred cleanup. Cancelling acquisition, sibling failure and body panic
cannot move creation beyond that barrier. `FixtureReport` retains the body,
all acquisition outcomes, all database cleanup results and drain. Native producer
errors remain failures in the report even if the body handled their delivered
error. Template initializers are Send and static; their task panics become native
initializer errors so upstream awaits template abort. Initializer-owned pools and
operations still require explicit close/join before returning.

A run rejects allocation beyond its own simultaneous-lease capacity. Harness
clones share native admission: other owners must release leases for waiting
producers to progress. Cancelling observation does not cancel that wait or free
another owner's lease. Per-run cleanup does not shut down the server; the caller
retains server ownership and calls shutdown after all runs and leases finish.
Deferred drain is the upstream shared-queue barrier and can observe earlier
submissions from another owner; it is not an exclusive per-run queue.

`PoolAcquire` means registered pools await driver cleanup. Low-level `Connect`
means all opened pools were closed and lease cleanup was awaited; its absent
cleanup error means success. Both paths retain lazy pool handles before polling
connectivity, including the pool whose initialization fails. They check one
connection per pool; SQLx maintains its configured minimum in the background.

Cancelling `FixtureRun::wait` leaves the same driver available to a later wait.
It returns a must-use borrowed report view; owned reports also warn on discard.
Joining the driver only establishes completion: callers inspect report outcomes
or consume `into_result` to decide fixture success.
Dropping its handle detaches the driver and loses report observation. Callers
must join operations and release checkouts before returning; a held checkout can
keep cleanup pending. Runtime destruction and internal driver failure have no
completion guarantee. The low-level `DatabaseFixture::finish` remains available
for caller-managed sequencing; cancelling that future or dropping that owner can
invoke destructive upstream lease Drop. `batter-kjl` retains the remaining
owner-loss, deferred-failure and retired-session failure matrix.

Use native SQLx pools and application migration entrypoints. Close all application
connections before returning a database lease. SQLx 0.9 pool close returns unit;
its incomplete observation must be distinguished from a returned cleanup error.
Preserve body failures and the actual lease/deferred-drain failures on their
respective paths; a first question-mark must not skip independent cleanup.

The inspected harness queues cleanup on lease Drop. Keep the lease in an
explicitly driven owner while connection shutdown remains incomplete; dropping
a skipped finalizer or calling defer is not non-destructive retention. External
harness shutdown is a no-op, so deferred cleanup requires a separate awaited
drain. If an adapter detaches SQLx connections, pool close alone cannot prove
those sessions stopped; fixture release needs their separately observed
termination. No runtime-death or arbitrary async-drop guarantee follows. Keep
a separate empty-database path for migration-order tests.

Do not issue Docker CLI cleanup, clone databases manually, delete by name prefix,
reset dirty test databases for reuse, or advance PostgreSQL clocks by calling
Tokio's time::advance. Do not silently pass tests when the database is unavailable.
Use explicitly selected live test targets and report prerequisites or failures.
The workspace checks enable all features and targets: feature gating alone
cannot keep database-dependent cases out of ordinary verification. The reference
compiles ignored live cases during those checks and runs them through an
explicit prerequisite-checking runner; skipped cases are not executed evidence.

## Dependency direction and extraction criteria

Applications depend on Batter and the upstream libraries. Batter adapters may
depend on upstream libraries; upstream core crates must not depend on Batter.
Select adapter packages explicitly rather than requiring all integrations for
every consumer. Keep SQLx, Runlimit, and Runledger composition local to an
example until common mechanics justify extraction. Integration fixtures may
depend on the participating libraries; the foundation must not depend back on
them through its generic test utilities.

Before promoting an adapter into a stable public API, use it in two different
application composition roots, exercise partial-startup and shutdown failures,
and document the external version contract. An ergonomic convenience is not a
reason to hide native SQLx transactions or collapse meaningful upstream errors.
