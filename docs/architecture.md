# Architecture

## A foundation of operational contracts

Batter's unit of reuse is an invariant: who owns this work, which deadline bounds
it, who observes its failure, and when may its dependencies close? It is not a
collection of wrappers around every dependency.

The source is one library crate with small modules plus a separate test-support
crate. Public functions accept native futures, concrete errors, and runtime
primitives. Only process/cleanup boundaries erase errors into BoxError, because
those boundaries aggregate heterogeneous component results.

```text
application composition root
  |-- native services / concrete constructors / domain errors
  |-- batter lifecycle + cleanup
  |-- batter operation + retry + admission
  |-- native tracing subscriber and exporters (application-owned)
  |-- optional batter HTTP adapter -> Axum / Tower
  |-- native SQLx PgPool / Transaction
  |-- Runlimit, Runledger (future thin adapters)
  `-- tests -> batter-test-support + postgres-test-harness (future composition)
```

The default graph has Tokio, tokio-util, tracing, thiserror, and pin-project-lite.
The latter provides safe pin projection for a private, allocation-free tracing
context wrapper; it was already a transitive dependency. Axum/Serde are
optional. SQLx is optional for the example. There is no TypeScript runtime,
algebraic-effect datatype, service locator, runtime-neutral abstraction, or
cyclic dependency on the user's reusable libraries.

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

Caller-owned driver cancellation is armed when `run_until` takes ownership,
including before its first poll. Actual startup stays lazy. Shutdown distinguishes
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

## Why there is no resource graph yet

Explicit constructors and cleanup ordering are easier to inspect than an
unproven generic DI graph. Shared immutable services can use normal Arc ownership.
Use narrow dependencies or Axum FromRef in applications. Do not pass a giant
AppContext merely to avoid constructor arguments. Add construction machinery
only after repeated real applications demonstrate that it simplifies—not hides—
resource ownership and partial-failure behavior.
