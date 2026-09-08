# Integration ownership and planned adapters

Only the Axum adapter and the native SQLx **example** exist in this snapshot.
Runlimit, Runledger, and postgres-test-harness are not dependencies of the
library/test-support crate. The designs below are handoff requirements, not
unimplemented APIs advertised as available.

## Axum: implemented, with a deliberately small boundary

Use RequestPolicy with middleware::from_fn_with_state. Put the guarded business
router behind it and merge unguarded liveness/readiness routes separately. Domain
services receive their own dependencies through State/FromRef/constructors;
request operation context arrives through Extension<OperationContext>.

`with_failure_renderer` maps middleware failures into an application-owned
envelope using a snapshot of request parts. Trusted correlation middleware must
run outside it; never assume raw inbound headers are trusted. Application
handlers must use their own renderer too. The HTTP example demonstrates both
paths with matching generated request-ID headers/body fields. Readiness requires
all registered components to acknowledge startup before traffic is admitted.

Keep authentication, authorization, request body limits, CORS, TLS, proxy trust,
trace-header validation, tenant resolution, and user admission policy external.
Choose middleware order deliberately: Batter's timer starts inside its middleware,
not before an outer queue. The example is GET-only and not an upload/streaming
security template. See [guarantees](guarantees.md).

The supervised Axum server uses with_graceful_shutdown and waits for it to finish
while dependencies remain alive. Aborting that wrapper is not accepted as proof
of transitive child termination; resource finalizers are conservatively skipped.
A real-transport verification task must exercise idle/active keep-alive requests,
slow bodies, streaming responses, shutdown, and client disconnects.

## SQLx: keep transactions visible

The [example](../examples/postgres_lifecycle.rs) uses native PgPoolOptions,
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

The reference application should transactionally create a business record and
submit a job using the current native enqueue/intent API. Tests must prove that
rolling back removes both, reusing a key with the same request deduplicates,
and changed-payload conflicts are explicit. External effects remain at least
once unless the external protocol independently protects them.

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

The integration example should show explicit layer ordering: trusted metadata,
authentication as needed, Runlimit admission, Batter operation policy, and domain
execution. The correct order depends on which identity is being limited and must
not become a universal security claim.

## postgres-test-harness: future application test adapter

Reviewed baseline: postgres-test-harness 0.2.0 provisions/connects to PostgreSQL
18, caches fingerprinted templates, and clones isolated test databases. Its
public API is independent of SQLx. Re-check the actual release before coding.

The future adapter belongs in test support behind an optional feature, with no
production container dependency. Cache a harness and stable templates once per
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
Avoid a feature that forces all three integrations onto every consumer.

Before promoting an adapter into a stable public API, use it in two different
application composition roots, exercise partial-startup and shutdown failures,
and document the external version contract. An ergonomic convenience is not a
reason to hide native SQLx transactions or collapse meaningful upstream errors.
