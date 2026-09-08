# Preserved design brief: Effect v4 -> a Rust operational foundation

The [2026-09-08 reconciliation](effect-v4-reconciliation.md) compares the supplied
eight-priority analysis with current source and tests. It records which proposals
are implemented and which capabilities or guarantees are absent. Delivery
work lives only in [Beads](roadmap.md).

## The thesis

Borrow Effect's coordinated runtime semantics, not its TypeScript remediation
or a second programming language inside Rust. Axum is an adapter. The user's
Runlimit, Runledger, and postgres-test-harness crates retain their existing
responsibilities. Batter fills the seams between those libraries and an
application's composition root, not the libraries' internals.

The official v4 API pages inspected for this snapshot identify 4.0.0-rc.112.
V4 core and the effect/unstable infrastructure surface have different stability
commitments. Treat v4-specific material as the reference; do not copy v3 service
or cause APIs from older tutorials. See [primary references](references.md).
Batter contains no Effect source code, dependency, interoperability promise,
or benchmark claim.

## What Rust already supplies

Result, enums, pattern matching, and `?` already give explicit expected failures.
Ordinary constructors/arguments expose dependencies. Futures are lazy. Ownership,
Send, and Sync provide native memory/thread-safety machinery. None of these
need to be reimplemented as an Effect<Success, Error, Requirements> datatype.

Effect's requirements parameter and service/layer composition can infer and
compose dependencies more conveniently, but explicit Rust construction is a
sensible starting point. Effect computation descriptions and individual Rust
futures are not interchangeable: a retry needs a factory for a fresh future,
not an attempt to restart a completed future.

Memory-safe work can still be operationally wrong: detached tasks can outlive
requests, task failures can be ignored, cleanup may not be awaited, and retry
boundaries can repeat committed side effects. Those are Batter's concerns.

## What to borrow, by area

| Effect-inspired concern | Batter decision | MVP status |
| --- | --- | --- |
| Explicit asynchronous lifetime | Distinguish operation, process, and durable work. Own critical and bounded finite tasks; separate shutdown ownership from waiting. | Implemented partially; no request child-task joining or general fiber scope. |
| Scope/resource finalization | Explicit owned asynchronous finalizers and ordered teardown. | Implemented, but not acquireRelease interruption masking or automatic async RAII. |
| Layer composition | One visible composition root, shared concrete services, partial-startup cleanup. | Pattern and example; generic graph/DI intentionally omitted. |
| Expected failure vs defect vs interruption | Preserve E, observe JoinError, distinguish cancellation/deadline. | Implemented; no Effect Cause clone or universal domain error. |
| Schedule/ExecutionPlan | One execution budget, fresh attempts, replay policy, provider delay, injected jitter, finalization reserve. | Retry/reserves implemented; fallback, per-attempt budgets, and circuits deferred. |
| Coordinated observability | Operation/task/cleanup completion and separate HTTP status/outcome/latency. | Useful default tracing implemented; metrics, exporters, and durable trace propagation deferred. |
| Schema/HttpApi coherence | Validated boundary types, explicit errors, generated contracts where useful. | Configurable infrastructure HTTP envelopes; OpenAPI/client generation deferred. |
| Deterministic test services | Paused runtime time, scripted dependencies, real PostgreSQL integration separately. | Timer/script tests authored; PostgreSQL harness composition deferred. |
| Durable work infrastructure | Use existing Runledger; never turn durable work into detached tasks. | Ownership contract documented; host/reference integration deferred. |

Effect Layer is dependency/resource construction; Tower Layer decorates services.
They are not the same abstraction. Using Tower where a Service-shaped boundary
fits does not imply every domain function should become a Tower Service.

Effect's managed runtime coordinates finalization and interruption. This MVP
cannot honestly claim equivalent guarantees simply because it has CleanupStack.
Its finalizers must be explicitly driven. The owned driver survives cancellation
of a completion waiter, but directly dropping `run_until`/`CleanupStack::close`
or terminating the runtime can still prevent awaited cleanup. Finalization
reserves separate work from cleanup time without masking parent cancellation.
The smaller contract is deliberate and must
remain visible until a stronger protocol has been designed and tested.

## Existing libraries and their ownership

Runlimit owns validated quota policies, storage semantics, bounded backends,
admission decisions, and its own Axum adapter. Applications own trusted identity
normalization and response policy. Batter should connect—not replace—those
boundaries and preserve uncertainty about consumption around database commit.

Runledger owns durable queues/workflows, leases, retries, scheduling, recovery,
transactional submission, and internal worker supervision. A future Batter host
should coordinate its lifecycle and correlation. Do not build another outbox
because the term appears in an architecture checklist. Use the upstream
transactional enqueue or durable-intent API appropriate to the application.

postgres-test-harness owns provisioning, template fingerprints, isolated clones,
connection admission, and cleanup machinery. Application/example fixtures should
compose initialization and teardown around it, not copy Docker/database management
code into the foundation or generic test utilities. The harness stays external;
native SQLx queries and migrations remain available.

## Failure semantics that must survive future work

A timeout only says the caller stopped waiting. A commit or provider mutation
can have happened even when its reply was lost. An `error.is_retryable()` boolean
cannot independently establish replay safety. Repeated nested three-attempt
policies can multiply underlying requests; there must be a deliberate retry
owner. Idempotent enqueue does not establish exactly-once external execution.

Cancellation tokens, request deadlines, and authorization data have different
meanings. Do not persist request cancellation/deadlines into durable jobs. Use
safe correlation metadata and a new job execution policy. Quota checks,
concurrency permits, and retry budgets must also remain distinct.

A global AppContext, opaque universal error, automatic transaction retry wrapper,
custom scheduler, and every possible abstraction are not prerequisites for
reliability. They increase the amount of private framework behavior an agent
must guess. A small API surface, compile-checked examples, and failure-path
contracts are the useful response to the "AI era" framing.

## Delivery ownership

[Beads](roadmap.md) owns the audited delivery outcomes and dependency graph.
This brief preserves design rationale and ownership boundaries, not a parallel
implementation sequence. Reusable APIs still require evidence of repeated
consumer needs and shared failure semantics.
