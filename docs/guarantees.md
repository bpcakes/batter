# Guarantees, preconditions, and non-guarantees

These describe the implemented contract and its limits. All workspace packages
retain Rust 1.94 as their minimum, with development pinned to Rust 1.98.1.
SQLx 0.9.0 belongs to the optional PostgreSQL adapter and unpublished example
packages, not the foundation or generic test support. See [validation](validation.md)
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
When an operation INFO span is filtered, execution and completion retain the
enabled application parent selected at first poll. Operation fields are recorded
only on the operation span; later polls and destruction cannot adopt an unrelated
ambient parent or overwrite inherited application fields.

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

`run_until` returns a `#[must_use] ShutdownReport`. Owned-driver `wait`,
`shutdown`, and observer `wait` return `Result<SharedShutdownReport, Arc<JoinError>>`.
The shared wrapper is also `#[must_use]`, so discarding it after `?` or `unwrap()`
warns; coordinator completion alone does not establish successful shutdown.
Inspect the report's task and cleanup outcomes. Cloning the shared wrapper
retains the same report and errors; dereferencing borrows `ShutdownReport`.
The wrapper displays `owned shutdown report`; its `Error::source()` exposes the
concrete report and its summary. Chain-walking diagnostics therefore show the
task/cleanup summary once, while allowing a concrete report downcast.
Debug retains application error contents. Propagating such an error out of a
`main` returning `Result` lets Rust print it through Debug; a sanitized Display
does not protect that boundary. Applications must select their exit output,
as shown by the complete `ExitCode` example on `SharedShutdownReport` and the
SQLx executable. Neither the foundation nor a report wrapper defines a
universal redaction policy for arbitrary domain errors.
These lints are advisory: binding, explicit dropping, or allowing the lint can
bypass the warning. They do not prove inspection or successful shutdown.

A finite receipt's
`ProcessTaskError::Failed` shares its `Arc<E>` with the report while exposing
the concrete E through `Error::source()`, so downcasts preserve domain identity.

Every registered component is long-lived and critical. Registration itself does
not start work. All direct JoinSet completions are observed by name, including
factory panics and early successful exits. Errors continue to be collected during
shutdown. Completion captures whether drain had started; a later drain cannot
reclassify an earlier successful critical exit as expected. There is no restart.
Application `mark_ready` arms readiness: Ready requires a running driver plus
every registered critical component's `mark_started` acknowledgement after
actual initialization. For direct `register`, acknowledgement is an application assertion, not an
inspection of its internal descendants. Forgotten acknowledgement leaves Starting.

The [component ownership comparisons](../crates/batter/tests/component_ownership.rs)
gate actual child initialization before acknowledgement, then gate child stopping
and join before dependent cleanup. A deliberately nonconforming wrapper returns
Ok during drain without joining its child: the direct report succeeds and cleanup
runs, yet the independently retained child answers a fresh request afterward.
The library cannot inspect hidden children or automatically skip cleanup because
they exist. Joining only the wrapper does not establish transitive termination.
Separate comparisons retain early-success failure, concrete task and cleanup
errors together, and native panic/abort JoinErrors with skipped finalizers.

`register_managed` is the adapter-facing path for a native runtime with its own
descendants. Name validation and the absolute startup context precede factory
invocation. An Err factory result asserts that validation rejected construction
before spawning native work; adapter implementations must enforce this. The
factory transfers initialization observation, native stop observation/control and
the complete native settlement driver together. Batter acknowledges initialization,
observes early native stop to drain peers, and retains the settlement owner outside
the direct waiter. A successful initialization poll is acknowledged only after
its future is destroyed and the startup context is rechecked; a returned error
remains intact if cancellation also occurs. Application readiness approval and dependency health remain
separate. No durable startup job is required by this protocol.

Before initialization, observed native stopping or settlement records
`ManagedInitialization::Stopped`; process drain records `Draining`. Native
termination still drains peers and retains its settlement and failures separately.

The supported native adapter accepts `PreparedSupervisor` from native
`SupervisorBuilder::prepare`, rather than arbitrary caller factories. Preparation
validates configuration and owns cloned handles without starting tasks. A live
supervisor cannot cross this registration boundary; rejection and unstarted drop
do not launch native work. Preparation errors propagate through owned application
startup and its registered cleanup. Adapter tests include compile-fail controls
for live supervisors and closures, plus runtime task-count observations.

`ShutdownReport.managed` freezes each component's observed initialization,
original typed native report, additional failures and pending settlement at the
end of bounded observation. Wrapper abortion cannot discard the only native driver.
Any pending settlement, native refusal of cooperative dependency release, or
panic prevents finalizer execution. A component observer can retain a late report;
it never changes the frozen process outcome or runs skipped finalizers afterward.
Original initialization errors, polling/destruction panics and later native reports
coexist. Every caught stop-callback panic is published before waiting for native
settlement, including callbacks for later clock tightening. A pending native
report cannot hide an already-observed callback failure. Default managed diagnostics omit their contents. The adapter's native
ownership/classification contract remains necessary; Batter cannot discover
arbitrary hidden tasks. [Managed contracts](../crates/batter/tests/managed_components.rs)
exercise actual Tokio descendants and controlled failures. Native adapter/live
execution is recorded in [validation](validation.md). A panicking third-party stop
callback can leave retained native settlement pending; the bounded process report
retains the failure and skips cleanup, without fabricating native termination.

Drain, cancellation and abort/reap use consecutive absolute boundaries anchored
at the first recorded process stop, including requests before driver polling.
Repeated requests and delayed phase observation cannot restart the allowance.
The native managed budget uses the parent's drain interval for native graceful
shutdown and its cancellation interval for native abort/join; parent abort/reap
remains available for final observation. The idempotent stop callback receives
the earliest known parent timestamp and returns the earliest native/enclosing
timestamp. An earlier native stop tightens the process clock; Batter propagates
later discoveries to components already settling. Active drain/cancel/reap waits
wake on tightening. The native first failure cause stays separate and is never
overwritten by a clock update. Concurrent components share these intervals rather than each
adding new process time. Cleanup uses its own budget after settlement. This does
not preempt a non-yielding task or make runtime death recoverable.

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
Critical components, finite tasks and cleanup hooks retain the available
application parent when filtering disables their Batter INFO span. The context
is selected before spawning (before enqueueing for finite work) and covers
polling and destruction, including abortion. Parent lookup for finite work
happens outside the admission lock. Subscriber and per-layer filters still own
which spans and events reach a sink.
A synchronous subscriber regression checks mutex availability during span
creation, current-parent lookup and parent cloning. It exercises real finite
submission without a competing coordinator, so a callback moved under the lock
fails immediately instead of relying on an async timeout to detect a deadlock.

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

## Owned finite commands

`Command::new` is inert; `start` retains the callback, its work context and
registered LIFO cleanup independently of waiters. Command cancellation stays
below the supplied parent context. Returning an error with `?`, factory/poll
unwinding or cancelling work still reaches registered finalization. Original
work results actually returned by a poll, late boundary interruptions,
future-destruction panics and cleanup
outcomes coexist in `CommandReport`. A failed cleanup coordinator cannot erase
the work result. `check_command` interprets the complete contract, and the shared
report warns when implicitly discarded. Formatting excludes value/cause contents.

Command interruption is captured at the final poll before destroying the work
future. Cancellation or deadline expiry during destruction cannot reclassify
completed work; destruction panics and cleanup failures still make the report
unsuccessful. This differs from startup readiness, which requires a live context
after initializer destruction before acknowledging readiness.

Cancelling borrowed waiters has no effect. Dropping `RunningCommand` requests
work cancellation; observers retain completion without keeping that owner alive.
Work completion cancels child contexts before cleanup. Finalizers must use their
native independent cleanup, not captured work cancellation tokens. Resources are
registered through validated reservations; no takeable stack lets the canonical
scope silently relinquish finalization. Unregistered resources and arbitrary
spawned tasks remain outside this future/cleanup ownership contract.

`Command::new` gives cleanup its own allowance after work stops. `Command::within`
reserves cleanup's total work and abort-observation allowance inside an enclosing
absolute deadline. Work and cleanup cannot restart that total. Scheduling delays
can consume the reserve and produce explicit skipped hooks. Neither variant can
preempt non-yielding code, undo remote effects or finalize after runtime death.
Published reports survive runtime destruction; an observer whose monitor died
before publication cannot fabricate a completed report. Tests in `tests/command`
cover these ownership/budget distinctions; the finite example exercises native UDP.

## Owned startup contract

`Startup` initializes a running supervisor, not a standalone finite command.
After successful initialization, no critical components **and** no configured
finite-work capacity produce `ShutdownCause::EmptySupervisor`. The initializer
can succeed and every finalizer can succeed while the shutdown report remains
unsuccessful. Configured finite capacity permits a component-free running
supervisor; root work still requires readiness, submission and explicit shutdown.
The executable startup rustdocs and `startup_composition` tests cover these
distinct outcomes without changing the empty-supervisor contract.

`Startup::scoped` is the canonical agent-consumer path. Its
`ProtectedStartupScope` exposes only validated stage selection, direct cleanup
reservation and a private-field `Registration` view. That view forwards ordinary
and managed registration but cannot start or replace the process, extract cleanup,
recover a `Supervisor`, or outlive its borrow. The sealed `RegistrationTarget`
lets native adapters accept either this protected scope or the lower-level
supervisor without allowing application-defined targets. Handles and contexts
captured before startup remain outside this restriction.

Protected startup retains application failures inside the redacted,
non-exhaustive `InitializationError<E>` envelope. The driver maps a returned `E`
at its poll boundary before destroying the completed initializer future, so the
application error and a separate destruction panic can coexist. It uses the same
coordinator, cleanup, report publication and running handoff as the legacy path.

`Startup::new` remains the explicitly lower-level compatibility path and invokes
no initializer. `start` launches the owner on a live Tokio
runtime. Dropping the inert builder only abandons its unstarted supervisor;
no asynchronous cleanup is implied. Once started, dropping `StartingSupervisor`
requests drain while the coordinator drives registered cleanup independently.
Cancelling `wait(&mut self)` leaves ownership and the pending handoff intact.
After its first completed result, that one-shot waiter must not be polled again.
`StartupObserver` retains a startup failure or a shutdown observer, never a
running owner. Dropping an unclaimed handoff requests running-driver drain.

Initialization checks the operation deadline/cancellation and process drain,
including before factory invocation and after successful future destruction.
While the initializer is pending, each private observation boundary first rejects
an already-ready drain, cancellation, deadline or configured Unix signal without
polling application work again. Otherwise it polls the initializer once and then
observes sources that became ready during that poll. A concrete application error
or initializer panic observed in that poll is retained; signal reception still
requests drain. A successful initializer observed with a signal transfers the
consumed reception into reserved registration, whose drain request is caught by
the final check, so it cannot publish readiness or require a second signal. An
independent destruction panic does not suppress that final lifecycle classification.
By default success arms readiness; the running driver and every critical
acknowledgement are still required. `Startup::without_readiness_approval` instead
hands off the running driver while application readiness remains Starting, so a
later composition stage can approve admission explicitly. Component
acknowledgement does not substitute for that approval. A drain request cannot
revive Ready. Failure preserves the
application's concrete error and static stage, interruption or unwind payload,
an independent initializer destruction panic, and all cleanup outcomes.
Stage metadata must be suitable for diagnostics. Default error formatting omits
cause contents; explicit error sources/panic inspection are trusted operations.
`PanicPayload::try_inspect` never waits for another inspector: concurrent or
recursive access returns `PanicPayloadBusy`. A callback panic does not poison
future inspection. Inspection remains synchronous and temporarily excludes other
inspectors; it does not clone, format or expose the retained payload automatically.

Cleanup has its own budget, independent of the startup operation context.
`reserve_cleanup` / `CleanupStack::reserve` validate names before acquisition;
the exclusive `CleanupSlot` registers its finalizer infallibly. An unused slot
adds no hook. The older `push` still consumes its factory on rejection. Native
acquisition cancellation may have external effects before yielding ownership;
resources acquired but not registered have only their native destruction
semantics. Register immediately after success without another await. No detached
work, arbitrary destructor, non-yielding initializer, aborting panic, process or
runtime destruction is covered. The default panic hook may still print secrets.
Unexpected coordinator termination retains a JoinError without inventing cleanup.

Protected `.with_unix_signals(name)` is inert until `start`. Start first retains
repeated selection as a configuration error; otherwise it checks existing drain
or interruption, reserves the component identity, and installs SIGTERM then
SIGINT synchronously before returning the owner. The coordinator owns reception
during initialization and fulfills the reservation with one real critical task
before running handoff. A signal observed during initialization requests drain;
it cannot approve readiness. Repeated signals do not restart cleanup's separate
fixed allowance or turn a timed-out finalizer into successful cleanup. Preflight failures skip the initializer but remain
owned through prior cleanup, even if the startup owner is then dropped. Cleanup
slots use a distinct namespace from component reservations.

`register_signals` and `install_signals` remain lower-level compatibility helpers.
The latter requires its caller to poll `InstalledSignals::received` during
initialization and register the same sources before handoff. Tokio changes
process-wide signal disposition and does not restore it on listener drop; a
partial installation failure is not rollback. Signal-enabled start requires a
live Tokio runtime with signal support. Reception is cooperative and is not an
atomic fence at kernel delivery.

Standalone finite commands use `Command` to retain work and cleanup independently
of the caller's waiter. The lower-level `OperationContext::run` and
`CleanupStack::close` remain separately driven operations; composing them manually
does not create an owned finalization driver. No remote database termination or
rollback guarantee follows from either local completion report.

For supervised services, `check_shutdown` accepts only a successful shutdown
report; failures retain the complete report, including forced abort, skipped
cleanup and unjoined work, or the original coordinator error. Its redacted
formatting does not inspect those causes.

The reference root uses owned startup and managed native settlement. Installed
Unix signals cover initialization before its first await and transfer to the
running driver. Native loop acknowledgement, fresh PostgreSQL sampling and
application readiness approval are separate facts. The native registry is empty
until a delivery handler exists; application readiness remains unapproved.
Production starts no durable control job, advisory-lock owner or reconciliation
pool. The startup allowance is 20 seconds. Process drain, cancellation and reap
allowances are ten, one and one seconds, followed by separately bounded pool
cleanup. Native graceful/abort settlement consumes the same process stop clock.

Applied legacy migrations and the owner-epoch sequence remain intact. The separate
finite retirement command requires explicit database identity and operational
quiescence, disables the exact legacy definition and cancels only its nonterminal
jobs through native APIs. It preserves history and domain rows. Independent
readback can clarify a lost commit acknowledgement but never replaces or erases
the original command failure. Retirement is not run by service startup; its
fixture acceptance is not evidence of a deployment retirement.

## Dependency health sampling

`HealthMonitor` owns one native probe factory and creates no tasks. Constructing
it or its consuming `run` future is inert. Register `run` as an ordinary
supervised component; first poll acknowledges that the sampling loop is usable,
not that the dependency is healthy. Application startup approval and dependency
health remain separate. Returned probe errors and timeouts update health and
allow recovery; panics propagate to the critical task boundary and stop the writer.

`HealthPolicy` requires positive, representable probe budget, completion-to-next
probe delay, maximum age and scheduling margin. Checked arithmetic requires
maximum age >= delay + probe budget + margin. Probes never overlap. Each delay
starts after the previous attempt's completion and destruction; delayed polling
admits one next attempt, without an interval catch-up queue. The budget covers
the complete application future, including acquisition and query work. A result
observed at/past its deadline is classified as timed out. Blocking construction,
polling or destruction cannot be preempted by that deadline.

Readers synchronously copy one result/timestamp and writer liveness under a short
private mutex. They perform no probes, create no tasks or per-reader queues, and
never renew an observation. No application formatting or replaced-error destructor
runs under the publication mutex. Only the latest probe is retained; a fresh
in-budget failure retains its concrete E behind Arc without requiring E: Clone.
Debug omits E's contents. Trusted inspection and normal ownership/destruction of
retained application values remain application responsibilities.

Unknown, failed, timed-out, stale and stopped states are all unready. At exactly
maximum age the last result is stale, even if the owner still exists or has
stalled. A later successful probe restores healthy status. Dropping the sole
writer invalidates future reads immediately; reader clones and saved snapshots
do not keep it alive. A snapshot is historical as of `observed_at`, so readiness
must obtain a fresh read rather than caching a healthy snapshot forever.

On observed drain or forced cancellation the monitor admits no new probe and
destroys active directly owned work before returning. A completion followed by
drain during destruction cannot publish success. Unpolled abandonment and outer
run-task abortion stop the writer too. Retained last-probe details after Stopped
are diagnostic history, not evidence that the dependency remains healthy or that
remote work stopped. No detached-child, remote-query cancellation, async Drop,
runtime-death or non-yielding shutdown guarantee is added.

The HTTP example combines process Ready with a fresh health read at `/ready`.
Observing lifecycle drain overrides cached success; this is not an atomic joint
snapshot with concurrent lifecycle transitions. A request racing a later drain
can still use its earlier Ready observation, as with existing admission.
Dependency observations do not mutate process state or automatically alter
`RequestPolicy` business-route admission. The example's probe is an explicit
simulated dependency read, not evidence of actual database availability.

## Cleanup contract

The coordinator receives an owned task summary only after releasing its task
collection. The private join operation records each consumed result before
returning; abandoning a pending join waiter neither cancels owned work nor
discards its later result. Summary extraction retains unjoined names and does
not promote asynchronous abortion into proof of termination.

`CleanupReport` is `#[must_use]`. Awaited completion can still contain failed,
skipped or unjoined finalizers; callers must inspect the retained report.

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

## PostgreSQL client disposition

The optional `batter-sqlx` package owns only checked-out client disposition.
`pool_in` synchronously consumes a prevalidated cleanup slot, constructs a native
lazy pool from the supplied options, and registers awaited `Pool::close` before
returning the pool. It does not establish connectivity or readiness. SQLx may
start native minimum-connection maintenance during construction; constructor
panics and runtime death remain native limits. Dependent work must be joined and
checked-out connections released before successful cleanup can be expected.
Local completion evidence requires the named successful cleanup record, zero
remaining pool size and `PoolClosed` from a later acquisition; `is_closed()` alone
only witnesses that close began. These facts still say nothing about detached
server sessions.
`register_pool_close` remains a lower-level compatibility path for pools acquired
elsewhere, with caller-owned cleanup if registration fails.

`PgLease` detaches and drops its client unless the application explicitly calls
`return_to_pool` after acknowledged query/commit/rollback completion. Keep the
lease inside the future whose interruption should retire it. Panics propagate;
Rust's default panic hook can still print payloads. No native error contents are
added to adapter diagnostics, but trusted source inspection and upstream logging
remain application-owned.

Retirement releases local pool capacity without awaiting interrupted SQL or
SQLx's pool-return ping. It does not prove remote cancellation, rollback or server
session termination. Detached sessions may outnumber max_connections and survive
completed Pool::close. The live regression separately proves local replacement
and close while locks remain held, then session disappearance after unlock.
Repeated interruptions record independent residual sessions, not a remote bound.
Ordinary successful return retains SQLx's asynchronous health-check policy.
Neither operation interruption nor native failure classification authorizes replay.

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
not infer severity. Status-only probes do not add overrides. A future destroyed
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

The two HTTP/1.1 lifetime targets share launch/completion evidence but retain
separate raw-wire and instrumented-IO fixtures ([ADR-008](adr/008-http-transport-ownership.md),
[ADR-009](adr/009-http-lifetime-observations.md)). They independently observe
handler/upload destruction, response observation, complete message framing, body
destruction, client EOF, native socket destruction, direct server outcomes and
cleanup. Incomplete upload reads inside a guarded handler consume the response
budget. A blocked response body remains outside it after the escaped request
context is cancelled. Ordinary drain lets admitted handlers finish; a later
keep-alive request may receive 503 or encounter connection closure. A separate
withheld-graceful case must witness admission rejection.

Both suites require native connection graceful acknowledgement before releasing
cooperative work, then retain live resources and absent server completion/cleanup
at a 50 ms held-work checkpoint. The instrumented suite additionally requires a
pending wire read. This ordering depends on resolved Axum 0.8.9 and the enforced
current-thread runtime ([testing](testing.md)); the producer signal alone is
insufficient. These are bounded observations, not indefinite survival guarantees.

Forced cancellation can drop a handler, return 503 and allow clean direct-server
joining and cleanup. Aborting the direct wrapper instead leaves a blocked body
pending through report inspection and skips cleanup for the unsafe exit. Later
body release establishes separate transmission/destruction, not a transitive join.
Full client shutdown plus socket close drops pending handler/body work in these
fixtures before test release: both suites bound their post-close resource
observations by one second. Write-half-close is a separate native-default
case. No immediate or universal disconnect propagation follows. Before response
construction the observer reports dropped without status; after headers it keeps
the original completion with no second status/event.

Both targets require scenario-specific successful completion from a PID-authorized
child; ordinary successful process exit is insufficient. Independent Unix bounds
reject deliberate stalls. Failed event assertions retain destructor evidence.
Their shared startup/exercise/teardown phase budgets fit below the parent watchdog.
Terminal report waits belong to teardown; only the intentional blocked-body abort
checkpoint observes a report during exercise. Delayed real finalizers prove that
successful teardown can outlast exercise's allowance without failing that phase.
The observation driver joins timed-out exercise work before teardown and retains
any observed shutdown report independently of later reconciliation failures.
Slow combined-phase and readiness-timeout controls test these diagnostic paths.
These are test-infrastructure contracts, not runtime-death or general async-drop
promises. No HTTP/2, WebSocket, capacity or detached-descendant joining claim follows.
Current Linux evidence and unexecuted macOS/hosted scope are in [validation](validation.md).

Default infrastructure responses omit raw errors and use application/problem+json
with stable codes. `with_failure_renderer` can select an application envelope,
status, and headers using a request-parts snapshot taken before the handler runs.
The callback owns output sanitization and must use trusted metadata explicitly;
Batter does not trust a client correlation header for it. The callback covers
middleware failures, not application responses or separate health probes.
Default server execution exhaustion uses 503, not a claim that client
upload timed out. No automatic Retry-After authorizes write replay. Authentication,
authorization, proxy trust, body limits, and domain error policy remain
application-owned. Server correlation is separately opt-in. Liveness/readiness must be mounted outside the readiness gate.

HTTP fixture diagnostic deadlines belong to the startup, exercise and teardown
owners. Observation event/wire waits retain interrupted names, partial bytes and
event snapshots on destruction; the driver retains tracing capture and any
observed report. Fast and delayed-report negative controls require the same
teardown timeout result. Keep-alive handler counts are rechecked after terminal
shutdown, with late-entry controls proving the final checkpoint. These are test
harness contracts, not production deadline or async-drop guarantees.


### Opt-in operational defaults

`operational_http` generates a new UUID on first poll and composes exactly one
HTTP observer inside its retained request span. It replaces all x-request-id
values and both Tower RequestId and adapter CorrelationId extensions before the
inner service runs. The response header is replaced after the service returns.
`CorrelationId` is opaque, cloneable and exposes a string for explicit metadata
propagation; it never selects authority. Inbound trace and proxy trust remain
uninterpreted. UUIDs are correlation values, not a durable uniqueness constraint.

The observer retains this typed ID as a completion-event field independently of
INFO spans, including WARN dropped observations under a different ambient
dispatch. Native nested operation tracing is still filtered normally and carries
context through enabled request spans. The INFO request span uses target
`batter::request`: `info,batter=warn,batter::request=info` retains its context
without enabling Batter INFO operation/HTTP completions. A span-name filter
can also enable nested events and is not equivalent to this static target policy.
Neither library installs a global
subscriber. Tests retain their subscriber registries across cases and check
completion fields after the event message, not only in formatted spans.

`with_infrastructure_json` opts into a fixed application/json envelope containing
exactly `code`, `message`, and `request_id`, with Cache-Control: no-store and the
HttpFailure status. Codes and messages are the sanitized literals in
`render_infrastructure_failure`; request_id is a generated string, or null when
the typed extension is absent. No inbound header fills that gap. The renderer
never formats causes. Legacy Problem JSON is unchanged; explicitly selecting a
custom renderer afterward replaces this policy. Domain responses remain owned
by their handler.

`ReadinessPolicy` reads a fresh dependency snapshot then lifecycle readiness,
without invoking a probe or retaining the writer. Ready requires both healthy
and lifecycle Ready; an observed drain overrides cached health. A subsequent
transition may immediately obsolete the decision. Responses have empty bodies,
200 for Ready and 503 otherwise, and retain `ReadinessReason` in extensions.
Starting/Draining default INFO; Stopped and dependency Unknown/Failed/TimedOut/
Stale/Stopped default WARN. Explicit level policy alters neither status, reason,
body nor outcome. During supervised drain, a stopped health writer still yields
Draining/INFO; after process completion it yields Stopped/WARN. Reading either
state creates no probes. ReadinessReason is intentionally exhaustive: new states
require an API compatibility decision and consumer policy review. Old `readiness` and `liveness` keep their status-only contracts.

`register_http` transfers a bound TcpListener and initialized Router into a
critical component. The factory does no work before supervision starts and
acknowledges on its task's first poll; application approval and a running driver
remain necessary. Invalid or duplicate registration releases only the rejected
listener. Cancelling a borrowed StartingSupervisor waiter leaves the listener
owned. Dropping the startup owner requests drain, releases registered listener
captures before dependent cleanup, and lets observers await the completed report
on a live runtime. Dropping an unstarted Supervisor releases captures without
running finalizers.
Binding and its errors belong to owned Startup. Axum retries native accept errors
and spawns its own connection and graceful-signal tasks. Native graceful return
waits for connection completion; wrapper abortion does not join descendants.
A real streaming regression holds a body beyond the request budget and through
wrapper abort: the report is unsuccessful and dependent cleanup is skipped even
though every direct task was joined. The test separately releases the body.
There is no new async-drop, response-stream, WebSocket or disconnect guarantee.
The helper accepts a plain Router and supplies no ConnectInfo extension. Native
`into_make_service_with_connect_info` belongs in an application-owned supervised
serve closure; the helper rustdoc includes that composition.

## Optional database fixture finish

`batter-sqlx/test-support` checks declared connection capacity before acquisition.
`FixtureSuite::start` retains native lease/template producers before their waiters
can be cancelled. Acquired resources register before delivery and native pools
register before initialization. Body exit is followed by joining all producers,
closing each database's pools, awaiting lease cleanup, then draining deferred
cleanup. Reports retain body, acquisition, database cleanup and drain results.
Owned reports and the borrowed `FixtureReportRef` returned by `wait` warn on
accidental discard. A successful driver join does not imply report success;
inspect it or use `into_result`. Explicit discard can still lose observation.
A failed producer keeps the report unsuccessful even if its waiter was abandoned
or the body handled the error. Template initializer panic reaches upstream's
awaited abort path with its native join cause retained.

The borrowed scope cannot escape into a detached task. A run rejects batches
exceeding its own simultaneous-lease limit; other harness owners must release
shared native capacity for waiting acquisitions to progress. Run completion does
not shut down a shared server. Caller-owned shutdown follows all runs and leases;
shared deferred drain covers prior accepted submissions, including other owners'.

Cancelling `FixtureRun::wait` permits resumed observation. Dropping the waiter
loses the report but detaches the driver. Application operations/checkouts and
initializer-created pools still require explicit join/release/close. Detached
backend termination, runtime death and internal driver failure have no completion
guarantee. Low-level `DatabaseFixture` is must-use; its finish remains caller-owned
and cancellation can trigger destructive lease Drop. Low-level template waiter
loss can leave initializing templates outside the deferred queue.
The manual template path also propagates initializer panic without the owned
path's abort wrapper. Cleanup with initializer-owned live connections and forced
producer cancellation is unverified; neither is a supported completion claim.

`PoolAcquire` reports pending driver cleanup; low-level `Connect` reports the
outcome after explicit cleanup. Live probes cover cancelled native creation,
abandoned initializer error/panic, shared admission, partial/sibling acquisition,
body panic, simultaneous errors, close ordering and independent catalog absence.
`batter-kjl` additionally retains handled pool-acquisition errors in each database
report; the delivered `PoolAcquire` and retained `pool_failures` share the same
native cause through Arc. SQLx's internal after-connect retries may expose only
the final native acquisition error; this does not recover errors SQLx itself hides.

`FixtureRun::wait_for` bounds observation only: `Ok(None)` means pending. A cancelled
wait returns no result and can be resumed; a cleanup-driver JoinError is separate.
`cleanup_progress` exposes historical database phases and observation errors.
With `with_session_observer`, cleanup closes every tracked pool, then queries all
sessions for the database through an independent admin pool. The query checks
that the database exists and the observer is outside it. The caller must select
the same server and stop all producers of new connections. This is a point-in-time
absence witness, not fencing, remote cancellation or permission to reuse a database.
Matching database names do not validate cluster identity. A differently targeted
server with the same name can satisfy the query; normal native catalog visibility
and actual same-server targeting are caller preconditions. The observation does
not clear prepared transactions, active logical replication slots or subscriptions;
native cleanup may still fail after session absence is observed.
The live SCRAM startup control shows why producers must stop: an authenticating
backend can have NULL database identity and pass this filter. Native lease
cleanup can then remain pending on PostgreSQL's process barrier until that
connection closes. A separate live autovacuum control proves that an assigned
background worker is included and can require explicit retry after timeout.
The startup control identifies the actual target DROP backend's ProcSignalBarrier
wait while its sole new startup socket stays open, then closes that socket and
requires completion. Its identity window requires the dedicated serial test server.

A bounded session observation that times out or returns an error retains the lease
and parks only that database until an explicit `SessionObserver::retry_with`
request is available. Requests broadcast; one made during an active attempt is
consumed after failure, and unread requests coalesce. Independent
cleanup continues. Explicit retries coalesce while an attempt is active; they
never interrupt active work. Each failed attempt remains in the final report,
which stays unsuccessful even after eventual cleanup succeeds. The driver retains
the retry channel: losing all external controls can leave it parked indefinitely.
There is no implicit destructive fallback while the driver remains live.

Native detach and adapter retirement tests hold acknowledged blocked backends
past successful pool close, observe their identities and retained database from a
separate one-slot admin pool, then release them and resume cleanup. Observer
PoolClosed and wrong-target failures retain identical causes across progress and
report. Separate native resources prove body, consuming-cleanup and deferred-drain
errors coexist; external harness shutdown does not replace drain. Default native
panic hooks and upstream harness diagnostics remain outside adapter redaction.

Report summaries count failed consuming database cleanups separately from retained
pool failures and observation attempts. Successful cleanup with a recovered error
still makes the report unsuccessful, while database_failures remains zero. Replacing
an incorrectly targeted observer does not close its sessions: callers must explicitly
close their native pool clone and witness backend exit before the corrected retry.

A live-runtime waiter-loss test proves driver cleanup continues after release of
a held checkout. A separate actual runtime-destruction test observes a cancelled
driver JoinError and native lease Drop deleting the database while a checkout is
still held. That is destructive fallback, not completed pool closure or a fixture
cleanup guarantee. The test drains upstream cleanup from a surviving harness on
another runtime; process death is not covered.

Reference fixture completion uses one bounded consumer helper. A pending error
retains the waiter, diagnostic pool, all session pools and retry control together for typed recovery;
its automatic formatting exposes phases/counts only. It never treats timeout as
permission to drop the database. Abandoning that error loses the recovery handle;
runtime destruction retains the native destructive Drop limitation above.

Specifically, an unrecovered pending result passed to the reference `assert_probe`
boundary panics and may destroy its per-test runtime, triggering that native
fallback. The helper's timeout itself does not delete a lease; subsequent test
runtime teardown may. This boundary does not guarantee database survival after
an unsuccessful test returns or panics.

The reference completion bound also covers administrative pool closure. Driver
failure closes session and diagnostic pools before propagating the original
JoinError. If a checkout prevents closure, the pending owner retains that cached
error and both capacities; resuming after release completes their close first.
A successful join leaves diagnostics open for the caller's catalog checks. Pending
formatting includes separate redacted driver-joined and driver-failure flags,
so successful driver completion with pending admin closure remains distinguishable.


## Explicit settings and redacted diagnostics

`batter::settings` supplies std-based explicit inputs, a bounded literal reader,
unsigned decimal/duration bounds, `SecretString`, and static diagnostic projections.
It installs no source order, setting names, subscriber, environment mutation or
file discovery. Applications validate source structure before overlaying inputs,
then parse winning values. Missing, empty and non-Unicode remain distinct;
invalid winners never select defaults. Duplicate pairs fail before merging.

`read_file` opens only its supplied path. `read_literal` reads at most the checked
byte limit plus one and requires UTF-8. The dialect supports LF/CRLF, blank lines,
full-line comments, ASCII identifier keys, outer space/tab trimming, and literal
unquoted or wholly single/double-quoted values. Unquoted whitespace, quotes and
`#` are rejected. Quoted interior bytes are literal; dollar and backslash have
no interpolation or escaping semantics. Export statements, multiline values and
inline comments are unsupported. Source errors identify static categories and
optional line numbers, never input keys, paths or values.

`SecretString`, `SettingsSource`, `SettingsError` and `RedactedError<E>` hide values
in ordinary, nested and alternate Debug/Display. Secret access and error-source
traversal are explicit trusted operations. The original native causes remain
available; arbitrary source-chain reporters can expose them. No encryption,
zeroization, process-wide logging policy or panic-hook redaction follows.

The reference root owns defaults, precedence and serve/setup password policy.
Its private validated fields feed native request, pool, connection, Bulkhead,
finite-process and Runledger constructors. Its supported TCP URL subset requires
explicit host, username, database and sslmode; only password, sslmode and
application_name query settings are accepted, once each. Credentials are decoded
once, invalid percent/UTF-8 encodings fail, and unknown query settings are rejected
before any SQLx URL parser can warn. Serve requires a nonempty password and worker
identity; Setup permits passwordless externally provisioned endpoints and no
worker identity, but cannot manufacture a worker in that state.

SQLx 0.9 lacks an environment-free default constructor. The reference rejects
all captured PG* entries, even empty/non-Unicode ones, and rechecks the actual
process environment before `new_without_pgpass` and explicit setters. The process
must not mutate its environment during construction. No passfile is loaded.
Returned native options and their Debug/URL conversion are trusted exposures;
upstream runtime diagnostics remain outside this projection. The selected graph
has no SQLx TLS backend: requested TLS modes are retained, not downgraded, but
TLS connections require the application to enable a native TLS feature. TLS
negotiation is unverified. SQLx's native URL formatter cannot round-trip a bare
setter-provided IPv6 host; the reference passes the bare host required by Tokio
and separately tests its native startup/password wire exchange.

Focused offline tests prove source policy, native field mapping, changed HTTP
response deadlines, independent admission capacities and retained startup/cleanup
failures. Explicit live cases additionally require a disposable PostgreSQL 18
endpoint; their execution status is recorded in [validation](validation.md).
The unpublished reference package now composes these constructors into a staged
command and probe-only worker host. No delivery-provider execution, publication,
deployment or external adoption follows from that example.

The private live endpoint handoff accepts `localhost` or `127.0.0.1` with
sslmode=disable. It rejects IPv6 literals before acquisition because SQLx 0.9
retains their URL brackets during TCP lookup. Direct root construction still
supports bare native IPv6 options. The handoff serializes the validated host and
database, canonical TLS mode and explicit password choice, including empty
passwords; fixture URL parsing must not select an ambient passfile. Query spaces
retain their meaning in both native clients. Parser regressions model the
harness's normalization and database-path replacement; they do not establish
live authentication or TLS.

## Offline legacy retirement

The reference's finite retirement command verifies expected cluster/database
identity on one direct PostgreSQL session, rejects replacement of that session,
and requires no other target-database client or unknown backend and no prepared
transaction. These checks precede native definition disable and repeat before
completion. It cancels only global nonterminal legacy controls through native
Runledger policy. Applied migrations, sequence state and terminal/domain history
remain. An absent definition is not a durable disabled tombstone.

Deployment tooling must stop old producers and prevent restart before maintenance;
the database report cannot establish that external fact. Native definition disable
blocks the inspected enqueue path and is preserved by the inspected additive
catalog sync, not arbitrary SQL or administrative re-enabling. Pool closure alone
does not prove backend death. Connection loss, deadline expiry or cancellation
never proves absence of remote effects or permits automatic replay.

A returned cancellation failure is published with the owned command report before
optional readback. The separate read-only command cannot overwrite that report;
its cancellation leaves the primary cause inspectable. Scoped native cancellation
retains SQLx begin/commit sources and a separately returned rollback failure.
Reports cover returned outcomes, not every intermediate value inside a cancelled
native future. Real commit rejection and interruption during readback are covered
by PostgreSQL 18.6 probes; full redesign acceptance remains tracked separately.
