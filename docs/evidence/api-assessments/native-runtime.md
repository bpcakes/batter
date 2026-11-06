# API assessments: Native packages, invocation phases and consumer evaluations

[ADR-010 policy](../../adr/010-agent-only-consumption.md) · [Assessment index](README.md)

These records apply ADR-010 to specific changes. They retain the evidence
and limitations recorded at delivery; current contracts remain in rustdoc
and [guarantees](../../guarantees.md).

## Native package reachability assessment (`batter-0jsf`)

The facade exposed `batter::runledger` and `batter::runlimit` but not the native
packages they translate. `batter-runledger` re-exported selected
`runledger_postgres` items and held `runledger-core` only as a dev-dependency, so
worker preparation, `JobsConfig`, `JobCatalog`, durable intents and the migrator
were unreachable; `batter-runlimit` re-exported `runlimit-core` and, behind
`postgres`, `runlimit-postgres`, but not `runlimit-memory`, `runlimit-http` or
`runlimit-axum`. Both surveyed consumers therefore declared the same workspace
revision two to eight times, and the cost was structural: every new capability
added another declaration to keep in step, and one consumer fell 139 commits
behind. Repeated declarations a consumer must keep consistent are the same signal
this ADR names for repeated coordination instructions.

Each native library package is now reachable through a feature-gated namespace
that is the native package itself, so facade and direct paths cannot diverge into
two identities. The invalid state this removes is a consumer holding two source
identities for one workspace; it is now unrepresentable through the documented
recipe because there is nothing left to declare separately.

The namespaces are deliberately weaker than the protected path, and the review
question is whether they make a known-invalid composition look equivalent to it.
They do not, and the naming carries that:

- `runledger::native::{core, postgres, runtime}` is reached through a segment
  named `native`, and its module documentation states the obligations a caller
  takes on by building a live native supervisor instead of handing inert
  preparation to `register_in`: observing initialization, requesting shutdown
  within a budget, classifying the settlement and ordering dependency cleanup.
  `register_in` still accepts only a `PreparedSupervisor`, and the existing
  compile-fail rustdocs still reject a live supervisor and a closure hiding one.
  Reachability adds no way to pass a live supervisor through the protected
  boundary.
- `runlimit::native_transport::{http, axum}` is named apart from
  `runlimit::http`, which remains the protected quota-before-body assembly. The
  native layer's documentation states that subject derivation, rejection mapping,
  status and body selection stay with the caller and that it acquires no operation
  deadline, admission ordering or telemetry from the foundation. Its features
  select neither `batter-axum` nor `runlimit::http`, so the two never appear as
  interchangeable spellings of one capability.
- `runledger::native::test_support` is documented as test-only and must not be
  selected in a deployed graph. This is application policy: Cargo cannot express
  "development graphs only" for a feature, and the facade cannot detect the
  caller's profile. The recipe requires resolver 2 or 3 for dev-dependency
  feature separation and an explicit resolver at a virtual workspace root;
  resolver 1 also enables these features in ordinary dependency builds.

What types cannot express here is deliberate. These are the native packages'
own APIs; Batter does not narrow them, and narrowing them would create the second
identity the task exists to remove. The remaining boundary is that a consumer can
reach a low-level native path where a protected one exists. That is the standing
escape-hatch position of this ADR, not a new weakening, and the documented
canonical path is unchanged. Fresh-agent usability evaluation of the recipe is
proposed and unexecuted.

## Runledger invocation phases assessment (`batter-5h5y`)

A read-only survey of two downstream consumers and the reference worker found
three copies converting a job execution into a fresh root operation: the
remaining work budget fed to `OperationOwner::new`, the execution deadline fed to
`RootDeadline`, or a fresh root sized from the remaining budget. Each repeated
the deadline and reserve arithmetic, restarted the budget at derivation, and
built a root whose cancellation nothing could trigger. The worker enforces
timeout and lease loss by dropping the handler future, so awaited work stopped,
but work that observed the context instead of being dropped with the handler was
never told the invocation had been abandoned. A context that silently never ends
was the invalid state; the repeated arithmetic was the caller obligation.

At this assessment's delivery, the canonical path was
`job_phases(execution, reserve)`. The later `batter-f9kq.2` assessment below
supersedes that public path with `PhasedJob`; derivation is now private. It takes the whole
`JobExecution`, so its deadline is the worker's own absolute instant rather than
a value the caller re-derives, and the reserve is a required argument, so no
default hides the final-state budget. It returns the existing `OperationPhases`:
work ends `reserve` before the native deadline and finalization keeps it, through
the one core computation, so the bridge adds no arithmetic. The value carries no
authority: `OperationPhases` cannot cancel both phases and
`OperationContext::cancel` stays private, so a handler can neither end its
invocation nor cancel the shared root, and children cancel only downward.

Linkage. Batter contexts cancel through a tokio-util `CancellationToken`,
`runledger-core` had no Tokio dependency, and a detached forwarding task was
excluded, so only two mechanisms could let native ownership hold the link. The
selected one is a native exit hook. `runledger-core` gains a std-only
`JobInvocationOwner` whose read-only, cloneable `JobInvocation` offers `ended()`,
`has_ended()` and `on_end(hook)`; the owner runs registered hooks once when it is
ended or dropped. The bridge moves the derived root's `OperationOwner` into one
hook, so its only cancellation authority lives in native-owned storage and runs
on every exit path, including destruction during task abort. Batter-core's
construction surface is unchanged: `OperationContext::{new, at, under}` stay
unreachable with their existing compile-fail controls, `OperationOwner::under`
stays crate-private, and tokio-util stays out of both public APIs. The rejected
alternative, a downward-only child `CancellationToken` from native plus a
capability-shaped core constructor, would have made tokio-util 0.7 part of the
public API and semver of both `runledger-core` and `batter-core`, and would have
needed a public core constructor linking a new root to any token a caller holds:
the unrelated-root construction the operation-authority assessment above removed.
Its one advantage, that native would run nothing but token cancellation, is
covered differently: the owner contains each panicking waiter wake or hook and
still notifies later hooks and observers, and the worker records every panic as
`RuntimeCallbackFailure::Panicked` evidence, keeping settlement conservative. The
remaining cost is that synchronous hook code runs in the native exit path.
`on_end` requires hooks to be brief, non-blocking and panic-free; containment does
not suppress the panic hook or catch aborting panics. Each derivation keeps one
hook until the invocation ends.

Waiter containment (`batter-5h5y.2`). A custom waker can panic before exit hooks
run. Marking the invocation ended does not itself cancel derived contexts, and
dropping a skipped hook drops the root authority without cancelling it. Rust's
`Waker` capability does not express panic freedom, so the native notification
boundary contains each wake independently and retains its panic before invoking
the remaining waiters and hooks. The capability API and waiter-before-hook order
stay intact; no caller coordination or new public type is needed. The existing
`JobInvocationHookPanics` report and runtime callback category retain both wake
and hook failures. Primitive end/drop regressions, a runtime diagnostic regression
and acknowledged derived-context observers cover this failure mechanism.

A subsequent actual-source probe exposed the coupled disposal phase: an opaque
`panic_any` payload can panic when dropped, after the notification hook ran but
before later diagnostics were retained. This is library-owned error disposal,
not an application replay decision. The report now owns that boundary and offers
borrowed `payloads()` for diagnostics; the worker no longer extracts raw boxes.
Following native shutdown containment, report drop releases known string payloads
and intentionally retains opaque allocations. Catching and then dropping each
secondary panic was rejected because payload destruction can panic recursively.
Raw `into_payloads()` remains source-compatible as a documented low-level escape
hatch with caller-owned disposal; it is not equivalent to borrowed diagnostics.
Tests exercise opaque payloads with panicking destructors on owner/report drop,
later native failure records and actual derived cancellation. This bounded remedy
keeps disposal in library ownership without a new caller protocol; retained
opaque allocations are an explicit cost, not proof of resource cleanup.

The same foreign-callback boundary applies while observers are active. An
actual-source probe and bounded regressions reproduced a deadlock when replacing
or removing a stored waker: its last-reference destructor read `has_ended` while
the invocation mutex was still held. The native implementation now clones before
locking and moves replaced or removed wakers out before destroying them. This
is library-owned locking, not application policy; the existing API needs no
caller coordination. Regressions cover both paths, subsequent owner end and hook
progress, while existing tests retain notification order and waiter counts.

Zero reserve. `reserve_finalization` rejects `Duration::ZERO` through the shared
positive validator, while the bridge must read zero as no reserve. The selected
resolution is a core extension: `OperationContext::split_finalization` returns
the same sibling `OperationPhases` with no interval reserved and shares one
private work-deadline computation with `reserve_finalization`, whose existing
rejections and tests are unchanged. The rejected alternative, a bridge output
whose zero form is a single context, would have given one choice two output
shapes and either removed `cancel_work` or let the zero form's `cancel_work` also
cancel finalization: a known-invalid composition selected by a reserve value.
With the extension every reserve yields the same sibling semantics; zero merely
leaves finalization no time after work expires, which the bridge rustdoc states
together with the worker's `job.timeout_exceeded` precedence.

Ownership and drop order. The worker owns one signal per invocation in
`worker/invocation.rs`. Its owner is a local that outlives the handler future:
every returned path destroys the handler future, ends the invocation, and only
then returns to outcome persistence; task abort and task-set destruction drop the
owner after the handler future. A graceful stop request touches neither, so drain
keeps admitted invocations running until native escalation abandons them.
Continuations and retries get new owners. Custom runtimes keep compiling because
`JobExecutionServices::invocation` is a provided method returning `None`; the
bridge refuses such services with `JobPhasesRejection::Unsupported` rather than
returning phases that could never be cancelled. Opting in means owning one
`JobInvocationOwner` per invocation, as the native changelog shows.

Against the review questions: derivation rejects unsupported, ended and
exhausted executions and invalid reserves before returning phases, and operation
preflight rejects later expiry or cancellation before any factory runs; no
ordering, paired call or guard remains with the caller, because native ownership
establishes and releases the link; authority is split into a non-cloneable native
owner and read-only observations, contexts and phases; the bridge is the only
ordinary path, used by the reference worker and the facade-only consumer. What
remains is application policy (the reserve size, the rejection-to-`JobFailure`
mapping and replay classification), the custom-runtime obligation to end one
owner per invocation, and unverifiable remote effects. A generic
`OperationError`-to-`JobFailure` conversion was deliberately not added:
`JobFailure` keeps only kind, code and message, and native policy treats timeout
and lease expiry as retryable, so a universal conversion would erase concrete
causes and choose replay policy for the application.

Compile-fail controls reject ending an invocation through its observation,
cloning its owner, carrying `JobExecution` into a thread, cancelling both phases,
and cancelling finalization through its context. PostgreSQL-backed worker tests
end the signal on success, continuation, returned failure, panic, timeout,
swallowed progress lease loss, heartbeat lease loss, lease-maintenance failure,
direct task abort and worker-loop destruction, with acknowledged, multiple and
late observers and an outcome gate proving the end precedes persistence; drain
keeps the invocation running. Paused-time bridge tests cover the arithmetic,
rejections, phase semantics, observers, isolation and typed forwarding. The
reference worker's live state probe drives provider-work expiry through its
production `execute` path and requires the retained state to commit inside the
native deadline, and the facade-only consumer requires each invocation's exit to
cancel its derived work phase.

Fresh-agent usability evaluation was executed on 2026-10-09 with two
independent agents that had no context from this implementation and could read,
but not change, the repository. Each worked in a scratch crate outside the
repository that declared only `batter` (feature `runledger`), Tokio and
serde_json. The implementation task asked for a provider call bounded by the
job's own deadline, a 300 ms recording reserve and cancellation on abandonment,
without naming a Batter API. The agent found and used `job_phases`, mapped every
`JobPhasesRejection` explicitly, ran the call under `work()` and the record
under `finalization()`, rejected a fresh root from the remaining budget, and
passed build, Clippy and five unit tests driven through services built on
`JobInvocationOwner`. The modification task added a lookup with its own
two-second limit and raised the reserve to one second. The second agent used
`phases.work().child(limit)` for the lookup, kept the send under `work()`, and
passed build, Clippy and ten tests; temporary mutations confirmed that those
tests catch a send under the lookup's child, a child of finalization, a missing
limit, an unlinked root and a shortened reserve. Gaps they reported were fixed in
the rustdoc and guides before closure: no facade-path handler example, exit
ordering and spawned-work reach missing from the bridge rustdoc, an example that
collapsed provider outcomes, no guidance for nested limits or against a new root
inside a handler, and terse `child` documentation. Two remain outside this task.
Consumers write their own test execution services, and constructing a
`JobContext` needs a UUID; both are closed by the handler test facility
assessment (`batter-3upk`) below. A new independent root is still constructible inside
a handler, because `OperationOwner::new` and `RootDeadline` remain the general
independent-root API; the guides direct nested limits to `child`, and only an
exit test detects the unlinked root. These are two executed evaluations against
in-progress documentation, not a benchmark, and their scratch artifacts were not
committed.

## Whole-application baseline (`batter-k64t`)

Executed on 2026-10-09; the record is `docs/evidence/whole-app-baseline-2026-10-09`.
One general-purpose agent with no session context built a complete service from
a Git-free copy of master (`71e37ea`) under a reading allowlist of public
documentation only, in a scratch crate that declares only `batter` with the
`axum` and `runledger` features plus registry crates: typed settings, an owned
pool, schema setup, a POST route that inserts a row and records a durable job
intent in one transaction, a GET route, probes, a `records.notify` handler with
a bounded provider step and a final-state write, and signal-driven shutdown.
Against an oracle frozen before launch, all eleven decision points were the
canonical path or the documented alternative; none was wrong. The first build
failed once, on a missing `PgScopeError` re-export and a by-value
`CorrelationId`; `clippy -D warnings` needed three further rounds for test-code
repairs and a production `result_large_err` allowance on `RecordsStore::create`
that retains the complete atomic error. The crate passes build, clippy, fmt and
26 tests, and the archived copy builds against this tree. No live PostgreSQL run
was made.

Separate from the eleven scored decisions, the agent reported eleven gaps:
six documentation repairs, one missing re-export, two capability follow-ups,
and two policy or default questions. The re-export and documentation repairs
shipped in the same change. The capability follow-ups are one profiled pool
owner shared with Runledger (`batter-nhu5`) and a handler test facility
(`batter-3upk`, closed on 2026-10-24 by the assessment below). Shutdown budget
sizing remains application policy; the consumer
default between `PgSessionProfile::with_timeouts` and `new` remains unresolved
and needs a maintainer decision. The evidence README records each disposition.
This is one run of one agent and does not establish population reliability.

## Handler test facility assessment (`batter-3upk`)

The whole-application baseline's seventh gap and the residual recorded under
the invocation phases assessment were the same obligation: the only in-process
way to run a `JobExecutionHandler` was `JobExecution::new(&context, &services)`
over hand-written `JobExecutionServices` owning a `JobInvocationOwner`, a
recipe documented only on `JobExecutionServices::invocation`. The reference
worker's tests, the facade-only consumer's tests and the baseline agent's crate
each carry a copy. Each copy restates the deadline, keeps the owner behind a
mutex so the test can end it, must end it after the handler returns and before
asserting on derived contexts, must keep the deadline the services report equal
to the one the test enforces, must handle the `#[must_use]` hook-panic report,
and must mint a UUID for a `JobContext`. The rustdoc example itself never
ended its owner, and the baseline agent's copy dropped the `#[must_use]` report
until Clippy objected. Under the review questions this is caller memory of
ordering, a paired completion and a deadline relationship (question 2), and a
hand-written runtime in every test crate where one consuming library-owned
execution could stand (questions 3 and 4).

The canonical path is `batter_runledger::test_support::HandlerInvocation`,
reachable as `batter::runledger::test_support` with the facade's
`runledger-test-support` feature, so a consumer's development graph declares
only `batter`. `new(budget)` declares a fresh job, and
`for_context(budget, context)` the claimed job a handler reads: its
organization, a later run or attempt, or the lease identity that fences the
handler's own writes. Both accept the budget through the same
`RootDeadline::after` acceptance as every Batter root budget, so zero and
over-long budgets are rejected before a handler exists, and `for_context`
rejects a non-positive run number or attempt, which the queue row's own checks
exclude for a claimed job. `with_checkpoint` and `with_progress` are the
optional inputs, the resume state the worker would supply: the checkpoint the
handler reads, and the `progress_done` and `progress_total` that a retry, or a
continuation preserving its state, keeps on the row and that the worker merges
and validates every write against. `with_progress` refuses a pair the row's own
constraint refuses, with the native `JobProgressValidationError`. Because the
context is taken at construction, each replaces only its own fields, in either
order: a builder that replaced the whole context would have let a later call
silently discard an earlier checkpoint. `execute(&handler, payload)` consumes the declaration and owns the
whole drive: it creates one `JobInvocationOwner`, builds services whose
`deadline()`, `remaining_budget()`, progress writes and the enforcing timeout
all read one instant set when the future is first polled, dispatches through
`JobHandler::execute_with_services` exactly as the worker does, so an adapted
handler is never asked to run without services and typed and legacy handlers
run unchanged, contains a handler panic at the poll boundary, including one a
hand-written dispatch raises before returning its future, because the dispatch
call runs inside that boundary as it does in the worker, releases every caught
payload as `JobInvocationHookPanics` releases notification payloads, retaining
an opaque `panic_any` payload instead of running a destructor that could itself
panic, destroys the
handler future and only then ends the owner, returning its hook-panic report
instead of dropping it. Its locals are declared in the worker's order, so
destroying the facility's own future, the test's equivalent of a task abort,
drops the handler future before the owner ends. Progress persistence is a row
in memory, starting from the declared checkpoint and progress, with the queue
row's `COALESCE` rule, validated on the effective pair and refused at the
deadline, as `LeaseExecutionServices` refuses it.

The outcome is an exhaustive `HandlerExit`: `Returned` carries the handler's
own `Result<JobCompletion, JobFailure>` when it was observed strictly before
the deadline; `TimedOut` is any observation at or after it, including a result
from a handler that crossed the deadline without yielding, applying the
acceptance rule `JobExecution::deadline` documents rather than Tokio's poll
order; `Panicked` carries the message. As in the worker, nothing preempts a
handler that never yields: the drive completes only once the handler next
returns or yields. Returning the
worker's `job.timeout_exceeded` and `job.handler_panic` failures instead was
rejected: it would restate the worker's classification strings in Batter and
let `is_err()` accept a late or panicking handler as a returned failure. The
`HandlerRun` is `#[must_use]`, has no public constructor, and carries the
persisted row and the hook-panic report beside the exit. Nothing a handler or
test reaches can end the invocation, because the owner is never exposed, and a
declaration drives exactly one invocation, because `execute` consumes it.

Against the review questions: the validated inputs are rejected at
declaration, and the payload, the checkpoint and the rest of a declared context
are values the worker also passes through from the row unvalidated; whether a
declared identity names a leased row is the handler's own lease fencing, as it
is under the worker; no ordering, paired end, deadline relationship or panic-report
handling remains with the test, and exhaustive matching replaces a `Result`
whose error could stand for three different exits; authority is a consumed
declaration and an unreachable owner; library-owned execution is the only
ordinary path, while the custom-services recipe remains the documented
low-level path for a custom runtime and is not equivalent, since it leaves the
end obligation with its author. What remains is application policy or upstream
behaviour, kept explicit: the handler's own `JobFailure` mapping; the worker's
durable outcome policy, which the facility does not simulate, so retries,
continuations and dead letters are asserted on the returned completion or
failure rather than observed; lease loss, which is not representable because
there is no lease; and remote effects, which a returned exit proves nothing
about. The facility is adapter-owned rather than a native package change: its
contract is the Batter test path for a Batter-bridged handler, and reachability
authorizes no native runtime change. It sits behind the adapter's existing
`test-support` feature, which already carries the native PostgreSQL test
graph; that cost is recorded, not removed.

The two residuals recorded at first delivery are closed. A later run or
attempt, an organization and a leased identity are declared through
`for_context` instead of custom services, and both consumers that carried a
copy of the recipe now use the facility. The facade-only consumer's job tests
drive its registered handler, and run single steps on the invocation's own
clock through a handler that derives the same phases. The reference worker's
handler, admission and state probes each run inside an invocation of the
owner-scoped or leased context they need. Two custom-services values remain
there as negative controls for states neither the worker nor the facility
builds: services without an exit claim must still be refused, and an invocation
that ended before derivation maps to the same budget exhaustion as no remaining
work time. Paused-time tests cover
success, retryable and terminal failures,
phase rejection without a work factory, destruction before work with a spawned
observer, an elapsed budget, a parked handler future destroyed before the
invocation ends on both of those exits, a budget that starts at the first poll
after delays following declaration and construction, with `remaining_budget()`
reading the same deadline, acceptance at and one nanosecond before the
deadline for results and at it for panics, contained panics including one a
hand-written dispatch raises before returning its future, opaque payloads with
panicking destructors retained before and at the deadline, retained hook
panics, progress merge and rejection, declared checkpoints, declared retained
progress bounding a resumed attempt's writes and refused at declaration where
the row refuses it, budget validation,
a claimed context reaching the handler unchanged with a later checkpoint
replacing only the checkpoint, non-positive run numbers and attempts rejected
at declaration, and typed, legacy and type-erased dispatch; the
`with_checkpoint` doctest shows a declared checkpoint retained when nothing is
committed, the `with_progress` doctest a retry refusing a write its retained
total forbids, and the `for_context` doctest an owner-scoped handler accepting its
claimed context and refusing a fresh one; a compile-fail
rustdoc rejects reaching the declaration's fields;
`scripts/check_facade_features.py` proves facade reachability and identity.
The exit is not total in one recorded case: a destructor that panics while the
drive destroys the handler future is not contained, as it is not in the worker,
whose unwind boundary also covers polling only; a should-panic test pins this.
Fresh-agent usability evaluation is proposed and unexecuted.

## Borrowed handler work and finalization (`batter-f9kq.2`)

The frequency-ranked audit found that borrowing native execution services forced
feature handlers to sequence manual phases. An ordinary `?` after work could
skip finalization. The protected path is now `PhasedJob<PhasedJobHandler>`:
the native worker invokes one adapter future that owns work, complete outcome
capture and finalization. No public phase-running method or separate waiter is
exposed by that adapter. `JobWork` cannot access finalization; `JobFinalization`
is constructed only after work-future destruction. Both borrow the invocation's
existing services. Finalization cannot overwrite work evidence; completion needs
both phases to succeed, and `failed` can return only a native failure.

The foundation's command polling kernel supplies timing, panic and destruction
semantics through a deliberately lower-level host-owned driver. Its cancellation
obligation belongs to the adapter: use it only inside an invocation the host
already owns. It is not a cancellation-safe borrowing bracket for application
code. The protected adapter does not expose that driver. Native lease loss,
deadline or task destruction still revoke invocation authority; running a later
finalizer would not restore that authority. Native fencing and remote outcome
reconciliation remain native/application responsibilities. No spawned coordinator,
second worker engine, service proxy, or new native execution lifetime is added.

Regression scenarios exercise borrowed checkpoint calls, early errors, work
panic, work timeout with remaining reserve, simultaneous work/finalization errors,
finalization timeout, native invocation destruction and invalid reserve refusal.
The external single-dependency consumer registers the adapter and persists a
checkpoint from finalization. Compile-fail rustdocs reject forged scopes and
work-phase access to finalization. The public `job_phases` function is removed;
no shim exposes the phase pair. All consumers migrate, including the reference
worker, whose work returns a typed provider outcome for final-state persistence.
Private bridge and native harness tests remain internal regression tests.
Breaking migrations are in CHANGELOG. Fresh-agent usability evaluation
remains unexecuted. Execution results belong to the owning Bead.

The [public API comparison](../public-api-diff-2026-11-06/README.md) records
the executed surface diff and the no-shim migration decision for this cutover.

Panel repair assessment (`batter-f9kq.3`): the callback-await wrappers were an
implementation error at the adapter/kernel boundary. They destroyed a ready
callback before retaining its result. Direct future forwarding lets the existing
kernel retain both return and destruction evidence; regressions cover values and
errors in both phases through the core seam and the public native adapter.
`OperationPhases::validate_reserve` shares the existing core configuration rule
with adapter construction. It creates no execution capability; ignoring its
result cannot bypass the protected constructor or invocation-time checks.

The other two findings belong to the reference application's provider policy.
Its classifier must preserve native non-retryable panic semantics after both
phases settle. Its dispatch fence may commit without acknowledgement, so a
distinct `FenceInterrupted` action and the finalization reserve reload durable
state before a lease-fenced legal transition. These rules cannot prove a remote
rollback. Tests drive the production classifier for construction/poll/destruction
panics, including a retryable finalization error; live cases interrupt actual
pending fence SQL with both outer and nested deadline ordering. A separate live
case commits the real fence then withholds the work result to test retained
uncertainty; it does not emulate wire-level COMMIT acknowledgement loss.
The first repair also sent a concrete `RetryDeferred` through that interruption
fallback, changing retained state that the existing live reconciliation test
requires unchanged. The corrected policy preserves concrete returned fence
failures and reloads only interrupted/panicked work or `FenceInterrupted`.
Core already supplies this distinction in the retained outcome; moving provider
state policy into the library would put it in the wrong owner. Skipping all
fence recovery would retain the original race. This coupled-phase assessment
and its live failure evidence are recorded in the owning Bead.
These are implementation and application-protocol repairs, without adding a
native queue, exposing phase cancellation or restoring removed APIs. Fresh-agent
feature implementation/modification evaluation remains unexecuted. Executed
verification is recorded in `batter-f9kq.3`.
