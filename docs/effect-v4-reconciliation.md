# Effect v4 analysis reconciled with Batter

Updated: 2026-09-11. Originally reviewed 2026-09-09 against Git baseline `22848ea`;
the implemented-evidence column now includes later settings, fixture, ownership
and HTTP-lifetime deliveries. This is a design reconciliation, not a new
implementation or a claim of production validation. Execution evidence lives in
[validation](validation.md).

Batter implements much of the operational core proposed in the analysis. The
complete application path and some boundary conventions remain unimplemented.
Delivery outcomes and dependencies are tracked in [Beads](roadmap.md). The central thesis holds: ordinary Rust types,
futures, constructors, and Tokio should remain the programming model. Uniformity
is useful where it makes ownership, errors, and boundaries predictable.
The [backend evidence assessment](design-evidence.md) sharpens that rationale
using historical review failures and the newly committed implementation.

The eight-item proposal is not an implementation checklist to copy literally.
Some items already exist, some need narrower contracts, and some belong in
application composition roots or upstream libraries.

## What exists against the eight priorities

| Proposed area | Implemented evidence | Current boundary |
| --- | --- | --- |
| 1. Error model | [OperationError<E>](../crates/batter/src/operation.rs) and [RetryError<E>](../crates/batter/src/retry.rs) preserve concrete failures and distinguish interruption; [HttpFailure](../crates/batter-axum/src/lib.rs) has stable sanitized codes, default Problem JSON, and configurable rendering. | No universal Error, app-wide domain-code contract, or HTTP panic catcher. Domain errors remain concrete; wire policy belongs to the application. |
| 2. Composition root and lifecycle | [Supervisor](../crates/batter/src/lifecycle.rs), [owned driver](../crates/batter/src/lifecycle/driver.rs), [finite work](../crates/batter/src/lifecycle/process.rs), and [CleanupStack](../crates/batter/src/cleanup.rs) implement acknowledged readiness, admission, drain/cancel/abort observation, retained reports, and LIFO cleanup. Private state owns terminal transitions and abandonment. [SQLx example](../examples/postgres-lifecycle/src/main.rs) shows explicit acquisition and partial-startup cleanup. | Resource construction remains an application pattern. No DI graph, memoizing builder, automatic async resource scope, or general request-child joining. Non-yielding, scheduling, component-ownership and HTTP/1.1 lifetime suites have scoped execution evidence; they do not detect hidden children or bound streaming bodies after response construction. |
| 3. Ambient request context | HTTP supplies explicit `Extension<OperationContext>` with deadline/cancellation. Opt-in `operational_http` supplies opaque server-generated `CorrelationId` and retained HTTP event fields; the [HTTP example](../crates/batter-axum/examples/http_service.rs) adopts it. | No task-local request context, tenant/principal model, inbound trace-parent handling, or durable envelope. Ambient access is optional ergonomics, not an authorization mechanism. |
| 4. Retry policy | [RetryPolicy](../crates/batter/src/retry.rs) implements fresh factories, explicit replay authorization/classification, bounded attempts, provider delay floors, total budgets, and injected jitter. | No `backon` dependency or outbound HTTP/SQLx/job adapter. There are no per-attempt budgets or retry tokens. Share mechanics while retaining one retry owner for each operation. |
| 5. Config and secrets | [`batter::settings`](../crates/batter/src/settings.rs) supplies explicit bounded sources, parsing and redacted diagnostics; HTTP and reference roots own their schemas and native constructors. [Argument validation](../crates/batter/src/validation.rs) still rejects invalid budgets/registration. | Not a configuration framework or secret-erasure tool. Live forty-case acceptance of the reference constructors remains pending. Argument validation is not a config API. |
| 6. Observability | [Telemetry](../crates/batter/src/telemetry.rs) records outcomes/timing; independent HTTP observation covers assembled routes, probes/fallback and rejection, with explicit response severity and retained correlation under filtering. [Scoped dispatch](../crates/batter/src/scoped_dispatch.rs) retains owned-future tracing through polling and destruction. | No metrics/exporter setup recipe or durable trace propagation is implemented. No global subscriber installation belongs in the library; arbitrary synchronous subscriber failures are not isolated. |
| 7. Schema and contract | HTTP infrastructure rendering can match an application's wire envelope. Optional Serde currently serializes that envelope. | No validated JSON extractor, schema generation, OpenAPI, or client round-trip pipeline. |
| 8. Test kit | [Test support](../crates/batter-test-support/src/lib.rs) provides scripted results and preserves body plus cleanup errors. Tests use native Tokio paused time. Optional [`batter-sqlx/test-support`](../crates/batter-sqlx/src/test_support.rs) owns isolated fixture leases/templates; reference live probes stay ignored until endpoints are configured. | No TestApp. PostgreSQL provisioning stays in the external harness. Combined forty-case live acceptance has not run on this checkout. |

The crate also already has [process-local concurrency admission](../crates/batter/src/admission.rs),
explicit finalization reserves, bounded finite task receipts, and shutdown waiters
that do not own cleanup. These strengthen the proposed lifecycle model without
introducing a runtime. Semaphore waiters themselves are not bounded; finite
process admission separately rejects at capacity without creating a waiter queue.

## Decisions to retain or narrow

**One error convention need not mean one error type.** Preserve the existing
`E` through operations/retries. A stable client code and explicitly sanitized
representation can be an application boundary convention without promoting an
upstream QueryError into a universal Batter error. `Display`, `Debug`, and
`Error::source()` on arbitrary errors are not safe HTTP or telemetry output.
Process/cleanup reports already preserve multiple heterogeneous failures; a new
Cause algebra needs a demonstrated fan-out use case beyond those reports.
See [ADR-005](adr/005-errors-and-telemetry.md).

The default HTTP body already contains `type: "about:blank"`, `title`, `status`,
and `code`, with `application/problem+json`. It is a basic Problem Details shape,
not an app-wide problem-type catalogue. RFC 9457 uses `type` as the primary problem
identifier; `code` is Batter's extension, and `detail`/`instance` are not mandatory.
Application-owned types/codes define the domain wire contract. Existing response
tests do not establish comprehensive RFC conformance.
See the [RFC reference](references.md#boundary-conventions-reviewed-2026-09-08).

**Panic observation and HTTP panic recovery are different.** Owned task and
cleanup panics are reported. Operation panics propagate, and the HTTP middleware
does not catch handler panics or turn them into `HttpFailure::Internal`.
The [handler-unwind regression](../crates/batter-axum/tests/observation/correlation.rs)
proves propagation, admitted-context cancellation and a dropped observation with
no fabricated HTTP status or copied panic payload.
An opt-in Tower panic boundary would require an explicit application renderer.
Such a boundary cannot catch aborting panics, sanitize the default panic hook,
or convert a body panic into a fresh 500 after headers were sent. The application
owns panic-hook policy; recovery does not establish that damaged shared state can
continue serving. No HTTP panic recovery guarantee is implemented.

**Use the existing supervisor.** JoinSet supplies direct completion and panic
observation; substituting TaskTracker alone would lose the failure-supervision
contract. Resource identity comes from constructing and sharing values once at
the root. It does not require Layer memoization machinery. Cleanup still requires
explicit awaiting; aborting/joining a wrapper proves nothing about detached
descendants. The current conservative finalizer skipping remains necessary.

**Separate correlation, authority, and execution lifetime.** Keep application
services explicit and trusted identity resolution outside Batter. First establish
typed request metadata via extractors/extensions and explicit propagation. A
future task-local convenience must have defined absence and child-task behavior;
it cannot make arbitrary `tokio::spawn` inherit context. Batter-owned tracing
already crosses its task boundaries, including destruction under scoped subscribers.

Durable jobs may carry validated, versioned correlation and application-selected
identity data, but correlation alone grants no authority. Worker authorization
must be explicit. Never persist a CancellationToken or Tokio Instant, or make
committed durable work a child of the request's cancellation token. Each durable
job attempt needs a new execution budget; this composition is not yet verified here.

**Unify retry rules without multiplying retry owners.** The current policy is
already independent of a dependency's error type. HTTP replay, transaction replay,
and durable rescheduling have different certainty and ownership rules. No direct
mapping to an upstream handler timing API is verified yet. Runledger retains
durable retries; a host must not add another job retry loop. Native SQLx transaction
boundaries stay visible, and an unknown commit outcome is not a replay permission.
Replacing the implementation with `backon` is unnecessary unless it preserves all
existing contracts and solves a concrete maintenance problem.

**Setup belongs at the root.** An observability recipe may configure a subscriber,
exporters, and ordered flush, while reusable Batter code emits observations.
Root config may compose upstream typed options; it should not require upstream
crates to depend on Batter or replace their public configuration conventions.
Schema/extractor libraries named in the analysis are candidates, not selected
dependencies. Choose and version-check them during the reference implementation.

## Durable execution, AI, packaging, and agent guidance

The DAG-versus-imperative workflow choice belongs to Runledger and its consumers.
Batter hosts the chosen upstream model; it must not fill an integration gap with
a journal, deferred-signal engine, queue, or workflow DSL. The supplied comparison
of upstream workflow features is not a compatibility check against a pinned
Runledger release. Verify it before relying on a particular enqueue/workflow API.

An LlmClient abstraction is outside current scope until a real application needs
provider interchange. AI provider calls would still need explicit replay,
streaming ownership, admission, and durable execution policies. No AI integration
is implemented here. Streams, STM, structural equality, and an Effect datatype
remain outside scope. Mutexes/atomics/channels are not equivalent to STM;
`Stream` alone also does not bound an entire pipeline's
buffers or spawned producers.

Cargo features control optional compilation; they do not themselves establish an
unstable API compatibility policy. Do not add a catch-all `foundation-unstable`
flag merely to reproduce Effect's packaging. HTTP and SQLx adoption now
use separate packages; neither is a foundation feature. See
[ADR-006](adr/006-workspace-packages.md) and [Cargo's feature policy reference](references.md#boundary-conventions-reviewed-2026-09-08).

[AGENTS.md](../AGENTS.md), rustdoc, six runnable demonstrations plus a read-only
live-suite preflight, and the
[failure-contract test map](testing.md) already provide the agent-facing starting
point. No `llms.txt` exists. An additional index would be navigation only, not a competing contract. Predictability should come from checked types,
examples, and failure tests. The reference package is a pinned compatibility
probe, not a complete durable service.

## Evidence behind this comparison

Representative existing tests include `original_application_error_survives` and
`panic_is_not_converted_to_expected_failure` in [operation tests](../crates/batter/tests/operation.rs);
`attempt_timeout_is_not_automatically_retried` and provider-floor tests in
[retry tests](../crates/batter/tests/retry.rs); error retention and conservative cleanup in
[lifecycle tests](../crates/batter/tests/lifecycle.rs); readiness acknowledgements, receipt
ownership, and waiter cancellation in [process tests](../crates/batter/tests/process_ownership.rs).
[HTTP tests](../crates/batter-axum/tests/http.rs) cover deadlines and trusted renderer metadata;
[core telemetry tests](../crates/batter/tests/telemetry.rs),
[HTTP telemetry tests](../crates/batter-axum/tests/telemetry.rs),
[core dispatch tests](../crates/batter/tests/scoped_dispatch.rs), and
[HTTP dispatch tests](../crates/batter-axum/tests/scoped_dispatch.rs)
cover context and output exclusions. These existing tests support the implemented
rows; the remaining rows are proposals without implementation evidence.

Effect's dated release and migration claims are checked separately in
[primary references](references.md#effect-v4). The RC timeline is supported by
the upstream announcement and rc.112 release. Two corrections to the analysis:
Schema and JsonSchema are core modules, and no primary benchmark was verified
for the blanket streams/batching 20-times multiplier. The flattened Cause also
does not preserve sequential-versus-parallel composition. None of these upstream
claims establishes Batter performance or equivalent runtime semantics.
