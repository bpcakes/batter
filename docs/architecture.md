# Architecture

## A foundation of operational contracts

The workspace targets Unix backends. Windows is unsupported and not planned;
process and signal boundaries use native Unix semantics without non-Unix
fallbacks. [ADR-007](adr/007-unix-platform-scope.md) records this platform policy.

Batter's unit of reuse is an invariant: who owns this work, which deadline bounds
it, who observes its failure, and when may its dependencies close? It is not a
collection of wrappers around every dependency.

The root is a virtual Cargo workspace. The `batter` foundation,
`batter-axum` adapter, and `batter-test-support` utilities are separate libraries;
`batter-example-postgres-lifecycle` is an unpublished executable package.
Public functions accept native futures, concrete errors, and runtime
primitives. Only process/cleanup boundaries erase errors into BoxError, because
those boundaries aggregate heterogeneous component results.

```text
application composition root
  |-- native services / concrete constructors / domain errors
  |-- batter lifecycle + cleanup
  |-- batter operation + retry + admission
  |-- native tracing subscriber and exporters (application-owned)
  |-- optional batter-axum -> batter + Axum / Tower
  |-- native SQLx PgPool / Transaction
  |-- Runlimit, Runledger (future thin adapters)
  `-- tests -> batter-test-support + postgres-test-harness (future composition)
```

The foundation graph has Tokio, tokio-util, tracing, thiserror, and pin-project-lite.
The latter provides safe pin projection for a private, allocation-free tracing
context wrapper; it was already a transitive dependency. Axum/Serde belong to
the adapter package. SQLx belongs to the example package. There is no TypeScript runtime,
algebraic-effect datatype, service locator, runtime-neutral abstraction, or
cyclic dependency on the user's reusable libraries.

`batter-test-support` depends on neither the foundation nor an adapter. Core
tests can use its generic scripts without pulling higher layers back into the
foundation. Cross-package fixtures belong in their application/example test
targets. The external PostgreSQL harness is not moved or made a dependency by
this reorganization. New SQLx, Runlimit, or Runledger adapter crates require
proven shared mechanics; the current SQLx composition remains native application
code. See [ADR-006](adr/006-workspace-packages.md).

Each package declares its version, Rust minimum, and publication policy. All
currently retain version 0.1.0, Rust 1.94, and `publish = false`; a shared
workspace does not imply a mandatory stack or synchronized releases. The root
lockfile and verification matrix remain shared.

## Three work lifetimes

**Operation lifetime:** ordinary composed futures, bounded by an OperationContext.
The context contains deadline and cancellation—not application dependencies or
an authentication principal. `run` creates a child scope, drives one future,
and notifies descendants when the scope ends. It does not supervise arbitrary
spawned descendants. Its static operation name is a telemetry convention.

**Process lifetime:** long-lived critical components registered with Supervisor.
Their factories are inert until run_until is polled. Factory invocation happens
inside the owned task, so factory panics are visible through JoinError. `start`
drives this coordinator independently of result waiters. Finite
startup checks are ordinary awaited calls before readiness, not critical loops.
All critical exits before drain are failures, even Ok. There is no automatic
restart and no ignored task-result path.

Optional bounded finite admission shares process ownership. Root submission
stops at drain; active scopes may admit descendants until forced cancellation.
Receipts carry results without owning cancellation or capacity. Successful
completions increment a counter; task-level failures close admission and initiate
drain, retaining all failures in the bounded outstanding set. Expected domain
rejections are task values. This is not persistent scheduling or general fiber scope.

**Durable lifetime:** work persisted in Runledger. Durable work must not inherit
an expiring HTTP cancellation token or serialize a Tokio Instant. Persist safe
correlation metadata and give each job its own policy. Runledger owns leasing,
retries, scheduling, recovery, and its own internal supervisor.

## HTTP observation ownership

The adapter separates response observation from admission/deadline policy, since
probes and rejected or unmatched requests still need observations. The observer
owns response facts, the first-poll dispatcher and a retained context span. Its
INFO span is optional diagnostics: filtering it must not erase event fields or
an available application parent. At first poll, `Span::or_current` selects the
HTTP span or current application span for execution and event parenting. A later
poll or destructor cannot substitute another request's ambient identity. HTTP
field recording targets only the original HTTP span, leaving application fields
untouched. Sink-specific filtering and formatting remain application-owned.

This uses native tracing handles and the foundation's existing dispatcher wrapper;
it introduces no identity service or second async ownership mechanism. Handler
unwinds follow the same dropped-response observation path and propagate normally.

## Composition and readiness

Acquire dependencies in an explicit composition root. Register each resource's
cleanup immediately after successful acquisition. Register dependencies before
dependents so teardown reverses the order. Run application-specific migration
compatibility and readiness checks before calling handle.mark_ready().

Each registered component acknowledges actual initialization with
`ShutdownSignal::mark_started`. `mark_ready` only arms publication: the driver
must be running and every component must acknowledge before Ready is visible.
Batter cannot prove an acknowledgement is truthful or inspect hidden children.
The signal example acknowledges only after installing its listeners.

If startup fails, extract the cleanup stack and await it. Preserve the startup
error and the cleanup report separately. Cancellation/panic during unprotected
startup can still skip explicit asynchronous cleanup. This is not an Effect
Layer graph or acquireRelease masking protocol.

## Shutdown state machine

`lifecycle/state.rs` owns readiness and admission facts. Every transition uses
the same mutex as finite-work admission, including the final Stopped transition.
Only a private method taking that mutex guard can publish the atomic readiness
snapshot. This keeps reads independent of an in-progress enqueue while making
Stopped irreversible. Other modules receive operations and a restricted enqueue
guard, never mutable fields. Startup history is named `driver_started`; it does
not claim that a completed driver is still running.

```text
Starting --driver + mark_ready + all mark_started--> Ready
    \                     |
     `------request-------' --> Draining
                                  |
                      stop admission / new claims
                                  |
                       await drain allowance
                                  |
                      notify forced cancellation
                                  |
                     await cancellation allowance
                                  |
                    request abort; bounded join wait
                                  |
             clean task termination? ---- no --> report skipped finalizers
                        | yes
                 explicit LIFO teardown
                        |
                     Stopped + full report
```

Stopped means the coordinator finished, not that every future in the process was
proved dead. Read ShutdownReport. A successful report requires no task failure,
no requested abort, no unjoined task, and complete successful finalization.
Cooperative cancellation can still produce a successful report when tasks
return cleanly. External process exit policy belongs to the binary.

The owned driver separates work ownership from waiting. Last-owner drop requests
drain; cancelling an observer does not cancel cleanup. Its monitor observes the
coordinator and publishes a retained report or JoinError. The lower-level
`run_until` and `CleanupStack::close` remain cancellation-fragile when driven
directly by callers. No guarantee survives termination of their Tokio runtime.

The supervisor holds a synchronous abandonment guard from construction, before
its application captures. Dropping an unstarted supervisor signals drain and
cancellation and wakes readiness waiters with Draining, without running any
factories/finalizers or publishing a report. `run_until` transfers that guard
into the caller-owned driver, including before its first poll. Actual startup
stays lazy. Shutdown distinguishes
unobserved task results from unfinished tasks: ready joins are harvested at phase
boundaries, and only unfinished tasks receive abort requests.

The current policy deliberately skips dependent finalizers after panic, requested
abort, or unjoined direct tasks. This is conservative: aborting an Axum wrapper,
for example, is not proof that hidden connection work no longer owns a pool.
The caller gets explicit failure instead of a misleading clean shutdown.

## Explicit errors, not a universal error hierarchy

OperationError<E> differentiates returned E from cancellation/deadline. RetryError
retains E and the retry stop reason, including the previous E when interrupted
in backoff or a later attempt. Neither can classify an external commit outcome
without application/protocol knowledge.

TaskRecord and CleanupRecord retain boxed original causes because heterogeneous
components meet there. Automatic telemetry logs only category/name/count fields.
A trusted sink may inspect causes, but HTTP serialization must use sanitized
application-owned mappings. See [security](../SECURITY.md).

Scoped tracing is an execution-lifetime concern, not just a polling concern.
A private wrapper restores the originating dispatcher during both polling and
destruction of Batter-owned futures, including their nested spans. Operations
and HTTP boundaries capture it on first poll; admitted finite work captures it
at submission. This requires no downstream wrapper, global subscriber, extra
task, or per-operation heap allocation.

`batter::telemetry::with_current_dispatch` exposes this behavior as an opaque
future for adapter authors. It captures the current dispatcher when called and
protects both inner polling and destruction. It does not capture or enter the
current span; instrument the inner future when needed. The Axum adapter calls it inside
its async boundary, preserving the existing first-poll capture. Observations and
nested spans must live inside the protected future. This seam introduces no
task ownership, cancellation shielding, global subscriber, or runtime abstraction.

## Why there is no resource graph yet

Explicit constructors and cleanup ordering are easier to inspect than an
unproven generic DI graph. Shared immutable services can use normal Arc ownership.
Use narrow dependencies or Axum FromRef in applications. Do not pass a giant
AppContext merely to avoid constructor arguments. Add construction machinery
only after repeated real applications demonstrate that it simplifies—not hides—
resource ownership and partial-failure behavior.
