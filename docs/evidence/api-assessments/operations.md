# API assessments: Operation authority, acquisition and blocking work

[ADR-010 policy](../../adr/010-agent-only-consumption.md) · [Assessment index](README.md)

These records apply ADR-010 to specific changes. They retain the evidence
and limitations recorded at delivery; current contracts remain in rustdoc
and [guarantees](../../guarantees.md).

## Tracked operation invocation assessment (`batter-2hqe`)

Consumers reconstructed a start marker because `run` projects both refusal and
post-invocation interruption into one error. `run_tracked` records invocation
inside the same library-owned boundary and returns exhaustive failure variants.
`NotStarted` has a private constructor and no conversion from an interruption;
only a boundary that did not invoke its factory creates that evidence. Callers
need no paired marker update. The result is a historical observation for this
call, not authority to replay or a witness tied to arbitrary later work. Copying
an existing witness does not establish anything about another invocation.
Started does not prove remote effects; earlier work and capture destructors are
outside the non-invocation guarantee. Applications retain remote reconciliation
and retry policy. No blanket `From` conversion silently discards the distinction.

`child_until` constructs only parent-clamped children and returns the existing
non-cloneable cancellation owner. Past instants create expired children, avoiding
consumer deadline-to-duration reconstruction or accidental independent roots.
The existing `run` remains supported for applications that need no invocation
certainty, preserving its precedence and error shape. Shared execution and
regression tests prevent the two paths from drifting. Tests include refusal,
started interruption, same-poll precedence, panic/drop and scoped destruction;
the reference provider adopts tracked dispatch without changing its remote
protocol. Fresh-agent evaluation is not executed by these tests.

## At-rest rewrap pairing assessment (`batter-t2uq`)

The earlier wrapper-only rotation returned a `WrappedKey` that the caller had to
attach to the input descriptor. Attaching it to another descriptor compiled and
failed authentication only when used. The canonical
`Keyring::rewrap_envelope` now authenticates once and returns the complete
`Envelope` with the input descriptor, so ordinary rotation has no separate
attachment step. A complete result cannot be passed directly as a `WrappedKey`
to another envelope's composer. The old wrapper-only signature and split-part
constructors remain for compatibility; their rustdoc marks them as lower-level
paths, and extraction can still reintroduce the mismatch. Cross-descriptor
authentication tests cover that deliberate escape hatch.

This local shape proves only the pairing returned by that call. Decoded and
reconstructed envelopes share the same type and may be unauthenticated; the
method does not read the body. Storage must retain header/body association,
compare-and-swap a complete prior header or covering revision, fence current
key policy, and enforce durable wrapper-encryption budgets. These remote and
application-owned effects cannot be proved by the returned Rust value. Fresh
agent implementation and modification evaluations remain proposed and
unexecuted for this API.

## Operation authority assessment (`batter-tc9w.1`)

A cloneable context previously let a consumer cancel the shared operation or
construct an unrelated root while processing a child callback. That made a
sibling or parent interruption possible through an ordinary execution argument.
The canonical path now passes cloneable `OperationContext` only for observation
and execution. Non-cloneable `OperationOwner` retains explicit cancellation;
child owners derive their deadline and token from the complete parent context.
`RootDeadline` preserves absolute time for an explicitly independent root, and
`OperationAdmission` returns an owner linked to process cancellation only after
readiness. Compile-fail rustdocs reject context cancellation, independent root
construction through a context, and owner cloning. Runtime regressions cover
child isolation, deadline clamping and process cancellation.

The application root still chooses when an independent operation is appropriate;
local types cannot prove that policy. An owner may deliberately give up its
authority with `into_context`, and dropping it does not cancel detached work.
The application must still await any work it owns and keep cleanup separate from
process cancellation. These are explicit ownership limits, not claims of async
drop or remote effect rollback. Fresh-agent usability evaluation is proposed
and unexecuted.

## Bounded blocking work assessment (`batter-0772`)

[ADR-014](../../adr/014-reuse-after-two-consumers.md#accepted-bounded-blocking-work)
found the same hand-written helper in all three surveyed consumers: acquire a
permit, move it into a `spawn_blocking` closure so capacity stays held until the
closure returns, then await the join. The canonical path offered only
`Bulkhead::enter`, whose rustdoc asked callers to hold the native permit "for
the work's full lifetime" and enforced nothing. Holding it in the awaiting
future compiles and is wrong: when the caller is cancelled or dropped while the
closure runs, the permit is released and later callers oversubscribe the
blocking pool. The negative control
`awaiter_held_permit_is_released_while_its_blocking_work_runs` reproduces that
release while the closure is still parked. Each copy also had to classify
`JoinError` itself and, to honour its deadline, check its context after
admission: `Wait` admission can return a permit after the deadline when its
timer has not fired yet, as the control
`enter_alone_can_grant_a_permit_after_an_unfired_deadline` shows. Under the
review questions this is caller memory of an ownership transfer, a paired join, a deadline relationship and exhaustive outcome
handling (question 2), where one library-owned execution can replace the helper
(questions 3 and 4).

The canonical path is `Bulkhead::run_blocking(context, admission, work)`,
reachable as `batter::admission`. It admits with `enter`'s policy and preflight,
checks the context again after a grant, records the one bulkhead decision after
that check and then, in the same poll with no await in between, moves the permit
into the closure it hands to Tokio's blocking pool. Neither the permit nor the
`JoinHandle` is exposed, and Batter never aborts the task: Tokio documents that
abort only "may" prevent an unstarted blocking task from starting, so an abort
would make the abandoned outcome ambiguous. The permit is a field of a private
closure owner, released only after the closure returns or unwinds and its
captures are destroyed, or after Tokio destroys an unstarted closure, and always
before the result is published, so `Ok` and `Panicked` arrive with capacity
already free. The context bounds only the wait, with `run`'s precedence. The
closure's value is returned unconverted, so a fallible closure's domain error
stays concrete inside `Ok(Err(error))`. Failures are the exhaustive
`BlockingError`: `NotAdmitted` (never handed to Tokio, no capacity used),
`NotStarted` (destroyed unstarted by a shutting-down runtime), `Abandoned` (the
wait ended; the closure may be queued, running or finished and keeps its permit)
and `Panicked` (payload retained, formatting redacted). It has no `From`
conversions, so `?` cannot turn an `Interruption` or `AdmissionError` into one of
these outcomes; two compile-fail rustdoc examples pin that. The four Display
texts are distinct, so a log line cannot conflate work that is still running with
work that never ran. The closure runs, and an unstarted closure is destroyed,
under the dispatcher captured at first poll, in a `batter.blocking_work` span
whose parent is the caller's span at handoff. A closure panic is caught inside
the closure owner, and the value or payload travels back through Tokio in a
private wrapper, so an abandoned one is destroyed unobserved under the same
dispatcher, outside that span, even when Tokio drops it on the blocking thread.
A capture's panic while Tokio destroys an unstarted closure is rethrown in the
same wrapper. A value or payload delivered to the caller is the caller's to
drop.

Against the review questions: nothing is constructed that execution later
rejects, and `BulkheadCapacity` remains the validated capacity witness
(question 1). No ownership transfer, join, recheck or outcome classification
remains with the caller (question 2). The permit is consumed into the closure
owner and outcomes are an exhaustive enum rather than a `JoinError` whose
cancellation and panic cases each copy interpreted (question 3). Library-owned
execution is the ordinary path; `enter` keeps its signature and behaviour, and
its rustdoc now points blocking work to `run_blocking` (question 4). What
remains is kept explicit (question 5):

- Upstream behaviour: Tokio cannot abort a started blocking closure. The
  blocking pool is shared with `tokio::fs`, DNS resolution and other
  `spawn_blocking` callers, defaults to 512 threads and queues beyond that
  without bound, so bulkhead capacity counts admitted closures, not reserved
  threads. A busy blocking thread still runs queued closures during runtime
  shutdown, so `NotStarted` is not guaranteed. A caller sees it only while the
  pool is shutting down and the time driver has not yet shut down: a
  multi-thread runtime's own tasks or a call polled from outside it, including
  on a plain drop, or a call polled from outside a current-thread runtime
  during `shutdown_timeout` or `shutdown_background`; afterwards a poll panics
  in the timer unless it first observes cancellation, or a deadline that had
  already passed at a context check. When the operating system refuses to
  create a blocking thread, other than a temporary refusal while threads
  exist, Tokio panics in the caller after queueing the closure, with the permit
  inside; that path is source-inspected and untested.
- Application policy: sizing the bulkhead against the pool; whether to replay
  after `Abandoned`, whose effects are unknown and which a retry would run beside
  the first copy within capacity; cooperative `OperationContext::check` calls in
  long closures, which receive no context; the lifetimes of resources an
  abandoned closure still uses; and settlement of outstanding closures before
  dependent cleanup, which stays on ADR-014's
  [watch list](../../adr/014-reuse-after-two-consumers.md#watch-list) until a second
  consumer of this path needs it.
- Unverifiable remote effect: what an abandoned or panicked closure did outside
  the process.
- Low-level escape hatch: `enter` plus a hand-written `spawn_blocking` still
  compiles, as does a permit held by the awaiting future. Forbidding it would
  need a hard cut of the native `OwnedSemaphorePermit` that `enter` returns for
  async work and process-task transfer. The `enter` rustdoc and the facade's
  canonical-path table point blocking work to `run_blocking` and list the
  hand-written form as not equivalent. An or-pattern that merges `NotAdmitted`
  with `Abandoned` also compiles and erases the still-running fact; the enum's
  rustdoc and runnable example match each variant in its own arm. A closure
  that returns unfinished work, such as a future from `|| async move { .. }`
  or a spawned task's or thread's `JoinHandle`, also compiles, because stable
  Rust cannot exclude futures from `T`. That work is not bounded by the
  bulkhead: a returned future runs only after the permit is released, and a
  spawned task or thread may run beside the closure and keep running after the
  release. The `run_blocking` rustdoc says so and points async work to `enter`.

The handoff ordering has one maintenance hazard that no test can detect: an
await inserted between the decision and the handoff would record `admitted` for
work that might never be handed to Tokio. Review guards it.

The integration tests are
[`blocking_ownership.rs`](../../../crates/batter-core/tests/admission/blocking_ownership.rs)
(`cancelled_caller_keeps_capacity_until_the_closure_returns`,
`dropped_caller_keeps_capacity_until_the_closure_returns`,
`deadline_during_wait_abandons_the_result_not_the_work`,
`queued_admitted_work_holds_capacity_and_runs_after_its_caller_is_gone`,
`abandoned_closure_panic_releases_capacity_only_after_unwinding`,
`awaiter_held_permit_is_released_while_its_blocking_work_runs`,
`closed_bulkhead_admits_nothing_new_while_running_work_keeps_its_permit`),
[`blocking_refusal.rs`](../../../crates/batter-core/tests/admission/blocking_refusal.rs)
(`reject_overload_destroys_work_uncalled`,
`precancelled_context_is_not_admitted_with_free_capacity`,
`wait_deadline_interrupts_admission_before_work_starts`,
`wait_cancellation_interrupts_admission_before_work_starts`,
`unpolled_call_and_dropped_admission_wait_do_no_work`,
`admission_granted_after_an_unfired_deadline_never_starts_work`,
`enter_alone_can_grant_a_permit_after_an_unfired_deadline`,
`polling_outside_a_runtime_panics_before_admission`,
`polling_on_a_runtime_without_timers_panics_before_admission`),
[`blocking_dispatch.rs`](../../../crates/batter-core/tests/admission/blocking_dispatch.rs)
(`blocking_work_runs_under_the_dispatcher_captured_at_first_poll`,
`a_call_destroyed_while_waiting_for_admission_destroys_work_under_its_first_dispatcher`,
`a_wait_call_finished_elsewhere_keeps_the_runtime_checked_at_first_poll`,
`an_abandoned_value_is_destroyed_under_the_first_poll_dispatcher`,
`an_abandoned_panic_payload_is_destroyed_under_the_first_poll_dispatcher`) and
[`blocking_outcomes.rs`](../../../crates/batter-core/tests/admission/blocking_outcomes.rs)
(`closure_panic_is_reported_with_its_original_payload`,
`opaque_panic_payload_is_retained`,
`returns_a_value_from_a_blocking_thread_and_frees_capacity_first`,
`fallible_closure_keeps_its_concrete_domain_error`,
`blocking_work_runs_under_the_callers_dispatcher_and_span`,
`run_blocking_future_is_send`,
`a_task_on_a_dropping_multi_thread_runtime_is_not_started`,
`an_outside_call_during_current_thread_shutdown_is_not_started`). The
[unit tests](../../../crates/batter-core/src/admission/blocking/tests.rs)
(`cancelled_blocking_join_maps_to_not_started`,
`panicked_blocking_join_maps_to_its_retained_payload`,
`permit_outlives_work_and_its_captures`,
`work_and_unrun_work_use_the_captured_dispatcher_and_span`,
`recheck_refuses_a_cancelled_or_expired_context_and_releases_the_permit`,
`span_destruction_releases_the_callers_span_under_the_captured_dispatcher`,
`close_keeps_a_running_closures_permit_until_it_returns`,
`abandoned_outcomes_are_destroyed_under_the_captured_dispatcher`,
`a_capture_panic_while_destroyed_unstarted_is_retained_under_the_captured_dispatcher`,
`a_caught_closure_panic_still_records_a_failed_wait`) cover the join mapping,
release order, unrun destruction, span closing on a thread with another
default dispatcher, the permit a closed bulkhead leaves with running work, the
destruction of abandoned values and panic payloads under the captured
dispatcher, including a capture's panic from unstarted destruction, and the
failed outcome a caught closure panic still records on the wait. The
[metrics tests](../../../crates/batter-core/tests/metrics/blocking.rs)
(`handed_off_calls_record_one_admitted_decision`,
`refused_calls_record_their_refusal_once`,
`deadline_refusals_record_deadline_exceeded_not_admitted`,
`run_blocking_destroyed_while_unwinding_records_panicked`,
`run_blocking_outside_a_runtime_records_nothing`) pin one decision per call
polled inside a timer-enabled runtime, and
[`report_redaction.rs`](../../../crates/batter-core/tests/report_redaction.rs)
(`blocking_error_formatting_never_discloses_panic_payloads`) pins redacted,
distinct formatting. The runnable `run_blocking` rustdoc example replaces the
hand-written helper, and two compile-fail rustdoc examples on `BlockingError`
reject `?` conversions. `NotStarted` is exercised through `run_blocking` for a
task on a dropping multi-thread runtime and for a call polled from outside a
current-thread runtime during `shutdown_timeout`. Fresh-agent usability
evaluation is proposed and unexecuted.

## Owned phase and acquisition handoff assessment (`batter-jxj2`)

The initial implementation (`batter-jxj2`) owned phase execution and generic
pair-to-registration handoff. The direct panel follow-up `batter-17fe` added
Runledger guidance and dispatch regressions. The branch review then exposed
stale primary examples and missing zero-reserve execution coverage. Root-cause
assessment `batter-4qvs` found two deeper locally expressible gaps; `batter-rtxb`
implements the complete bounded correction below.

**Phase outcomes.** The coordinator already owned sequencing: work destruction
precedes finalization, whose callback receives retained work evidence. It reuses
command polling and existing publication ownership rather than a second runtime.
Owner loss cancels work only; parent cancellation stops both, and cancelled
borrowed waiters own no execution. Typed finalization cannot be a command cleanup
hook because it needs typed work evidence, a typed return, and the original total
and parent lineage, while command cleanup has an independent cancellation budget.

The former checked wait returned the same general report type on success and
failure. Consumers could relabel successful evidence as `PhasedFailure::Report`
and the raw phase result could appear successful despite late interruption.
Now `PhaseOutcome` privately stores accepted success or opaque `PhaseFailure`;
`classify` checks the complete contract, while failed evidence explicitly exposes
`returned_result`, late interruption and destruction. Checked wait returns
`PhasedSuccess<T,U>` holding both accepted values, or `FailedPhasedReport` holding
both phase outcomes. Constructors are private. No mutable access, general-report
conversion to failure, or repeated consumer success check is needed.

**Publication ownership (`batter-w8fr`).** The subsequent panel reproduced a
library ownership gap: the publisher claimed `ScopedOutput` before storing the
checked result in watch. A receiver outliving the publisher could destroy unread
evidence under an unrelated subscriber. Execution wrappers could not protect
that later destruction. Publication now requires a private `SharedOutput` built
from guarded output, with no raw accessor. Sharing observers only shares this
guard; completed `wait` synchronously claims the checked result. Its internal
one-way atomic observation state supports concurrent observers without requiring
domain Clone or adding a paired consumer operation. Static ownership protects
the handoff; actual observation remains a runtime event. Tests synchronize on
publisher closure, then abandon success/error evidence, and verify both orders
of caller-owned destruction after observation. Public signatures are unchanged.

The two accompanying lifecycle gaps were evidence/documentation gaps: successful
work already cancelled its context before finalization, and handler destruction
already preceded native invocation exit. Controlled tests now exercise both.
Changing native ordering or treating cancellation as remote write authority would
change the wrong owner; application/native fencing remains outside this proof.

**Acquisition ownership.** A factory yielding `Acquired` cannot prove when its
inner native resource was acquired. Cancellation after native success but before
pair construction, and a nested native future's destruction panic, both left no
registered finalizer in executed assessment probes. Calling these only partial
acquisitions describes the limitation; it does not remove the agent's obligation.

`CleanupSlot::acquire_semaphore` and `bind_udp` now take native configuration, not
application callbacks. The private kernel polls the actual native future,
retains its raw output, registers a fixed library cleanup action, destroys the
completed future, then returns. The resource local lives outside that destruction
scope: a native destructor panic unwinds the undelivered reference while cleanup
retains its own. A regression caught and removed the alternative return-expression
ordering that leaked the undelivered Arc. No caller finalizer-builder or projection
runs in the handoff. Semaphore permits never escape cleanup; UDP returns shared
native ownership and explicitly requires all other references to be dropped for
socket closure. SQLx `pool_in` remains the corresponding adapter-owned operation.
This is a bounded supported-resource set, not a generic resource registry or DSL.

Generic pair-producing factories moved to `cleanup::low_level::acquire_pair`,
with `Acquired` in the same namespace. They still own all native acquisition-to-pair
obligations and are not the canonical path. Manual `register` remains for explicit
application hooks and supported adapters; arbitrary remote correctness and
resource/finalizer correspondence cannot be established by local types. No public
user-implementable trait disguises those obligations as a proof.

**Native clocks and compatibility.** Existing phase pairs transfer without
recomputing clocks. Zero reserve remains an explicit supported policy: after the
shared deadline both outcomes are interrupted and the finalization factory does
not run. Runtime checks own clock passage, not typestate. Paused-time tests cover
this execution and conversion of Runledger invocation pairs, including native exit.

Public hard cuts and before/after migrations are in CHANGELOG. No deprecated
same-shape shim restores general success/failure report interchange or generic
acquisition on the protected slot path. Root aliases for generic pairs would
blur the explicitly lower-level boundary and are omitted. Primary command/startup
examples and guides now use supported acquisition; the finite-command example
keeps its application reporting failure hook separate from native socket release.

Tests cover failures, raw returned values after interruption, native-future
unwinding, pending acquisition cancellation, LIFO cleanup, type-level forgery
rejection, zero-reserve execution and tracing through both spawned paths. These
are executable failure scenarios, not fresh-agent usability measurements; that
separate methodology remains unexecuted. Runtime death, non-yielding tasks,
detached children and remote effects retain their explicit existing boundaries.
The outbound example retains its distinct ready-response precedence, rather than
wrapping its exchange evidence in the cancellation-first generic runner.

## Async bulkhead execution (`batter-f9kq.1`)

The audit reproduced two concurrent workers through capacity one when callers
used `let _ = enter(...).await?`. The native guard was released before work.
The canonical async `Bulkhead::run` now owns acquisition, the post-admission
context check and the whole factory future. The callback receives a child
context, never the permit. A local guard retains capacity through future
destruction on every exit path. Waiting and work share the original deadline;
refused and unpolled calls never invoke the factory. Errors retain concrete
application causes without printing them. `Bulkhead::enter` is removed with no
shim because exposing the raw permit restores the invalid lifetime. All consumers
migrate to owned execution; the breaking call shapes are in CHANGELOG.

Independent regression scenarios cover overlapping factories, cancellation
and destructor ordering, late semaphore grants with an unfired deadline timer,
shared waiting/execution budget, refusal, panics and concrete error retention.
The facade HTTP example exercises the new canonical API. No fresh-agent
usability evaluation is claimed. A future returned as a value, native detached
work and remote effects are not locally owned work. Protocols retaining durable
outcomes must not equate cancellation of an enclosing operation with rollback;
`run_resolved` applies their retained-evidence resolver inside the same owned
admission boundary. Reference submission uses its existing transaction progress
resolver here rather than placing another interruptible boundary around it.

The [public API comparison](../public-api-diff-2026-11-06/README.md) records
the executed surface diff and the no-shim migration decision for this cutover.
