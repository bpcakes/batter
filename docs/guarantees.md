# Guarantees, preconditions, and non-guarantees

These describe the implemented contract and its limits. All workspace packages
retain Rust 1.94 as their minimum, with development pinned to Rust 1.98.1.
SQLx 0.9.0 belongs only to the unpublished PostgreSQL example package. See [validation](validation.md)
for executed checks. Passing tests do not establish guarantees beyond their scope.

## Execution boundary

OperationContext uses a Tokio monotonic deadline. Children clamp their deadline
to the parent and inherit cancellation in one direction. Clones of one context
share its cancellation token; dropping the value alone does not cancel clones.
`run` creates a child token and cancels it on return or drop. The original parent
remains reusable until cancelled or expired.

Preflight checks prevent starting a factory that is already cancelled/expired.
When branches are simultaneously ready, cancellation precedes deadline, which
precedes future completion. A just-completed side effect can therefore have an
interrupted caller outcome. This is intentional; it does not imply rollback.
A future that never yields can exceed any configured time allowance.

Returned errors retain their concrete type. Panics propagate to a task boundary;
ordinary OperationError is not a panic recovery mechanism. A never-polled run
produces no work and no telemetry. Dropping a polled run records `dropped`, not a
fabricated application failure, and signals its child scope.

Batter restores the originating tracing dispatcher during polling and destruction
of its observed futures. Completion/drop events and nested span destruction stay
with that subscriber when a runtime abort happens outside the original poll.
Operations and HTTP boundaries capture this context on first poll; this does not
install global state or supervise application tasks spawned outside Batter.

Adapter authors can use `batter::telemetry::with_current_dispatch` to preserve
the dispatcher at wrapper construction through inner-future polling and
destruction. It returns a future and adds no task, heap allocation, or `Send` /
`'static` requirement to the wrapped future. Keep observations and nested spans
inside it. It does not capture or enter the current span, drive a dropped future
to completion, or supervise work.

`reserve_finalization` divides an existing context into sibling work/finalization
contexts. Work ends at the original deadline minus the positive reserve;
finalization keeps the original deadline. Cancelling/finishing work does not
cancel finalization; parent cancellation cancels both. The caller must await
finalization explicitly while the enclosing owner remains active. The reserve
does not extend total time, shield parent cancellation, or prove rollback.

**Not supplied:** asynchronous descendant joining, preemption, cancellation
shielding, remote cancellation acknowledgement, transaction rollback proof,
commit certainty, or exactly-once effects.

## Retry contract

Each attempt invokes a fresh factory. The initial call is attempt 1 and consumes
the attempt limit. Only returned application errors reach the classifier.
ReplaySafety is an explicit caller assertion, not an enforcement mechanism or
proof about an HTTP method/SQL statement. Never permits only the first call.

A single deadline covers attempts and backoff. Exponential backoff saturates at
the configured cap. RetryAfter is a provider LOWER bound and is combined using
max, never min. When the required sleep consumes all remaining budget, return
InsufficientBudget immediately, retaining the application error; do not falsely
report an already elapsed deadline. The library makes no estimate of how much
execution time the next attempt will require.

Cancellation/deadline stop the sequence and are not automatically replayed.
The latest returned error remains in the interrupted result when available.
A current timed-out attempt may have no returned error at all. Panics are not
classified. `execute` retains deterministic backoff. `execute_with_jitter` takes
a caller-supplied u64 sample stream and selects equal jitter in
`[max(1ns, backoff / 2), backoff]`, then applies the provider lower bound with max.
Tests can replay samples; production callers need independently seeded sampling.
There is no global RNG, fleet coordination, retry-token budget, or circuit breaker.

Replay keys, provider deduplication retention, payload matching, transaction
isolation, and ambiguous commit handling belong to the application/upstream
protocol. Choose one retry owner so nested budgets do not multiply attempts.

## Admission contract

Bulkhead is a process-local semaphore. Reject fails immediately without a free
permit. Wait uses the caller's total deadline. The returned permit is Tokio's
OwnedSemaphorePermit; retain it until the bounded work actually finishes.
Closing admission wakes waiters but does not revoke acquired permits.

This bounds permit holders, **not** waiting callers, request bodies, socket
buffers, database connections in other components, or global fleet concurrency.
Context preflight cannot atomically serialize a concurrent cancellation with
semaphore acquisition; code must still check/run its admitted operation.
Rate limiting, concurrency limiting, and retry accounting are separate policies.

## Process ownership and shutdown

Every registered component is long-lived and critical. Registration itself does
not start work. All direct JoinSet completions are observed by name, including
factory panics and early successful exits. Errors continue to be collected during
shutdown. Completion captures whether drain had started; a later drain cannot
reclassify an earlier successful critical exit as expected. There is no restart.
Application `mark_ready` arms readiness: Ready requires a running driver plus
every registered critical component's `mark_started` acknowledgement after
actual initialization. Acknowledgement is an application assertion, not an
inspection of its internal descendants. Forgotten acknowledgement leaves Starting.

`with_process_capacity` adds finite process-owned admission. `try_spawn` rejects
synchronously before startup/readiness, after root drain, or at capacity; it
creates no queue of permit waiters and never invokes rejected factories. Queued
plus executing tasks hold capacity until termination, not until their receipt is
dropped. Payload sizes and detached work are not bounded by the task count.
Active `ProcessScope` descendants may enter during drain and use the same bound;
escaped inactive scopes and forced cancellation cannot admit them. Parent/child
capacity is shared: awaiting a child at full capacity requires handling rejection,
not creating an unbounded waiter. Finite work remains in-memory, not durable.
Its callback carries the submitting span/subscriber, while process supervision
and dependency cleanup retain the driver's diagnostic context.

A finite task's `Err(E)` initiates process drain. Expected business denial should
be `Ok(Err(denial))`, not a fatal task error. Original failure E is shared with the
typed receipt and retained report. Successful finite work is counted, not retained
as an ever-growing history. Failures close admission and are retained for the
bounded outstanding set. Panics/aborts produce a terminated receipt and an
observed task failure in the report; they do not fabricate a typed E.

On component failure, the caller's shutdown future remains owned by the driver
through dependency cleanup. Splitting shutdown coordination into private helpers
does not end that future's lifetime early.

Drain notifies components to stop admission/claiming without immediately
cancelling admitted request contexts. Forced cancellation follows the drain
allowance. Abort is requested only after a further cooperative allowance.
An additional bounded phase attempts to observe direct task termination.
Ready results are harvested before escalation and final reporting. Being present
in a JoinSet does not alone make a task an abort target: already-finished tasks
retain their actual outcomes and do not suppress cleanup as requested aborts.

Total configured allowance is drain + cancel + abort observation + cleanup work
+ cleanup final abort observation. Scheduling delays, synchronous blocking,
non-yielding task bodies/destructors, and OS suspension can exceed it. This is
not a hard wall-clock SLA. Nonpreemptible blocking work requires its own owner
and shutdown strategy; Batter has no spawn_blocking wrapper.

A direct task join says nothing about descendants it detached. Component authors
must preserve ownership themselves. After any requested abort, observed panic,
or unjoined direct task, dependent finalizers are conservatively skipped.
The report is unsuccessful even if the immediate wrapper was joined afterward.

`start` explicitly launches an owned coordinator and completion monitor.
`RunningSupervisor::wait`/`shutdown` and `SupervisorObserver::wait` may be cancelled
without cancelling the driver or finalizers. Last-owner drop requests graceful
shutdown; observers and admission/control handles do not prolong ownership.
Observers share the retained report or coordinator JoinError. The Tokio runtime
must remain alive; process/runtime termination cannot be shielded.

The lower-level `run_until` remains caller-owned. Constructing its future transfers
ownership and arms emergency cancellation without starting factories or publishing
readiness. Dropping it, even before its first poll, withdraws readiness, wakes
readiness waiters, and signals drain and forced cancellation. After startup,
JoinSet also requests abortion of owned tasks, but Drop cannot await them or run
asynchronous finalizers. Never put an outer timeout
around run_until and then describe the result as completed graceful shutdown.
Drive its own phased protocol and inspect the report. A process watchdog may
terminate an unresponsive binary, but no cleanup guarantee survives that action.

## Cleanup contract

Finalizers are owned Send factories, invoked inside directly owned Tokio tasks.
They run sequentially in LIFO order. Per-hook work is capped by a shared work
deadline. After timeout, abort observation is capped by both a per-observation
allowance and one final absolute deadline. Total phase allowance cannot reset
once per hook. Errors and observed panics are retained while later hooks run.
Every skipped hook receives one report entry and one warning, in LIFO order;
skipping never invokes the registered factory.

A timed-out finalizer that is joined has stopped its direct future. If joining
cannot be observed, remaining dependency hooks are explicitly skipped. Hooks
must not detach work; joining a hook cannot prove hidden work completed. No
source or panic payload is automatically included in tracing fields.

Dropping CleanupStack does NOT call the hooks. It emits a count-only warning.
Skipping or dropping a hook still drops its captured values using their native
Rust Drop behavior; it does not keep resources alive indefinitely or suppress
those types' own destruction protocols.
Dropping close mid-flight aborts its local task but cannot await completion or
return a report. Shutdown/readiness, resource acquisition, and cleanup are not
interruptibility-masked like a full managed effect runtime.

Register cleanup immediately after successful acquisition, but do not confuse
this convention with an atomic acquisition/registration guarantee. The caller
must handle partial acquisition and initialization according to native resource
contracts. Resource values must not require an unavailable runtime after shutdown.

## HTTP boundary

Handler panics propagate through this middleware; `HttpFailure::Internal` is not
an automatic panic catcher. Process-task and cleanup-hook panic observation does
not establish HTTP recovery or redact Rust's default panic-hook output.
This is source-inspected behavior, unverified by a dedicated HTTP handler-panic test.

The admission point is the readiness read. A request racing drain may be admitted
when that read sees Ready. It receives an OperationContext extension tied to
forced process cancellation, not immediate drain. Server-side duration is fixed
by RequestPolicy; the middleware trusts no client deadline or proxy metadata.

The timeout ends when Next returns a Response. It does not bound streaming body
polls, WebSockets, an upstream Tower queue, or slow upload behavior occurring
outside this layer. An escaped request context is cancelled at response
construction, so a streaming design needs a distinct owner.

Client disconnect triggers operation drop only when the transport actually drops
the handler future. There is no promise of immediate universal disconnect
propagation. Real connection behavior still requires tests.

Default infrastructure responses omit raw errors and use application/problem+json
with stable codes. `with_failure_renderer` can select an application envelope,
status, and headers using a request-parts snapshot taken before the handler runs.
The callback owns output sanitization and must use trusted metadata explicitly;
Batter does not trust a client correlation header for it. The callback covers
middleware failures, not application responses or separate health probes.
Default server execution exhaustion uses 503, not a claim that client
upload timed out. No automatic Retry-After authorizes write replay. Authentication,
authorization, trust boundaries, body limits, request IDs, and application errors
are not supplied. Liveness/readiness must be mounted outside the readiness gate.
