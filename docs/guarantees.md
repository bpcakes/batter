# Guarantees, preconditions, and non-guarantees

These describe the implemented contract and its limits. All workspace packages
retain Rust 1.94 as their minimum, with development pinned to Rust 1.98.1.
SQLx 0.9.0 belongs only to the unpublished PostgreSQL example package. See [validation](validation.md)
for executed checks. Passing tests do not establish guarantees beyond their scope.

The workspace's platform scope is Unix-only. Windows is unsupported and not
planned; no non-Unix process or signal fallback is provided. Linux x86_64 and
macOS arm64 have execution evidence on both supported toolchains. The updated
macOS CI job and other Unix targets remain unverified. See [ADR-007](adr/007-unix-platform-scope.md).

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

Readiness only moves forward: Starting may become Ready or Draining, Ready may
become Draining, and coordinator completion publishes Stopped. Stopped cannot
be reverted by a concurrent shutdown request or late startup acknowledgement.
The private lifecycle state module serializes all transitions with admission;
its atomic readiness snapshot is published under the same mutex guard. Reads
remain available while admission is held. Explicit readiness/drain/cancellation
wakeups happen after releasing the guard; native queue enqueue can wake its
receiver while retaining admission.

`with_process_capacity` adds finite process-owned admission. `try_spawn` rejects
synchronously before startup/readiness, after root drain, or at capacity; it
creates no queue of permit waiters and never invokes rejected factories.

Admission errors are classified after name validation: permanent closure
(`Closed`) takes precedence over startup (`NotRunning` / `NotReady`) and
capacity (`Full`). Root drain, forced cancellation, task failure, an expired
ancestor, or a closed coordinator queue permanently closes the affected
admission path. This includes shutdown before startup, dropping/aborting an
unpolled driver, and dropping an unstarted supervisor. A fresh supervisor still
returns `NotRunning`.

Queued plus executing tasks hold capacity until termination, not until their
receipt is dropped. Payload sizes and detached work are not bounded by the task
count. Active `ProcessScope` descendants may enter during drain and use the same bound;
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

The initial task-triggered shutdown cause distinguishes ownership:
`ShutdownCause::ComponentExit(name)` identifies a registered critical component;
`ShutdownCause::FiniteTaskExit(name)` identifies an admitted finite task that
returned an error or panicked. Finite labels may repeat across invocations;
the cause carries a label, not a unique invocation ID. Successful finite work
does not initiate shutdown. The coordinator selects the cause when it begins
shutdown. A ready explicit request takes priority over task results in the same
poll, including failures that completed before the request but remain unobserved.
For task-triggered shutdown, the cause reflects observation order rather than a
guarantee about chronological failure order. Later observations do not replace
the selected cause, including after an explicit `Requested` shutdown.
Shutdown aborts occur after the cause is selected; inspect `abort_requested` and
`TaskRecord::outcome` for requested and observed aborts, and task/cleanup outcomes
for all subsequent failures.

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

The [subprocess tests](testing.md#non-yielding-subprocess-tests) exercise these
limits with a direct task whose poll never returns. When another worker can
drive shutdown, the report retains its name in `abort_requested` and `unjoined`,
contains no joined task outcome for it, and skips dependent finalizers without
invoking them. `Readiness::Stopped` means the coordinator finished; the direct
task can still be live and native Tokio runtime destruction can still block.
On a blocked current-thread runtime, even a previously polled timer and the
shutdown report cannot progress. An external process kill supplies containment
for these tests. The fixture also exits on parent-pipe closure or its independent
emergency deadline, without releasing the blocked task or running its Rust
destructors. These are private test-process controls, with no claim of library
preemption or application finalization after process termination.
The blocked-runtime test watchdog bounds startup separately from observation:
five seconds from spawn to capture a complete drain record, then the fixture's two-second
observation plus one second of margin. Startup cannot consume that observation
window; missing startup evidence fails and triggers kill/reap. The combined
eight-second allowance precedes the ten-second emergency exit, subject to OS
scheduling/process-control limits. Watchdog evidence requires the selected
deadline to have elapsed when the first kill is requested, including when it
extends past five seconds. A live regression pins the selected wait deadline to
captured drain time plus observation for both blocked scenarios. Elapsed time
after reaping/capture is diagnostic only.
The delayed-panic control derives its synchronization bound from the scenario's
maximum wait, including observation after the latest accepted startup. It does
not impose a second, shorter startup deadline. Live wiring controls also reject
kill requests delayed by a full observation allowance past the selected deadline;
this is an executed scheduling check, not an OS latency guarantee.
Startup and final validation share one exact, complete-line protocol parser.
Capture timestamps and deadline decisions use the same mutex: an on-time record
survives late polling, while a late record fails and polling cannot reset the
observation allowance. Later panic diagnostics cannot erase an on-time startup
record: that record establishes timing, not a clean outcome. Final validation
still rejects every captured child panic. A known panic without a startup record
fails promptly, and capture overflow always invalidates startup evidence.
Capture retains 64 KiB of raw diagnostics and metadata
for at most 64 distinct events. Either overflow, or an unfinished protocol
record at final validation, prevents a successful evidence claim. These controls
measure parent-side capture time, not the child's unobservable exact write time.
The capture reader retries interrupted reads. An I/O error or reader panic makes
its explicit finish fail after joining the reader; partial bytes are diagnostics,
not proof of complete capture.
An event wait that observes child exit joins the reader before judging final
startup evidence. Byte and event limits report distinct causes; the first
detected cause survives later overflow. Live controls require natural exit after
flooding output beyond each limit, proving that overflow does not stop draining.
The Linux parent-death probe must reject incorrect termination and forbidden
cleanup evidence even when Python optimization is enabled; its checks do not
depend on Python's removable `assert` statements.
The fixture-owner unwind control must finish within its five-second startup
budget, before the child's ten-second emergency exit; waiting for that fallback
does not establish prompt kill/reap behavior.

A direct task join says nothing about descendants it detached. Component authors
must preserve ownership themselves. After any requested abort, observed panic,
or unjoined direct task, dependent finalizers are conservatively skipped.
The report is unsuccessful even if the immediate wrapper was joined afterward.

The [seeded scheduling suite](testing.md#seeded-scheduling-exploration) exercises
these contracts with bounded two/four-worker workloads, controlled action orders
and independent result accounting. A replay seed reconstructs test choices, not
Tokio or OS scheduling. Delayed-coordinator regression cases establish completion
before escalation; eventual success by itself does not make a racing abort false.
Cooperative work can legitimately miss a live shutdown deadline. The scheduling
oracle reconciles either its successful completion or its observed abort with the
report and cleanup decision, including skipped cleanup after an abort request
that races with successful completion. Paused-clock regressions preserve exact
cooperative-success and late-completion assertions without an OS latency premise.
Descendant closure cases keep post-closure admission rejection strict while
reconciling each admitted receipt with completed or named aborted work. They
preserve the original closing-task error and verify actual finalizer execution
versus explicit skipping. Controlled force- and task-failure cases exercise both
prompt completion and observation delayed beyond the cancellation allowance.
Process watchdogs contain hung tests and preserve bounded diagnostics, without
establishing application finalization. Scheduling child launch requires explicit
arguments and a PID-bound record; ambient flags leave ordinary discovery inert.
The test process owner distinguishes direct-child exit from pipe EOF, preserves
partial evidence on failed observation, and reports escaped pipe owners as
incomplete output rather than successful cleanup. Its 140-second maximum watchdog
and five-second cleanup observation precede the 149-second emergency backstop.
The test owner retains the direct child, alive or unreaped, until pipe EOF or the
group-termination decision; a reaped child never authorizes a later group signal.
Partial capture construction closes its selector before propagating an exception,
including interruption; descriptor release does not depend on garbage collection. The private process
owner requires the main thread, default SIGCHLD and a standard SIGINT disposition
before launch; custom or unknown SIGINT handlers are rejected. An inherited
ignored SIGINT stays ignored in the owner and its children. For Python-default or
Unix-default SIGINT, it temporarily records a stop request across acquisition,
observation and cleanup, then restores the exact prior disposition after resource
release. Repeated signals do not reset cleanup deadlines, and interruption remains failed evidence.
Descriptor closure runs even when settlement exits exceptionally. This does not
shield unrelated raising signal handlers, process death or arbitrary Python code.
The outside-group pipe-writer control retains its own direct-child handle through
bounded cleanup, including exceptional observation paths. The finite non-yielding
fixture checks that unjoined work can retain a pending receipt after the report
is published. Its watchdog allows startup, the entire bounded case, and hang
observation before termination; a delayed-start regression requires the same
report evidence. Both unjoined fixture tests independently require the twelve-second
watchdog observation and termination/reaping before twenty-one seconds, including
cleanup and interpreter/startup margins. These elapsed checks do not establish an
OS scheduling guarantee.
Passing this corpus is not exhaustive proof or a stronger runtime-death guarantee.

Local verification overlaps the core and workspace test configurations and runs
the Python scheduling controls in isolated processes. Both configurations and
every discovered control remain required; sharding changes neither assertions
nor fixture deadlines. The parent rejects incomplete/duplicate shard completion,
failed or missing child outcomes, output overflow and interruption. Matrix logs
retain bounded initial output and separate stdout/stderr tails, marking omitted
bytes explicitly; retained final diagnostics do not make overflow valid evidence. Its bounded
process-group settlement retains leader identity until output EOF or termination,
and never signals after reaping. This is test-runner containment, not a new
application guarantee for detached descendants. See [testing](testing.md#jig-verification)
for execution bounds and the final-evidence reuse conditions.

`start` explicitly launches an owned coordinator and completion monitor, creating
their completion channel at that boundary. Obtain a `SupervisorObserver` only
from `RunningSupervisor::observer`, available immediately after `start` even
before the coordinator's first poll. `ShutdownHandle` provides lifecycle control
and readiness signals, not completion observation. Migrate former
`handle.observer()` calls to `running.observer()` after starting the supervisor.
`RunningSupervisor::wait`/`shutdown` and `SupervisorObserver::wait` may be cancelled
without cancelling the driver or finalizers. Last-owner drop requests graceful
shutdown; observers and admission/control handles do not prolong ownership.
Observers share the retained report or coordinator JoinError. The Tokio runtime
must remain alive; process/runtime termination cannot be shielded.
If runtime shutdown drops the completion monitor before publication, awaiting
the observer on another runtime panics rather than returning a fabricated report
or coordinator error. The same applies to `RunningSupervisor::wait`/`shutdown`.
An outcome published before runtime shutdown remains readable. Tests cover both
the unpublished-monitor failure and a late observer retaining a published report;
they do not establish cleanup or task termination after arbitrary runtime loss.

An unstarted `Supervisor` owns abandonment signaling from construction. Dropping
it withdraws readiness, signals drain and forced cancellation, and wakes
`wait_ready` with `Err(Readiness::Draining)` before dropping captured values.
It invokes no component or finalizer factory and publishes no completion report;
Stopped still means coordinator completion. Extracted cleanup must be explicitly
awaited. Extraction does not detach any operation tokens captured by its hooks:
dropping the supervisor cancels those tokens before the extracted stack runs.
Finalizers must perform teardown independently of process operation cancellation,
using the stack's cleanup budget and, if needed, a fresh `OperationContext::new`
rather than a context derived from `operation_token`. This also applies to normal
shutdown, which cancels process operations before closing resources. Unstarted
supervisors, standalone shutdown handles and caller-owned `run_until` drivers
cannot construct completion observers; await `run_until` directly for its report.

The lower-level `run_until` remains caller-owned. Constructing its future transfers
the abandonment guard without starting factories or publishing readiness. Keeping
that future unpolled leaves startup pending. Dropping it, even before its first
poll, withdraws readiness, wakes readiness waiters, and signals drain and forced
cancellation. After startup,
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

`request_admission` applies the combined readiness/deadline `RequestPolicy` and
inserts `OperationContext`. `observe_http` independently observes response
construction without lifecycle state, a deadline or a context extension. The
existing `request_scope` combines those behaviors for compatibility. Each
installed observer emits its own HTTP completion event; use outer `observe_http`
with inner `request_admission` to avoid duplicate observations. Operation events
remain separate. Subscriber filtering and transport delivery are application-owned.

HTTP completion events carry normalized method, matched route template (or
`<unmatched>`), actual numeric status when a response exists, HTTP outcome and
construction latency as event fields. The observer retains these facts separately
from its INFO span, so disabling that span does not remove fields from an enabled
completion event. The span still carries the same fields for nested context;
formatters may show them in both places. Application-owned correlation in other
spans remains subject to those spans' filtering.

At first poll, observation retains its enabled HTTP span or the current enabled
application span for execution and completion parenting. Later ambient spans
cannot replace the completion event's parent; if neither was available, that
event remains a root event. HTTP fields are never recorded into the inherited
application span. Per-layer subscriber filters can still hide retained context
from an individual sink; this is not an exporter delivery guarantee.

Completion events default to WARN for 5xx responses and INFO otherwise, including
probes. `HttpObservationLevel(tracing::Level)` in response extensions selects a
different event level in both observers. It does not change response status,
HTTP outcome, field sanitization, the INFO span level, or event count. A 503 at
INFO still has `http_outcome="server_error"`; status/outcome alerts need their own
probe policy. DEBUG/TRACE events require subscriber configuration that enables
them. This is event severity selection, not guaranteed delivery or suppression.

Handlers, failure renderers or middleware inside observation must explicitly
attach the override to the returned response. It remains in the response's
extensions for other middleware and is not serialized as a header. Middleware
replacing a response/status owns retaining, replacing or removing its override.
Nested observers each read that retained override. An observer records the status
and override returned by its inner service; middleware outside it can subsequently
rewrite the response without changing that already completed observation.
Request extensions, client headers, route names and missing OperationContext do
not infer severity. Built-in probes do not add overrides. A future destroyed
without returning a response remains WARN, even if the handler constructed an
annotated response internally. No policy callback runs from the guard's Drop.

Assemble guarded routes, unguarded probes and fallback before applying observation
with `Router::layer`. Axum runs that layer after routing, so matched route
**templates** are available; raw paths, queries, headers, bodies and error contents
are not recorded. Nonstandard methods normalize to `OTHER`; absent route metadata
uses `<unmatched>`. A wrapper outside routing lacks that metadata at entry even
for a matching route. Routes appended after `Router::layer` bypass it. Place
trusted identity outside observation and rejecting/status-changing middleware
inside it to observe their returned status.

A polled response future emits its actual response status/outcome and construction
latency on completion, or `dropped` without a status if destroyed before a response.
The first-poll subscriber protects full future destruction, including the
observation guard and nested instrumented spans. Never-polled entry points do no
application work and emit no completion. Dropping the returned body afterward
emits no second HTTP completion and is not classified as a dropped request.

Handler panics propagate through this middleware; `HttpFailure::Internal` is not
an automatic panic catcher. Process-task and cleanup-hook panic observation does
not establish HTTP recovery or redact Rust's default panic-hook output.
Dedicated tests verify an unwind before a response propagates as a Tokio task
panic, cancels admitted context, and emits one WARN `dropped` HTTP event without
a status or panic payload. This covers Rust unwinding, not aborting panics.

The admission point is the readiness read. A request racing drain may be admitted
when that read sees Ready. It receives an OperationContext extension tied to
forced process cancellation, not immediate drain. Server-side duration is fixed
by RequestPolicy; the middleware trusts no client deadline or proxy metadata.
With the documented `Router::route_layer` composition, admission also wraps the
method fallback of a matched business path: an unsupported method returns 503
while Starting/Draining, and 405 while Ready. An unmatched path still reaches the
unguarded fallback and is observed separately.

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
