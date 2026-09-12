# Integration ownership contracts

The `batter-axum` and `batter-sqlx` adapters, native SQLx lifecycle example and
unpublished reference command package exist in this snapshot. The latter
composes pinned native upstream APIs in an atomic producer and explicit live probes; see the
[compatibility manifest](reference-compatibility.md).
Runlimit, Runledger, and postgres-test-harness are not dependencies of the
library/test-support crate. The contracts below govern composition; they do not advertise unimplemented
APIs. Delivery scope, acceptance tests and dependencies live in [Beads](roadmap.md).

## Axum: implemented, with a deliberately small boundary

The real HTTP/1.1 [lifetime suite](../crates/batter-axum/tests/http_lifetime.rs)
registers the native serving future with lifecycle supervision, as the executable
does. Ordinary drain directly drives graceful shutdown. Handler admission,
response construction, body transmission, socket closure and direct server
completion are separate observations. The serving result does not aggregate
connection errors, and aborting the serving wrapper does not join spawned
connections. Pending streaming work can survive that report; dependent cleanup
is conservatively skipped after the unsafe direct exit. Full-disconnect tests
cover the resolved transport, not immediate universal propagation or write-half
closure. [ADR-008](adr/008-http-transport-ownership.md) records the measured
contracts and [validation](validation.md) records platform-specific evidence.

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
handlers select their own domain mappings. The HTTP example opts into the
standard infrastructure renderer for both handler and admission failures, with
matching generated request-ID headers/body fields. Readiness requires
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
those fields drive alerts. Status-only probes and unannotated responses retain the
WARN-for-5xx/INFO-otherwise defaults. No response means no override: dropped
futures retain WARN. The HTTP example selects INFO for Starting/Draining probe
responses through `ReadinessPolicy`, leaving Stopped and unhealthy-dependency
probes while Ready at WARN.

Keep authentication, authorization, request body limits, CORS, TLS, proxy trust,
trace-header validation, tenant resolution, and user admission policy external.
Choose middleware order deliberately: Batter's timer starts inside its middleware,
not before an outer queue. The example is GET-only and not an upload/streaming
security template. See [guarantees](guarantees.md).

`register_http_in` registers a bound `TcpListener` and initialized `Router`
through constrained startup authority. Its opt-in companion
`register_http_with_connect_info_in` accepts the same arguments and installs native
`ConnectInfo<SocketAddr>` for direct-peer admission middleware and handlers.
The address and port come from the accepted TCP socket; forwarded headers do not
select identity. Behind a proxy this is the proxy address. Authentication and proxy
trust remain application-owned. `register_http` retains its exact
`&mut Supervisor` signature for lower-level compatibility. All three enter one native
implementation and register a direct critical component. It acknowledges startup on its first task poll; bind
errors remain in owned `Startup`. Registration failure and abandoned startup
release the listener. Native Axum accept errors are retried internally.
The server uses with_graceful_shutdown and waits for it to finish
while dependencies remain alive. Aborting that wrapper is not accepted as proof
of transitive child termination; resource finalizers are conservatively skipped.
The process smoke remains a startup/signal/response check. The separate
[HTTP/1.1 lifetime suite](../crates/batter-axum/tests/http_lifetime_observations.rs) observes
uploads, response streaming, established keep-alive connections and disconnects
through drain/cancellation/abort. Its test-owned socket instrumentation and held
shutdown signal are fixture controls, not adapter API. [ADR-009](adr/009-http-lifetime-observations.md)
records why a normally returned server can permit cleanup after forced request
cancellation, while an aborted wrapper leaves body/socket ownership uncertain and
requires skipped cleanup. The direct result does not aggregate connection errors.

`operational_http` is an opt-in replacement for outer `observe_http` plus identity
glue. It composes exactly one existing observer inside server UUID correlation.
Tower HTTP's native UUID generator is used directly; its header-preserving setter
is unsuitable at an untrusted boundary. Incoming x-request-id values, Tower
RequestId and previous CorrelationId extensions are replaced, as is any inner
response ID. Read `Extension<CorrelationId>` for explicit downstream metadata;
this is never authentication or authority. HTTP completion fields include the ID
even with INFO spans disabled; native operation spans retain their normal filter
semantics. Existing custom identity/observe_http and request_scope remain valid.

`RequestPolicy::with_infrastructure_json` explicitly selects the shared
code/message/request_id envelope. `render_infrastructure_failure` offers the same
mapping to handlers. Neither changes legacy Problem JSON; a later custom renderer
wins. No domain error or schema/codegen dependency moves into the adapter.

`ReadinessPolicy<E>` combines `HealthReader<E>` with `ShutdownHandle` without
probing. Mount `dependency_readiness::<E>` outside admission. The empty-body
200/503 response carries a typed reason and a separate severity extension.
Unknown, failed, timed-out, stale and stopped-writer dependency states are unready;
Starting/Draining default INFO, Stopped/dependency failures default WARN.
`with_level` overrides severity only. The final lifecycle read overrides cached
health on observed drain; this decision is not atomic with subsequent transitions.

The [HTTP composition root](../crates/batter-axum/examples/http_service.rs) deletes
its local ID, renderer, readiness and serve implementations in favor of these
helpers. Authentication/metadata policy in the reference consumer remains
separately owned by batter-in2. [Real socket tests](../crates/batter-axum/tests/operational/serving.rs)
prove graceful startup/drain and demonstrate an outstanding stream surviving
wrapper abortion with cleanup skipped; they do not establish a general body,
WebSocket or disconnect ownership contract.

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

For a pool created inside a Batter owner, reserve its cleanup name first and call
`pool_in(slot, PgPoolOptions, PgConnectOptions)`. The synchronous return means the
awaited close hook has already been published; it does not mean the database was
contacted. Perform a bounded query or `probe` before readiness. SQLx can begin
minimum-connection maintenance during lazy construction, and pool close waits for
accounted checkouts, so join dependent work and release leases before cleanup.
The adapter-owned `owned_pool` example demonstrates this finite `Command` path.

The [service example](../examples/postgres-lifecycle/src/main.rs), packaged as
`batter-example-postgres-lifecycle`, uses native PgPoolOptions and the optional
`batter-sqlx` bounded probe and Pool::close registration. It registers close as a dependency finalizer and
uses `Startup` for owned initialization and startup-error cleanup. It reserves the
pool finalizer name before connecting and registers closure immediately after
acquisition. Protected application roots can instead select `Startup::scoped(...)
.with_unix_signals("signals")`, which installs listeners before the owner is
returned and removes the initializer's manual reception/handoff obligation.
`register_signals` remains the lower-level compatibility helper. Pool sizing,
probe and cleanup budgets remain application choices. Its fixed process diagnostic retains concrete early
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

The reference service now demonstrates that contract for delivery of a versioned
generic record. A unique `(owner_id, idempotency_key)` command row, delivery row,
and native Runledger enqueue share one READ COMMITTED transaction and one
`PgLease`. The command retains its canonical record/generation/JSONB and immutable
enqueue inputs. Exact replay reads that committed identity without enqueueing
again; changed input conflicts. The stable delivery UUID namespaces Runledger's
key, while the authenticated owner becomes its `organization_id`.

Only acknowledged commit or rollback returns the connection to the pool. Failed
commit/rollback acknowledgement or interruption after `BEGIN` retires it and
produces an uncertain response. `GET /delivery-commands/{idempotency_key}` lets
the same authenticated owner reconcile without a response-generated identifier;
absence during settlement is not rollback evidence. This initial command has no
automatic transaction replay, worker handler, provider effect, or exactly-once
claim. The later controlled concurrency/fault suite owns stronger proof.

## Runledger: optional native lifecycle adapter

The three native Runledger packages use one immutable Git revision, with no
sibling path overrides; see the exact source and evidence limits in the
[compatibility manifest](reference-compatibility.md). `batter-runledger` consumes
inert `PreparedSupervisor` values. Native supervision, durable policy, claim
behavior, observers, schedules, workflows and retries remain upstream.

Protected startup passes its sealed registration target to
`batter_runledger::register_in`; the exact legacy `register(&mut Supervisor, ...)`
signature remains and shares the same implementation. The adapter translates local loop initialization, earliest stop time and complete
native settlement into Batter managed-component ownership. Native joins survive
direct waiter abortion. Reports retain later errors and unresolved descendants;
Batter skips dependent cleanup when settlement is unproved. Absolute phase
budgets exchange the earliest native/parent stop time, including tightening an
already-active wait. No application termination gate or independent join driver
is required. In-flight claims can still dispatch after stop; arbitrary detached
handler descendants and remote server sessions are not covered by native joins.

The reference combines native initialization with fresh PostgreSQL health and
explicit application approval. The production registry is empty and approval
remains withheld until the real delivery handler is installed. Durable execution
proof belongs to isolated tests. Startup performs no control-job enqueue, advisory
lease, epoch allocation or reconciliation loop.

The separate offline retirement command disables the exact legacy native job
definition and cancels its remaining pending/leased global controls. It requires
external restart revocation plus verified identity and actual session/transaction
quiescence, including hidden sessions and prepared transactions. It refuses a
replacement mutating session and preserves history, migrations and sequence.
Native cancellation retains SQLx causes and secondary rollback failure. Optional
readback is a separate owned read-only command: it cannot rewrite a failed or
ambiguous primary result. Neither database snapshots nor an advisory lock prove
external restart revocation. See [retirement usage](../examples/reference-service/README.md).

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

`PoolAcquire(Arc<sqlx::Error>)` means registered pools await driver cleanup. Its
cause also remains in `DatabaseCleanup::pool_failures` if handled by the body. Low-level `Connect`
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
invoke destructive upstream lease Drop. `batter-kjl` tests both live-driver waiter loss and actual runtime loss: the
latter can queue deletion through native lease Drop while a checkout is held.

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


### Optional fixture session observation

Install `FixtureSuite::with_session_observer` before `start`. The shared
`SessionObserver` takes a caller-budgeted admin pool on the same server, outside
the disposable databases, and a positive attempt deadline. After tracked pools
close, it requires database presence and absence of all database sessions,
including detached ones. Applications must stop new connection producers first;
this read does not fence a later connection. The minimal reference fixture uses
two separately bounded one-slot admin pools: session observation and independent
body/catalog diagnostics. Initializer pools remain caller-owned.

A timeout or native observation failure retains the lease and appears in
`cleanup_progress`; independent databases keep cleaning. Repair the cause and
call `SessionObserver::retry_with` to resume parked observations, including with a
replacement admin pool. Failed attempts remain in `observation_failures` after
recovery. `FixtureRun::wait_for` returns `Ok(None)` for pending observation; only
`wait`/`into_report` yields a completed report. Neither pending nor driver join
means clean reuse. Losing the retry control can strand the live driver; destroying
its runtime can run destructive native lease Drop. No native close error is
invented for SQLx's unit-returning `Pool::close`.

Retry replacement does not close the old pool's sessions. The shared completion
owner exposes retained native pools through `observer_pools()` for explicit repair.
If one accidentally connects inside a disposable database, close it and witness
backend exit before requesting the corrected retry. The wrong-target regression
recovers the same pending owner without keeping an external wrong-pool clone.
Automatically closing replaced pools would disrupt
active attempts or other runs sharing those pools.

The unpublished fixture API now shares PoolAcquire's payload through Arc and adds
per-database pool/observation failure collections. Workspace consumers and struct
literals move together; downstream source consumers must adapt payload matching
to `as_ref()` and inspect the new collections. Domain/native causes retain their
concrete identity. Generic `Script` and `finish` remain unchanged leaf helpers.

The reference consumers share `ObservedRun::finish` with a 30-second whole-run
wait budget. Pending returns a redacted phase/count error owning the original
`FixtureRun`, retry control, all session pools and the diagnostic pool. Downcasting its concrete cause
permits repair and another bounded finish; no retry is automatic. The private retry method retains each replacement pool with its control;
completed runs close all session pools. The caller must retain a diagnostic clone;
after a successful driver join, that clone stays open for caller-owned catalog
checks and awaited closure. A terminal
driver JoinError instead closes diagnostics in the completion helper before
propagation. The same wait budget covers these administrative closes; a blocked
close retains the cached report/JoinError and pools in the pending owner. Discarding a pending error loses observation
and recovery control while the live driver continues; destroying its runtime
can still invoke destructive native lease Drop. This is a consumer test-failure
policy, not a cleanup-completion or async-drop guarantee.

Retry requests broadcast across all databases/runs using the same SessionObserver.
Unread requests coalesce. A request during an active attempt does not cancel it;
if that attempt fails, the unread request authorizes the next attempt immediately.
A successful attempt needs no retry. The first attempt uses the latest pool.
The budget covers connection acquisition and every catalog poll: timeout means
absence was not established, not proof that sessions were present.

The reference test boundary (`assert_probe`) panics on an unrecovered pending
error. Its per-test runtime can then be destroyed and native lease Drop can
submit FORCE deletion; a bounded failed test is not a retained-runtime cleanup
guarantee. Callers needing recovery must keep the runtime and the typed pending
owner alive. A missing database remains ObservationTarget/pending. This catches
the wrong-server test's missing name, but an identical name on a different cluster
could pass; the caller must select the actual harness server. No API treats
external deletion as successful owned cleanup. The native pg_stat_activity catalog
and its normal session-identity visibility are required; a substituted filtered
view is outside the contract. Session absence does not inspect prepared transactions,
active logical replication slots or subscriptions. Those can still prevent native
DROP and appear as consuming-cleanup errors.

The reference suite exercises the missing-target branch against a distinct
PostgreSQL cluster, restricted-login visibility, cross-run retry and pool closure,
pre-first-attempt replacement, template recovery and both Tokio runtime flavors.
Its SCRAM startup negative control establishes a narrower boundary: a backend
can exist with no assigned database name, allowing observation to advance to
native lease cleanup while that connection is still starting. PostgreSQL 18's
subsequent DROP may wait on its process-signal barrier. The test closes the
startup connection and awaits the same driver before claiming completion.
A separate real autovacuum worker keeps observation pending until that worker
exits and the caller explicitly retries. Stop connection producers, including
startup attempts, before body exit; native cleanup phases are not completed reports.

`DatabaseProgress` Debug follows report privacy: only phase and failure count are
formatted; names and native causes remain available for deliberate typed inspection.
The detached-session probe transfers its session pool to the completion owner and
has no outer close on a pending result. Its two-second attempt gives backend exit
more margin; the five-second witness deadline remains independent.

Parked leases also retain upstream admission permits. A shared harness whose
capacity is exhausted by pending observations cannot admit further leases until
recovery releases capacity; observe and repair existing runs before awaiting new
ones. A shared retry control does not create additional capacity. Pending
reference summaries distinguish `driver_joined=false` from admin closure after
a joined driver, and separately report `driver_failed` without native contents.

Reference completion starts all retained observer-pool closes concurrently. An
older pool's held checkout cannot postpone marking later replacements closed.
The pending owner still retains every pool and the cached driver outcome until
explicit recovery; the timeout grants no remote-cleanup or async-drop guarantee.

## Application settings constructor handoff

The foundation's `settings` module has no SQLx, Axum or Runledger dependency.
The runnable HTTP example uses its literal source reader and bounds and passes
its selected capacity into the actual router. The reference package owns
`config::{RootSettings, PoolSettings, WorkerSettings, ConfigMode}`. Its constructors
return native `RequestPolicy`, `PgPoolOptions`, `PgConnectOptions`, `Bulkhead`,
Batter `Supervisor`, and `JobsConfig`/Runledger `SupervisorBuilder`.

Use `RootSettings::from_process(mode, selected_path, overrides)` once before
acquisition. Tests can inject file/environment/override sources directly.
Precedence is defaults < explicit file < captured environment < explicit
in-memory overrides. Dedicated files/overrides reject every unknown key;
environment ignores unrelated names but rejects unknown BATTER_/JOBS_ names and
all PG* entries. `connect_options_from_process` also rejects actual PG* entries even when the
settings loader was injected. Keep process environment unchanged during native
construction; no passfile, native URL fallback or `JobsConfig::from_env` path is
used. The application owns schema names and password/TLS policy; see the
[reference settings schema](../examples/reference-service/README.md).

The reference fixture paths and delivery command root consume these outputs.
The command root uses configured authentication, request deadline, pool,
Bulkhead and finite-process constructors; live held-work cases prove their
runtime effects. Its worker transfers native preparation to `batter-runledger`,
preserving native intent-promoter inheritance. Each enabled loop acknowledges
local initialization; fresh health and application approval remain separate.
The production registry omits `records.delivery.execute`, and readiness stays
unapproved until the provider task installs and verifies its real handler.
Authorized external migration remains `batter-7r3.6`.
The producer does not introduce Runlimit settings or an alternate job supervisor.

Acquire inside existing `Startup`, reserving cleanup before acquisition and
registering immediately after success. Pool close returns unit; retain native
acquisition/initialization errors and every cleanup record without inventing a
SQLx close error. Keep the external fixture lease owned until pool finalization
has completed, then await lease cleanup and deferred drain. The new live startup
probe follows this order; offline file-resource tests retain a real acquisition
error alongside a separate real cleanup failure.

The adapter requests native shutdown at Batter drain and exchanges the earliest
stop time. Native settlement retains all tracked joins outside the direct waiter;
Batter owns the conservative dependent-cleanup decision. Application code does
not add nested cleanup drivers or infer stopped descendants from wrapper exit.

Live reference preflight and fixture acquisition share the application validator
through a private live-endpoint policy. Preflight authenticates with the same
explicit SQLx options as the configured startup probe. Both endpoint settings
validate before either connection opens; the native checks retain primary
superuser/autovacuum/track_counts requirements and reject equal signed cluster
identities. Each read-only pool closes before its result is interpreted. The
entrypoint returns only a fixed success marker or sanitized error. The validated URL
handed to the harness encodes query `+` as `%20`, so the harness's native
tokio-postgres parser receives the same spaces as the root and SQLx. It also
serializes the validated hostname and database, canonicalizes the disabled TLS
mode and retains an explicit empty password instead of permitting SQLx passfile
fallback. IPv6 literals are rejected at this private live boundary: the locked
SQLx URL parser retains brackets during TCP lookup. Use `localhost` or
`127.0.0.1`; direct root construction still supports native IPv6 options. Userinfo
`+` and existing percent escapes retain their meaning. The Python runner
owns budgets and exact case inventory, not endpoint parsing or credential
fallback. Ordinary native-constructor tests cross a cleared-environment child
boundary, and the matrix requires success with hostile PG* parent variables.
The acquisition, cleanup-failure, pool-failure and primary/secondary observation
fixture roots all consume the shared handoff; their custom budgets do not re-read
the original URL.
