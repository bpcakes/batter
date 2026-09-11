# Architecture

## Agent-only consumption

Batter's integration APIs are designed for autonomous coding agents. The
canonical path should make ownership, cancellation, joining, registration,
finalization, deadline relationships, and error retention follow from
library-driven execution and constrained interfaces. Repeated caller
obligations to reconstruct those protocols are design debt, even when they are
documented. This policy does not move application-specific protocols into the
foundation or claim that types can prove arbitrary remote effects.

Examples are consumer contracts, and lower-level escape hatches must state the
obligations they leave with callers without appearing equivalent to the
protected path. Proposed design changes should be exercised with independent
failure scenarios and fresh-agent implementation or modification tasks. Review
quality and test volume alone are insufficient; absent recorded execution,
agent-usability evaluations remain proposed and unexecuted.

## A foundation of operational contracts

The workspace targets Unix backends. Windows is unsupported and not planned;
process and signal boundaries use native Unix semantics without non-Unix
fallbacks. [ADR-007](adr/007-unix-platform-scope.md) records this platform policy.

Batter's unit of reuse is an invariant: who owns this work, which deadline bounds
it, who observes its failure, and when may its dependencies close? It is not a
collection of wrappers around every dependency.

The root is a virtual Cargo workspace. The `batter` foundation,
`batter-axum` and `batter-sqlx` adapters, and `batter-test-support` utilities are separate libraries;
`batter-example-postgres-lifecycle` is an unpublished executable package;
`batter-example-reference-service` owns native upstream compatibility probes.
Public functions accept native futures, concrete errors, and runtime
primitives. Only process/cleanup boundaries erase errors into BoxError, because
those boundaries aggregate heterogeneous component results.

```text
application composition root
  |-- native services / concrete constructors / domain errors
  |-- batter lifecycle + cleanup
  |-- batter operation + retry + admission
  |-- batter settings (explicit sources; application schemas)
  |-- native tracing subscriber and exporters (application-owned)
  |-- optional batter-axum -> batter + Axum / Tower
  |-- optional batter-sqlx -> batter + native SQLx PgPool / Transaction
  |     `-- opt-in test-support -> external harness + generic test support
  |-- Runlimit, Runledger (future thin adapters)
  `-- reference tests -> batter-test-support + external postgres-test-harness
```

The foundation graph has Tokio, tokio-util, tracing, thiserror, and pin-project-lite.
The latter provides safe pin projection for a private, allocation-free tracing
context wrapper; it was already a transitive dependency. Axum/Serde belong to
the HTTP adapter package. SQLx belongs to the optional PostgreSQL adapter and examples. There is no TypeScript runtime,
algebraic-effect datatype, service locator, runtime-neutral abstraction, or
cyclic dependency on the user's reusable libraries.

`batter-test-support` depends on neither the foundation nor an adapter. Core
tests can use its generic scripts without pulling higher layers back into the
foundation. Cross-package fixtures belong in their application/example test
targets. Reusable pool/lease/template composition is opt-in under the SQLx
adapter test-support feature, selected by reference development dependencies.
The external harness retains provisioning and template-cache ownership. The optional SQLx
adapter shares the observed connection disposition mechanism while preserving
native transactions and application policy. Runlimit and Runledger adapters still
require proven shared mechanics. See [ADR-006](adr/006-workspace-packages.md).

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

The private `batter-axum/src/observation.rs` module owns the observation guard,
response facts and completion events. Its closure-based `observe_response`
entry composes with admission without importing `RequestPolicy` or lifecycle
state. Public middleware and `HttpObservationLevel` retain their crate-root paths.

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

`startup::Startup` owns an explicit native initializer from `start` through
failure cleanup or transfer to `RunningSupervisor`. Its boxed future borrows a
`StartupScope`; no service registry or dependency graph is introduced. Reserve a
cleanup name before acquisition, then register the finalizer synchronously after
success. Register dependencies before dependents so teardown reverses the order.
Application stages and initialization/cleanup budgets remain explicit inputs.

The sole `StartingSupervisor` owns the running handoff. Cancelling its borrowed
waiter leaves initialization running; dropping it requests drain. The startup
coordinator retains registered finalizers through failed-startup cleanup.
Observers retain outcomes without keeping a running owner alive. Initializer
construction, polling and destructor unwinds are caught separately; original
application failures and destructor panics coexist in the startup report.

By default, successful initialization arms readiness and starts the existing
owned driver. `without_readiness_approval` deliberately leaves application
approval to the running owner. Each registered component must still acknowledge
actual initialization with `ShutdownSignal::mark_started`. Drain cannot be
reversed by late approval.
`register_signals` installs native SIGTERM/SIGINT listeners during initialization;
installation errors enter startup cleanup before readiness. Applications with a
long initializer can instead call `install_signals`, select on the returned
sources during initialization, and transfer them into their critical component
before handoff. Completed reception is retained through registration. The staged
reference root uses that path around its complete awaited initializer. Tokio's process-wide signal handlers remain installed after listeners
are dropped.

That staged worker keeps its complete native join under an independent owner.
Preparation is independently owned before acquisition, and native spawning
transfers synchronously into the native driver. Its control session is monitored,
and bounded release observations are retained before dependency cleanup. Native
termination proof, release certainty and caller observation remain separate facts. Registration rejection returns
the already-stopping host to the caller, rather than discarding its only
awaitable owner. These are example composition contracts, not new foundation
supervision or database abstractions.

The lower-level `take_cleanup` pattern remains explicitly caller-driven. Its
caller cancellation can abandon asynchronous cleanup. Resources not yet
registered retain native Drop behavior even inside owned startup: reserve before
acquisition and avoid suspension between acquisition success and registration.
No async Drop, detached-child ownership or runtime-death guarantee is added.

At an executable boundary, use a redacted outer error whose source remains the
concrete inner failure. This keeps early startup errors inspectable without
printing their contents through `Result` termination. An unsuccessful owned
driver report must likewise remain an owned `ShutdownReport`; rebuilding an
`io::Error` from its `Display` text discards task and cleanup errors.
`check_shutdown` returns a redacted `ShutdownFailure` retaining the original
unsuccessful report or coordinator JoinError. Both service examples use it.

## Dependency observation

`health::HealthMonitor` is one non-cloneable owner, driven by an ordinary
supervised run future. `HealthReader` clones retain the latest concrete result
and monotonic completion time without retaining writer ownership. A short private
mutex makes publication coherent; error destruction occurs outside it. Readers
compute freshness at each call, so an unscheduled owner cannot extend success.
The owner stops on drain, destroys its active future and invalidates readers.
Probe errors/timeouts are recoverable dependency states, while panics remain
critical component failures. There is no hidden task or service registry.

The HTTP composition registers a monitor during owned startup and passes only
its reader to the readiness route. The route combines process state and cached
dependency status; it never queries a dependency. Acknowledging the monitor's
initialization does not make an unknown dependency healthy. Application timing
policy and actual probe implementation stay in the composition root.

## Shutdown state machine

The private `lifecycle/tasks.rs` module owns direct tasks and all join accounting.
The coordinator selects phases through narrow operations, never a raw `JoinSet`
or mutable record collection. Waiting for an exit consumes and records its join
in one poll before returning a shutdown cause; cancelling a pending wait cannot
lose a consumed result. Finishing transfers an owned summary and releases the
task collection before dependency cleanup, without claiming unjoined tasks have
terminated. Phase deadlines and conservative cleanup policy remain coordinator-owned.

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
coordinator and publishes a retained report or JoinError. `start` creates the
completion channel; only the monitor owns its sender. Completion observers come
from `RunningSupervisor`, so every observer has an owned driver publisher.
Shared readiness/admission state and `ShutdownHandle` carry no completion channel.
The lower-level `run_until` and `CleanupStack::close` remain cancellation-fragile when driven
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

Critical components, finite tasks and cleanup hooks use the native
`Span::or_current` fallback when their INFO task span is disabled. Select this
context before spawning or enqueueing; retaining the dispatcher alone cannot
preserve an enabled application parent across a task boundary. Finite admission
performs both span creation and fallback lookup outside its transition lock,
since either can call application subscriber code.

`batter::telemetry::with_current_dispatch` exposes this behavior as an opaque
future for adapter authors. It captures the current dispatcher when called and
protects both inner polling and destruction. It does not capture or enter the
current span; instrument the inner future when needed. The Axum adapter calls it inside
its async boundary, preserving the existing first-poll capture. Observations and
nested spans must live inside the protected future. This seam introduces no
task ownership, cancellation shielding, global subscriber, or runtime abstraction.

## Explicit settings

`settings` reads only caller-supplied pairs, literals or an explicit file path.
It parses bounded integers and durations, and redacts Display/Debug for secrets
and static diagnostic wrappers. It does not search for `.env`, read the process
environment, or own application field names. HTTP and reference roots keep their
schemas and pass validated values into native constructors. See
[guarantees](guarantees.md#explicit-settings-and-redacted-diagnostics).

## Why there is no resource graph yet

Explicit constructors and cleanup ordering are easier to inspect than an
unproven generic DI graph. Shared immutable services can use normal Arc ownership.
Use narrow dependencies or Axum FromRef in applications. Do not pass a giant
AppContext merely to avoid constructor arguments. Add construction machinery
only after repeated real applications demonstrate that it simplifies—not hides—
resource ownership and partial-failure behavior.
