# Integration ownership contracts

The `batter-axum`, `batter-sqlx`, `batter-runledger` and `batter-runlimit` adapters, native SQLx lifecycle example and
unpublished reference command package exist in this snapshot. The latter
composes pinned native upstream APIs in an atomic producer and explicit live probes; see the
[compatibility manifest](reference-compatibility.md).
Runlimit, Runledger, and postgres-test-harness are not dependencies of the
library/test-support crate. The contracts below govern composition; they do not advertise unimplemented
APIs. Delivery scope, acceptance tests and dependencies live in [Beads](roadmap.md).

## Facade feature selection

Applications can use `batter` as the single public import root. Its default
feature set is empty and re-exports the foundation from `batter-core`. Select
integration namespaces explicitly:

| Facade feature | Public path | Selects |
| --- | --- | --- |
| `axum` | `batter::axum` | `batter-axum` |
| `sqlx` | `batter::sqlx` | `batter-sqlx` |
| `runledger` | `batter::runledger`, `batter::sqlx` | `batter-runledger`, `batter-sqlx` |
| `runlimit` | `batter::runlimit` | `batter-runlimit` |
| `test-support` | `batter::test_support` | `batter-test-support` |
| `sqlx-test-support` | `batter::sqlx::test_support` | SQLx adapter fixture support plus generic support |

`runlimit-memory` and `runlimit-postgres` forward only the existing native
error bridges through `batter::runlimit`; neither selects the other backend or
HTTP. `runlimit-axum` enables `batter::axum` and
`batter::runlimit::http`. `runledger` also enables `batter::sqlx` because its
protected transaction bridge requires `PgSession`; native SQLx selected only by
`runlimit-postgres` does not expose that namespace. `axum+runlimit` does not
expose the Runlimit HTTP module. Native versions, TLS/runtime settings, storage,
fixture provisioning, and policy remain owned by the adapter or upstream package.

The facade feature proof runs in temporary external workspaces because the
workspace's all-feature build unifies optional dependencies. The bounded
runner checks every declared feature, the representative unions, graph
optionality, focused disabled imports, and one direct/facade identity fixture.
The direct Runlimit runner separately retains its eight native combinations.

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
contracts.

Import `ResponseConstructionBudget`, `RequestPolicy`, `ReadinessPolicy`,
`ProbePath`, `GuardedRouter` and `HttpBoundary` from `batter_axum`. Validate the fixed server budget and
each literal probe path before
composition, then pass the retained witness and the supervisor's
`OperationAdmission` projection to the infallible policy constructor.
`HttpBoundary::new(policy)` mounts liveness and readiness probes outside
admission, applies admission to every guarded route plus its default, custom,
nested and method fallbacks, and installs server correlation with the single
HTTP observer outermost; the caller cannot reorder those layers. Build guarded
routes through `GuardedRouter`, whose route, merge and typed-nesting operations
retain an inert identity inventory; opaque nested services remain outside this
protected path. Await `assemble`: it rejects inventory routes matching a
reserved probe path before Axum merge, without polling application code. Observation
covers public probes without admission and covers guarded fallbacks plus
rejection responses inside admission. The individual
middlewares remain available for compositions the boundary cannot express. Domain
services receive their own dependencies through State/FromRef/constructors;
request operation context arrives through Extension<OperationContext>.

Import browser-carried credential mechanics from `batter_axum::browser`.
`BrowserOrigin` validates trusted HTTPS or explicit loopback configuration;
`read_cookie` scans every Cookie field for one exact target; `BrowserCookie`
appends fixed host-only/root-path set and removal fields; `MutationPolicy` checks
configured exact Origin/custom-marker signals with optional Fetch Metadata on
origin-only policies, automatic strict same-origin Fetch Metadata on every
marker policy, and an optional JSON guard; `private_response` overwrites the
three fixed private headers on inner responses. Applications still own
credential/token meaning, account and session state, authorization, CSRF-token
design, CORS/proxy trust, route selection, revocation, and response rendering.

Apply observation after every route/fallback is assembled: Axum's router layer
runs after routing and only covers existing routes. A later-added route bypasses
it; a service wrapper outside routing has no matched route template at entry.
Place rejecting/status-changing middleware inside observation so its response
is covered. See the [compiling composition example](../crates/batter-axum/src/lib.rs)
and [HTTP example](../crates/batter/examples/http_service.rs).

`with_failure_renderer` maps middleware failures into an application-owned
envelope using a snapshot of request parts. Trusted correlation middleware must
run outside it; never assume raw inbound headers are trusted. Application
handlers select their own domain mappings. The HTTP example opts into the
standard infrastructure renderer for both handler and admission failures, with
matching generated request-ID headers/body fields. Readiness requires
all registered components to acknowledge startup before traffic is admitted.

`RequestPolicy` retains one combined readiness/deadline policy. The independent
observer requires neither that policy nor lifecycle state. `request_scope` keeps
the combined observation/admission behavior for compatibility. Only the outermost
observer emits a completion event; observation and operational ownership use
distinct request extensions, so a plain outer observer cannot suppress inner
server-generated correlation while stacked observers cannot duplicate HTTP
events. Admission and deadlines do short-circuit: `operational_http` must be
outside `request_scope`, `request_admission` and other rejecting middleware if
every outcome requires its generated identity. All entry points use
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

`AssembledHttp::register_in` registers a boundary-assembled router through
constrained startup authority; `register_http_in` accepts a plain bound
`TcpListener` and initialized `Router` for compositions outside the boundary. Its opt-in companion
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
A plain observer can wrap `operational_http`, but `request_scope` is supported
only inside it because admission rejection and deadline expiry can return
without polling inner middleware. Nested operational/quota wrappers still share
one completion event and one server-generated correlation identity in either
of their orders.

`RequestPolicy::with_infrastructure_json` explicitly selects the shared
code/message/request_id envelope. `render_infrastructure_failure` offers the same
mapping to handlers. Neither changes legacy Problem JSON; a later custom renderer
wins. No domain error or schema/codegen dependency moves into the adapter.

Foundation `ReadinessEvaluator<E>` combines `HealthReader<E>` with
`LifecycleStatus` without probing. It explicitly classifies each broad health
observation into `DependencyReadiness`, then returns `ReadinessDecision::Ready`
or `Unready(ReadinessUnreadyReason)`. A dependency reason accepts only
`DependencyUnreadyReason`, never Healthy. `ReadinessPolicy<E>` wraps that evaluator
for Axum; mount `dependency_readiness::<E>` outside admission. The empty-body
200/503 response carries the typed decision and a separate severity extension.
Unknown, probe-failed, probe-timed-out, stale and stopped-writer dependency states
are unready; Starting/Draining default INFO, Stopped/dependency failures default
WARN. Reuse `readiness_status` for the adapter's 200/503 mapping and
`default_readiness_level` when a `with_level` callback overrides only selected
decisions. Response extensions contain `ReadinessDecision`; import
`ReadinessUnreadyReason` from the foundation only to match its unready case. The final
lifecycle read overrides cached health on observed drain; this decision is not
atomic with subsequent transitions.
`RequestPolicy` separately accepts `OperationAdmission`, which can create only a
readiness-gated downward-cancelled operation context. Neither policy retains
`ShutdownHandle` or can request shutdown or approve readiness.

The [HTTP composition root](../crates/batter/examples/http_service.rs) deletes
its local ID, renderer, readiness and serve implementations in favor of these
helpers. Authentication/metadata policy remains application-owned.
[Real socket tests](../crates/batter-axum/tests/operational/serving.rs)
prove graceful startup/drain and demonstrate an outstanding stream surviving
wrapper abortion with cleanup skipped; they do not establish a general body,
WebSocket or disconnect ownership contract.

The reference consumer composes those helpers as
`operational_http -> direct-peer metadata -> bearer authentication ->
request_admission -> handler`. Its `TrustedPeerPolicy` has one explicit mode:
trust the IP from native `ConnectInfo<SocketAddr>` and ignore every forwarding,
trace and client request-ID header. The application-owned `http::register_in`
operation constructs that router and selects
`register_http_with_connect_info_in` together; the production root does not make
an independent transport-registration choice. Serving settings pin this policy
in code and carry it through preparation; no environment setting selects a trust
mode. The lower-level `in_process_client` returns an opaque non-service type and
requires a synthetic peer for every request; it owns insertion of the exact
`ConnectInfo<SocketAddr>` and cannot expose or serve its inner router.
`MockConnectInfo` affects extractor
fallback only and is not read by this middleware. No proxy allowlist/CIDR mode
or forwarded-header parser exists. Liveness and readiness are merged outside
that business boundary and therefore do not require peer metadata. Axum path,
JSON and body-limit extractor rejections remain native transport responses, not
application problem envelopes; the outer operational layer still assigns their
response correlation header.

`TrustedRequestMetadata` carries the shared adapter `CorrelationId` and opaque
direct peer only and intentionally has no public constructor. `OwnerId` remains
separate authority selected by the configured
bearer credential, and `OperationContext` remains the request deadline/
cancellation capability. Handler and error helpers receive metadata explicitly;
domain/authentication and shared infrastructure bodies use its typed correlation,
while the outer adapter alone sets the response header and records completion.
The public `TrustedPeer` value is the application-owned admission input required
by the downstream Runlimit composition, but no quota identity or grant is
inferred in this stage. Durable persistence remains with
`batter-7g0`; the new Runlimit adapter does not change that reference-service path.

## SQLx: owned transactions and session operations

For atomic application/library composition, use `run_atomic(&pool, async |scope| ...)`.
The runner withholds output until acknowledged completion and owns session retirement.
Its application operations own savepoint recovery and XID validation;
downstream code receives only `PgScopedSql::executor()`. `PgReadOnlySnapshot`
owns generic coherent read-only inspections. See the
[transaction contract](../crates/batter-sqlx/README.md#owned-transactions-and-snapshots).

`batter-sqlx` is independently selected. `PgLease::acquire` bounds acquisition
under an existing OperationContext. Move the lease into the operation future and
run its work through `lease.with_connection(async |session| ...)`: after `Ok`,
Batter executes `ROLLBACK` on that physical connection and returns it only if
the idle-state synchronization succeeds. `Err`, failed or interrupted cleanup,
or a dropped future retires it, so an interrupted query cannot be followed by
pool return. There is no direct pool-return call. Use the consuming
`lease.with_retiring_connection(async |session| ...)` path when every outcome
must retire. `PgSession::executor` is low-level SQL without atomic-result
guarantees. There is no public session `begin` or `PgTransaction`; use `run_atomic`
for writes whose output requires acknowledged disposition. Exceptional manual
ownership is only `low_level::PgAtomicTransaction`. The session exposes no
replaceable SQLx connection. Native execution can issue raw `BEGIN` without changing SQLx's typed
depth; the return-time rollback handles open and failed raw transactions before
the private return proof exists. It is not a general session reset. A lease
retained outside a cancelled future remains the caller's responsibility.

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
contents. This adapter adds no database provisioning or retry policy.
`register_pool_close` retains caller ownership on
registration failure; the caller must explicitly close that pool.

For a pool created inside a Batter owner, reserve its cleanup name first and call
`pool_in(slot, PgPoolOptions, PgConnectOptions)`. The synchronous return means the
awaited close hook has already been published; it does not mean the database was
contacted. Perform a bounded query or `probe` before readiness. SQLx can begin
minimum-connection maintenance during lazy construction, and pool close waits for
accounted checkouts, so join dependent work and release leases before cleanup.
The adapter-owned `owned_pool` example demonstrates this finite `Command` path.

The [service example](../examples/postgres-lifecycle/src/main.rs), packaged as
`batter-example-postgres-lifecycle`, selects `Startup::scoped(...)
.with_unix_signals("signals")`, which installs listeners before the owner is
returned and removes any initializer reception/handoff obligation. Its
initializer parses `DATABASE_URL` into native `PgConnectOptions`, reserves
`postgres.pool`, calls `pool_in` with native PgPoolOptions, and then runs the
bounded `probe` under a child of the root startup context; construction alone is
not connectivity. The reference root shares the `pool_in` ownership path but
establishes connectivity by directly acquiring a connection and then initializing
its schema under the overall startup context. The finite retirement command passes
its `CommandScope` slot and native `after_connect` guard to `pool_in` unchanged.
`register_pool_close` and
`register_signals` remain lower-level compatibility helpers. Pool sizing,
probe and cleanup budgets remain application choices. Its fixed process diagnostic retains concrete early
errors, the startup cleanup report, or the complete failed shutdown report in its
source chain. No database abstraction or generic transaction retry is introduced.
The example uses SQLx 0.9.0, which requires Rust 1.94 or newer. Compiling the
example does not establish pool closure.

Application-owned migration/schema checks happen before readiness. There is no
automatic migration during a health probe. Size pool capacity alongside admitted
HTTP work, Runledger concurrency, harness test admission, and maintenance tasks.
A semaphore bound is not a database-capacity calculation by itself.

Use `batter_sqlx::verification::verify` with the actual serving `PgPool`, its
`OperationContext` and application policy before readiness. Acquisition,
transaction-state normalization, inspection, rollback and lease disposition are
library-owned. Do not substitute owner credentials or duplicate cancellation and
lease-return logic around the checker. Pool session options and serving identity
are preserved. Catalog SQL uses a trusted transaction-local search path, restored
by rollback, so an application helper cannot redirect inspection. Thread the existing context through the application facade;
protected startup internals need no additional exposure.

For applications with separate schema and privilege stages, use
`verify_migrations` with `MigrationPolicy` and `verify_authority` with
`AuthorityPolicy`. Setup credentials can perform migration checks without a
fabricated authority allowance, and missing required ledger privileges can be
reported without reading that ledger. These bounded calls need not share a
transaction with application-specific checks. Their reports disclose different
coverage.

Keep per-profile required and allowed privileges in application policy data.
Required grants describe current-role direct/PUBLIC/INHERIT access; excess checks
include authenticated-login SET/ADMIN potential. Select schema discovery plus
per-kind defaults to catch unexpected supported objects, then use exact object
and PUBLIC overrides for exceptions. Check native application behavior alongside
policy findings; an ACL is not proof of RLS-visible rows or function safety.
The shared implementation bounds catalog rows/bytes, locks a migration ledger
before its snapshot, rejects ledger RLS, and restores local transaction settings
before reuse. It does not migrate, repair grants, or acknowledge remote session
termination. See the [canonical adapter example](../crates/batter-sqlx/README.md#read-only-schema-and-authority-verification).

Use `ExactRoleManifest` when one grouped, application-owned role declaration
should feed both startup verification and an operator-visible grant plan. Compile
once, pass the compiled value to `verify_exact_role` or
`VerificationPlan::exact_role`, and render `compiled.grant_plan()` only at an
explicit operator boundary. Its underlying authority policy is intentionally
private so protected safeguards cannot be dropped. Required
and provisioned privileges enter both outputs; allowed-only ceilings, ownership,
grant options, discovery defaults and PUBLIC policy do not become grants. The
renderer performs no I/O and deliberately omits role creation, revocation,
credentials and transaction control. The application supplies the target role,
the database name only when needed, surrounding transaction text and any global
PUBLIC-schema policy.

Use `SqlxMigrationManifest` for an exact or installed-subset SQLx 0.9 ledger and
`SchemaInspectionPolicy` for the exact stored `search_path` on every definer in
explicit existing schemas. Unrelated stored settings are accepted only within
the bounded captured configuration inventory. A non-empty `VerificationPlan`
has three independently optional axes: generic authority or a compiled exact role, generic
or SQLx migration inspection, and optional schema inspection. Any supported
combination runs in one protected transaction; schema inspection may also be the
required first component under setup credentials. A second component on the
same axis is rejected during plan composition. A migration ledger is a relation;
composition also rejects authority that declares that exact schema-qualified
identity as a sequence, regardless of which axis was selected first. Mutable generic authority
configuration remains an `AuthorityPolicyBuilder`; execution accepts only its
successfully built immutable policy. If the captured database has the opposite
relation kind, retain the ordinary missing-object or missing-privilege finding; discovery still evaluates
the actual object under defaults for its observed kind. Do not reinterpret this
drift as a construction error or combine both kinds in downstream policy data. Opt into
the compiled role's current-database ownership denial when that profile must own
no local object. The generic coverage intentionally does not choose migrations,
provision roles, inspect durable application protocols, interpret SECURITY
DEFINER bodies or trigger execution, or inspect other databases. Exact role
declarations remain local application data even when their compilation is shared.
Reachable superuser or predefined-role capability other than the implicit
`pg_database_owner` leaves the ownership-specific result incomplete because
pinned-role ownership dependencies are not exhaustive. An impossible stored
membership into or out of `pg_database_owner`, a missing database, owner, or
membership role identity, or a malformed retained dependency is likewise
incomplete, not an implicit-owner fact. The verification example's
superuser opt-in disables this narrower ownership conclusion accordingly.
SECURITY DEFINER bodies, extension
semantics and role defaults are explicit unsupported report surfaces; selected
ACLs on extension-owned objects are still checked. Add the corresponding
`AuthorityPolicyBuilder::required_surfaces` value when an application requires one of
those unsupported surfaces; that requested boundary produces `Incomplete`
instead of a passing result. Declared relation policies can explicitly allow
their composite row type's default PUBLIC `USAGE` with
`RelationPolicy::allow_row_type_public_usage`; unexpected row-type ACLs remain
visible when that flag is false. Direct role grants on a row type and standalone
user-defined types remain explicit `TypePolicy` entries.
Automatic array and multirange aliases use their element/range authority; they do
not acquire independent default PUBLIC grants. Requested parameters with hidden
metadata produce `Incomplete` with `ParameterVisibility`, preserving known findings.
Unknown custom-parameter definitions likewise cannot satisfy required privileges;
their conservative potential SET authority remains separate from positive proof.

For the durable reference path, application writes and Runledger submission must
use the same database transaction through Batter's opaque transaction capability.
Preserve upstream error distinctions,
especially outcomes around commit. A caller deadline or lost connection does not
prove rollback. Do not turn an ambiguous outcome into a generic "retryable"
Batter error. Implement application idempotency/reconciliation separately.

The reference service uses Runledger's READ COMMITTED `run_atomic` for
the command row, application delivery and Runledger enqueue. Scoped application
and domain operations preserve the transaction's birth identity. Exact
replay reads the committed command identity without enqueueing again; changed
input conflicts. The runner releases results only after commit/rollback acknowledgement. Interruption
before acknowledgement retires the connection and leaves uncertainty; no
asynchronous cleanup follows acknowledged completion. The operation boundary
retains known completion evidence before telemetry is finalized.
`GET /delivery-commands/{idempotency_key}` lets
the same authenticated owner reconcile without a response-generated identifier;
absence during settlement is not rollback evidence. The command performs no
automatic transaction replay. Its worker separately owns one selected provider
effect protocol; neither layer makes an exactly-once claim.

The provider row is inserted in that same transaction with a stable key derived
from the delivery UUID and a canonical versioned request containing owner, record,
generation and payload. The handler commits `reconcile_needed` before polling the
POST. A lost response, timeout, cancellation or worker death after that commit is
not retry permission: the next attempt performs keyed lookup first. Exact retained
acceptance confirms the original effect; retained-window absence permits the
original POST; mismatch or expiry becomes manual resolution. The local protocol
assumes 24-hour key retention. The application binds the SQL `resolve_before`
interval from `IDEMPOTENCY_RETENTION`; the database does not carry an independent
retention literal. Accepted retry delays have a separate
`MAX_ACCEPTED_RETRY_DELAY` policy, currently also 24 hours. Changing one policy
does not change the other. It proves no behavior for an arbitrary provider.

The handler first loads the retained effect state. Terminal states return without
provider admission; uncertain state reconciles before target-generation policy.
Only retained-window absence permits replay. Every effect mutation constrains its
legal source state in SQL, so local lifecycle events cannot downgrade uncertainty
or overwrite terminal truth.

Supported retained-data invariant failures and request-construction failures are
projected to `manual_resolution` under the same live native lease before the
handler returns a terminal failure. If that application update loses its lease,
the pinned Runledger completion predicates reject the stale handler outcome too.
This is application-owned failure projection, not a queue transaction or a claim
that arbitrary database tampering can be repaired.

Provider `Bulkhead` admission then waits inside the existing provider work budget
using the purpose-specific `BATTER_PROVIDER_CAPACITY`. Runledger has already
claimed the job and offers no attempt-neutral defer/refund, but its validated
global handler concurrency also bounds the number of provider waiters. This
durable-work composition avoids request-style immediate rejection without adding
a second claim engine or an unbounded waiter population. Admission interruption
records a source-state-specific outcome: pre-dispatch states remain known
undispatched, while `reconcile_needed` retains its acceptance possibility and
resolution deadline.

## Runledger: optional native lifecycle adapter

The five native Runledger packages are local workspace members under `runledger/`.
They share the facade's SQLx foundation without a root patch or sibling checkout.
The [compatibility manifest](reference-compatibility.md) records source provenance
and evidence limits. `batter-runledger` consumes inert `PreparedSupervisor`
values. Native supervision, durable policy, claims, observers, schedules,
workflows and retries remain owned by Runledger's distinct packages.

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
explicit application approval. The production registry contains the delivery
handler and grants ordinary approval after registration. Durable execution proof
belongs to isolated tests. Startup performs no control-job enqueue, advisory
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

## Runlimit: optional protected native quota adapter

`batter-runlimit` pins native core/memory 0.3.0 and PostgreSQL 0.3.1 at the exact
revision in its Cargo manifest. There are no default features. `memory`
enables fixed-window/GCRA error bridges; `postgres` enables the native error
bridge and canonical outcome-aware attempt runner; `axum` selects HTTP assembly.
Policies, hashed subject keys, atomic batch validation, quota algorithms, storage,
transactions and migrations remain Runlimit's responsibilities. PostgreSQL
initialization and maintenance remain application/native-owned. The reference
service has not adopted this adapter. Explicit live attempt acceptance and its
limits are documented in the adapter README and testing guide.

`Quota::run(context, Checks::new(&checks)?, work_factory)` performs one native
`Limiter::check_all`. An empty protected batch is rejected before execution.
Constructing `Quota::run`'s future does not invoke the native limiter; a
never-polled operation cannot consume quota. At this pin, native `MemoryStore`
and `GcraStore` trait checks also defer evaluation until their futures are polled;
the focused Batter regression exercises both single and batch `MemoryStore`
checks. Once polled, the native check may take effect before yielding. Batter
does not expose the native future through its protected operation or infer
rollback from dropping an in-flight check.
Denial and backend failure never invoke the work factory. Allowed and shadow
decisions remain separate from the typed work result. `RunResult::Rejected`
contains the native denial, its batch index and the validated nonzero evaluated
batch size. `RunResult::Admitted` contains either an allowed batch of native
validated `Allowance` values, or a shadow denial with its native index, nonzero
batch size and quota details. Enforced members cannot occur in the allowed batch
type. Interruption
reports only `NotStarted` or `InFlight`; neither implies rollback. Admission and
work share
the parent's total deadline; cancellation after a grant can prevent work without
erasing the grant. No retry or refund is automatic. Inner work retries do not
repeat quota admission. A started check without an observed result means unknown
consumption, not proof of continued local execution or remote rollback.

`HttpQuota::new(quota, policies, authenticate, subject)?.prepare(policy,
protected_routes)` rejects an empty policy set or mixed enforced/shadow modes
at construction, then owns lifecycle/deadline -> async authentication -> native
atomic batch -> body/handler ordering. Subject-dependent native batch validation
stays with Runlimit. Every route passed to `prepare` is guarded. To expose a
public probe, call `.with_public_probes(PublicProbes::new().get("/live", handler)?)`
before `prepare`; the default has no unguarded handlers. A public GET probe
that shares a path with a protected GET route makes `prepare` panic during
Axum route assembly, before serving starts. Declare distinct GET paths. The supplied protected
`Router` is treated as a complete service: its custom root fallback, nested
fallbacks and method fallback pass the same admission, authentication and quota
gates as ordinary routes. An explicitly supplied protected root fallback is
retained; without one, unmatched paths receive the default unguarded 404. The
public builder accepts only literal GET paths (with Axum's implicit HEAD) and
returns a typed error for capture or wildcard patterns. It cannot accept a
Router or fallback. Unsupported methods on a declared probe path get
Axum's default 405 without an application fallback. Public probes bypass
lifecycle/deadline admission, authentication and quota; they receive no protected
`OperationContext`, but retain root observation and correlation. Unavailable
lifecycle admission therefore precedes a 401 only on protected paths. The auth
factory gets owned headers, direct transport peer and the request
context, not a body or untrusted principal extension. The subject selector gets
only the authenticated principal, direct peer and native policy. It must return
native opaque keys; closure signatures are checked at `new`. Handlers extract
`Authenticated<P>`, whose constructor is private; raw `Extension<P>` is never
authentication evidence and can be overwritten by unrelated middleware. No
forwarded-header/proxy trust is inferred. Behind a proxy,
the direct peer is the proxy. Authorization remains application policy;
pre-authentication attempts use the separate `AttemptRunner` described below.

One batch is intentional: stacked native single-check HTTP layers can charge an
earlier quota before a later denial. This adapter reuses native atomic checking,
not native single-policy HTTP layering. Shadow denial permits work. For an enforced native denial, `Retry-After` is present exactly when the
native `Denial` supplies a `Delay`. Its `seconds()` value is the native
whole-second result: zero stays `0`, and positive subsecond remainder rounds
up (for example, 1 ns to `1`, 2001 ms to `3`). Denials without a retry delay,
authentication failures and backend failures omit the header. Concrete
auth/backend errors stay internal as response `Arc<E>` extensions. Fixed
boundary rejections use exactly one JSON
`{"code": ...}` body and `Cache-Control: no-store`:

| Condition | Status | `code` |
| --- | --- | --- |
| Authentication failed | 401 | `authentication_required` |
| Native quota exhausted | 429 | `quota_exhausted` |
| Native storage capacity denied | 503 | `quota_storage_capacity` |
| Native backend failed | 503 | `quota_backend_failed` |
| Missing protected composition context, direct peer, or observer | 500 | `missing_context`, `missing_peer`, or `missing_observer` respectively |

The three 500 codes are defensive faults outside the supported opaque
`PreparedHttp` serving path. The pinned native `Denial` has exhaustive
quota-exhausted and storage-capacity reasons. A new reason requires an explicit
adapter mapping before a later Runlimit revision compiles. The generic
observation writer retains `other_denial` for manual assertions outside this
native adapter; this adapter does not emit it at the current pin.
The PostgreSQL bridge delegates certainty to native `BatchCheckError::consumption`:
commit-outcome-loss and commit-timeout are `PossiblyConsumed`; pre-commit failures
are `NotConsumed`. The native exhaustive errors no longer contain a
post-commit malformed-response variant. Native decisions are built before commit.
Nested quota check or work interruption is a lifecycle failure, not a fixed
quota rejection. `request_admission` captures the original request metadata without the
private quota writer and installs an opaque interruption responder for the inner
adapter; it uses
the same `RequestPolicy` renderer as an outer cancellation or deadline. A
custom renderer, including `with_infrastructure_json`, owns that response's
status, safe body and headers. A completed native quota fact remains retained
when later work is interrupted.

`PreparedHttp::register_in` consumes the assembled service into native direct-peer
serving under process ownership. Its `in_process` alternative requires a synthetic
peer per request and cannot be served or converted into a Router. Probe routes
are explicitly unguarded. All routes, probes and fallback are covered by one root
observer with fresh server correlation. Opt-in `operational_http_with_quota` retains
bounded quota facts across outer timeout or drop and publishes them into the
one outer observer in either supported operational-wrapper order. The adapter claims the sole
writer before protected handlers and discards it before public probe handlers;
ordinary operational HTTP allocates no
quota record. Its consuming start/finish API retains `NotChecked` before the
check, `Unresolved` after a dropped in-flight check, and a terminal native fact
after completion; a later work timeout has no writer to downgrade it. The
terminal value remains an adapter assertion, not a proof of native truth. Raw
subjects, credentials and error strings are not observations.
Body streaming and detached descendants remain outside response construction.

The independent `attempts::AttemptRunner` owns native reservation, bounded
verification, transaction-scoped receipt claim, final application decision,
native completion, and acknowledged commit. Credential verification occurs
outside the application transaction; final replay and account-status checks
occur inside it. `Authentication::Rejected` commits failure state and audit,
while infrastructure errors roll back. A stale claim prevents the application
callback. Construction requires `PgProfiledPool` with one authoritative schema;
native admission and completion share its immutable declared profile. The SQLx
foundation owns all pool normalization hooks, including release before fast-path
reuse. No arbitrary hook-bearing pool can construct the attempt owner.
`run_atomic_profiled_in` retains acknowledged and uncertain results before
operation resolution. A valid claim holds its native row lock through commit;
verification-lease expiry after claim does not revoke that transaction.

See the [compiled quota consumer](../crates/batter/examples/quota_service.rs),
[attempt consumer rustdoc](../crates/batter-runlimit/src/attempts.rs), and
[failure contracts](../crates/batter-runlimit/tests). Fourteen explicit PostgreSQL 18
attempt tests passed locally, including restricted-role/custom-schema admission
and completion, profile drift, and missing-authoritative-table rejection.
Fresh-agent usability evaluation remains unexecuted.

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
every consumer. Keep application-specific composition local until shared
operational invariants justify an adapter. The user-approved Runlimit prototype
established a direct library path without requiring a reference-service task
first; it does not establish application adoption or stable API maturity. Integration fixtures may
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
The runnable HTTP example uses its literal source reader and bounds, retains
`BulkheadCapacity` and `ResponseConstructionBudget`, and passes those witnesses
into the actual router. The reference package owns concrete
`config::{ServingSettings, MaintenanceSettings, PreparedServing, PreparedHttp,
PreparedMaintenance, PoolSettings, WorkerSettings}`. Serving construction retains
Batter `BulkheadCapacity`/`ProcessCapacity`, Axum `ResponseConstructionBudget`, a
password-qualified endpoint, a concrete authenticator, and a validated native
`JobsConfig`, a validated provider origin and a redacted provider credential.
HTTPS is required except for loopback HTTP fixtures; redirects are disabled.
Maintenance recognizes only its database schema and has no
promotion or conversion into the serving types.

Use `ServingSettings::from_process(selected_path, overrides)` once, then transfer
the result through `runtime::prepare` before acquisition. The returned must-use,
non-cloneable `PreparedServing` owns inert `PgPoolOptions`, `PgConnectOptions`,
`Supervisor`, `JobsConfig`, provider client/bulkhead, bind address and `PreparedHttp`; `runtime::run`
accepts only that owner. Router construction consumes only `PreparedHttp` and is
infallible because authentication and local operational values are already
concrete. Offline commands separately consume `MaintenanceSettings::prepare`.
Tests can inject file/environment/override sources directly.
Precedence is defaults < explicit file < captured environment < explicit
in-memory overrides. Dedicated files/overrides reject every unknown key;
environment ignores unrelated names but rejects unknown BATTER_/JOBS_ names and
all PG* entries. Maintenance additionally ignores known serving-only names from
captured environment without parsing them; its dedicated file and overrides
remain database-only. Native preparation also rejects actual PG* entries even when a
settings loader was injected. Keep process environment unchanged during native
construction; no passfile, native URL fallback or `JobsConfig::from_env` path is
used. The application owns command schemas and password/TLS policy; Batter owns
the reusable operational witnesses rather than a generic configuration mode or
typestate framework. See the
[reference settings schema](../examples/reference-service/README.md).

The reference fixture paths and delivery command root consume these outputs.
The command root uses configured authentication, request deadline, pool,
Bulkhead and finite-process constructors; live held-work cases prove their
runtime effects. Its worker transfers native preparation to `batter-runledger`,
preserving native intent-promoter inheritance. Each enabled loop acknowledges
local initialization; fresh health and application approval remain separate.
The production registry installs `records.delivery.execute`; ordinary readiness
approval follows that registration, independently of native acknowledgement and
fresh health.
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
tokio-postgres parser receives the same spaces as maintenance preparation and SQLx. It also
serializes the validated hostname and database, canonicalizes the disabled TLS
mode and retains an explicit empty password instead of permitting SQLx passfile
fallback. IPv6 literals are rejected at this private live boundary: the locked
SQLx URL parser retains brackets during TCP lookup. Use `localhost` or
`127.0.0.1`; direct application construction still supports native IPv6 options. Userinfo
`+` and existing percent escapes retain their meaning. The Python runner
owns budgets and exact case inventory, not endpoint parsing or credential
fallback. Ordinary native-constructor tests cross a cleared-environment child
boundary, and the matrix requires success with hostile PG* parent variables.
The acquisition, cleanup-failure, pool-failure and primary/secondary observation
fixture roots all consume the shared handoff; their custom budgets do not re-read
the original URL.

### PostgreSQL verifier identity and evaluation limits

The SQLx verifier preserves discovered identities separately from their effective
permission sources; cross-schema multiranges remain covered. Qualified finding
names use individually quoted components and canonical routine signatures.
Captured evaluation cooperates with the existing OperationContext and fails
with EvaluationCapacity instead of returning partial results when work or report
limits are exceeded. The canonical pool/context entrypoints and client retirement
remain unchanged. See the adapter README for exact policy, row, work and payload
limits and the expected native idle-rollback warning.

Verifier scope is an explicit PostgreSQL18 capability contract. Selected temporary
namespaces and dependencies yield Incomplete/TemporaryNamespaces through every
entrypoint. Consumers should declare supported persistent objects and handle
unsupported results; they should not recreate missing namespace rules. PUBLIC
relation declarations replace column defaults unless a column is explicitly
declared, matching required-policy validation. The adapter preserves snapshot
evaluation because native privilege inquiry caches may observe newer grants.

Migration verification supports standalone ordinary ledgers. Inheritance in the
captured snapshot returns Incomplete/InheritedMigrationLedgers; ONLY reads prevent
late attachment from changing the snapshot relation set. Consumers retain native
migration policy and must not treat an unsupported ledger as verified.

When a manifest declares column authority, also declare its parent relation even
when no relation-level privilege is allowed. This makes the relation's ownership,
row-type and PUBLIC settings explicit and is enforced by compilation. Grant-plan
rendering rejects PostgreSQL's PUBLIC/NONE spellings and reserved `pg_` role
namespace, but applications still own membership and selection for ordinary role
targets.


## Owned native database composition

Use `PgLease::migrate` for application-selected SQLx migrations. For Runledger,
construct `RunledgerDatabase` with an explicit `PgSessionProfile`, then use
`verify_schema(&database)` and `run_atomic(&database, async |scope| ...)`.
The database owns mandatory acquisition hooks for role/schema/timeout/tenant
policy; workers and ordinary operations use `database.pool()`. Atomic and snapshot
reset re-establish the same policy before exposing SQL. Required intent recording
rejects known conflicts inside the transaction, before application state commits.
Intent recording precedes the consuming `scope.queue()` transition; enqueue is
available only in the resulting queue phase. No
native resource view or compatibility module remains. Dependency direction is
`batter-runledger -> runledger-postgres -> batter-sqlx -> batter-core`.
The facade optionally selects the integration; SQLx alone never selects Runledger.

`NativeReport` now owns Runledger's consuming settlement classification. Borrow
`native()` for diagnostics or `settlement()` for the typed outcome. The managed
adapter derives success and cleanup eligibility from those unforgeable variants,
not caller-supplied flags. Explicit `CommitUnconfirmed` remains an unknown native
effect; reference retirement preserves it and never automatically replays it.
