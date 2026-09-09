# Integration ownership contracts

Only the `batter-axum` adapter package and the native SQLx **example package**
exist in this snapshot.
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

The [example](../examples/postgres-lifecycle/src/main.rs), packaged as
`batter-example-postgres-lifecycle`, uses native PgPoolOptions,
query_scalar, and Pool::close. It registers close as a dependency finalizer and
shows startup-error cleanup. No database abstraction or generic transaction retry
is introduced. The example compiles with SQLx 0.9.0; it has not been run against
PostgreSQL here. This dependency requires Rust 1.94 or newer.

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
README. Re-check the exact release/API before implementation; Batter has not
compiled against any Runledger version.

Runledger retains internal supervision, heartbeat/lease logic, claim behavior,
schedules, workflows, retries, durable intents, schema compatibility, and
operator interfaces. Batter should host the upstream supervisor as one critical
component and preserve its shutdown result. Do not spawn each of its internal
loops under a second independent policy.

The host must align budgets: stop new claims during drain, allow the upstream
runtime enough time to drain, and observe its returned error. The outer grace
must not casually drop run_until_shutdown before its own cleanup completes.
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

## postgres-test-harness: future application test adapter

Reviewed baseline: postgres-test-harness 0.2.0 provisions/connects to PostgreSQL
18, caches fingerprinted templates, and clones isolated test databases. Its
public API is independent of SQLx. Re-check the actual release before coding.

The future fixture belongs beside the application/example integration tests,
with no production container dependency. Generic `batter-test-support` remains
independent of the harness, foundation, and adapters. The harness itself remains
external; this workspace split imports neither its source nor its dependency.
Cache a harness and stable templates once per
test process. Fingerprint every ordered application and dependency migration
bundle plus a revision for setup behavior not represented by SQL bytes.

Use native SQLx pools and application migration entrypoints. Close all application
connections/pools before returning a database lease. Preserve body, pool-close,
lease-cleanup, and deferred-drain errors; a first question-mark must not skip the
remaining cleanup. Keep a separate empty-database path for migration-order tests.

Do not issue Docker CLI cleanup, clone databases manually, delete by name prefix,
reset dirty test databases for reuse, or advance PostgreSQL clocks by calling
Tokio's time::advance. Do not silently pass tests when the database is unavailable.
Use explicit feature/test targets and report prerequisites or failures.

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
