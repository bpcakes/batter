# Foundation guide (`batter-core`)

## Purpose

Own native Tokio operational contracts. Read the root guide and
[guarantees](../../docs/guarantees.md) before changing failure behavior.

Follow the root [Unix-only platform policy](../../AGENTS.md#platform-scope).
Windows support and non-Unix fallbacks are out of scope.

## Key entrypoints

- `src/lib.rs` defines the public modules and boundary error aliases. The
  `batter` facade re-exports this implementation without wrappers.
- `src/lifecycle.rs` and `src/lifecycle/` own process tasks and shutdown reports.
- `src/lifecycle/tasks.rs` privately owns direct-task joins, metadata, outcome
  recording and escalation bookkeeping. The coordinator uses operations and
  an owned summary, never the underlying collections.
- `src/lifecycle/report.rs` owns the shutdown report type and retained-outcome summary;
  its public path remains `batter::lifecycle::ShutdownReport`.
- `src/lifecycle/managed.rs` and `managed/` own adapter-facing inert native factories,
  library initialization acknowledgement and independently retained native settlement.
  `managed/drive/initialization.rs` couples the initializer and sole readiness
  acknowledgement; guarded observers retain completed futures for separate destruction
  without polling them again. Application composition uses the native adapter;
  descendant accounting stays native.
- `src/lifecycle/state.rs` owns all readiness/admission facts and transitions;
  its private snapshot writer requires the admission mutex guard.
- `src/periodic.rs` and `periodic/` own the one serial schedule, per-run budget,
  startup acknowledgement policy, two library-owned stopping classes and
  bounded recoverable-failure evidence. The operation remains application- or
  native-owned.
- `src/settings.rs` and `src/settings/` own explicit source/bound/redaction
  mechanics; application schemas and native adapter constructors stay outside.
- `src/operation.rs`, `src/retry.rs`, and `src/admission.rs` bound application work.
  `operation/tracked.rs` owns opaque non-invocation evidence and the tracked
  public entrypoint; all run variants share the boundary in `operation.rs`.
  Keep the invocation marker immediately before the factory call, and preserve
  cancellation/deadline precedence for both tracked and legacy callers.
  `operation/resolved.rs` owns the lower-level `ResolvedOutcome` and
  `run_resolved_value` seam: one terminal classification carries its adapter value,
  and the existing boundary observes it before releasing the value. Never split
  that terminal state into a result plus a separately retained value slot.
- `src/admission.rs` owns the validated `BulkheadCapacity`; lifecycle process
  admission owns the distinct validated `ProcessCapacity`. Do not reopen raw
  primitive constructor paths or merge the two capabilities.
- `src/admission/asynchronous.rs` owns async factory admission, context rechecks,
  and permit retention through complete work-future destruction. `run_resolved`
  keeps retained-outcome resolution within that same boundary. Raw `Bulkhead::enter`
  has been removed; never restore a public permit-transfer path.
- `src/admission/blocking.rs` owns `Bulkhead::run_blocking` and `BlockingError`:
  admission, a context recheck, one bulkhead decision and a same-poll handoff
  of the permit into the blocking closure.
- `src/health.rs` and `src/health/` own dependency sampling, read-only observations,
  and the exhaustive observation-to-dependency-readiness projection.
- `src/readiness.rs` owns sampling order and the pure lifecycle, dependency and
  application-condition decision; adapters translate its valid result.
- `src/operation/phased.rs` and `phased/` own inert work/finalization assembly,
  typed success/failure proofs and work-only owner cancellation. Reuse command phase
  polling; finalization inherits the parent and original total deadline.
- `operation/phased/scoped.rs` is the lower-level borrowing kernel for adapters
  whose native host owns the whole invocation; it shares command phase polling.
  It is not an independently owned application finalizer.
- `src/cleanup.rs` drives explicit LIFO finalizers and validates acquisition reservations.
  `cleanup/acquisition.rs` owns native semaphore/UDP acquisition through registration
  and native-future destruction. `cleanup/low_level.rs` owns caller-assembled pairs.
- `src/startup.rs` and `src/startup/` own initialization, cleanup and driver handoff.
  `startup/initializer.rs` owns the sealed `Initializer` bound that boxes async-closure
  futures; `startup/application.rs` owns `ApplicationError`, which must never implement
  `std::error::Error` (its blanket `From` and `InitializationError` impl depend on that).
- `src/command.rs` and `command/` own finite callbacks and independently retained
  LIFO finalization, with separate work/cleanup outcomes and an optional total reserve.
  `command/owner.rs` owns the sole `CommandOwner` and its consuming `finish`;
  `command/report.rs` owns `CommandSuccess` and the private `into_checked`;
  `command/signals.rs` owns the private `WorkSignals` seam and SIGTERM/SIGINT
  listeners; `command/driver.rs` owns both coordinators; injected-source unit
  tests live in `command/tests.rs` and `command/tests/`.
- `src/service.rs` and `service/` own protected startup through separately retained diagnostic completion. `service/readiness.rs` owns single-use typed output and readiness observation; `lifecycle/state/readiness.rs` keeps the deadline gate under the existing lifecycle lock. Only a protected startup can release the opaque diagnostic completion witness; adapter panics never replace service outcomes.
- `src/completion.rs` privately owns the snapshot-before-wait mechanism shared by
  command, startup and process completion observers; their public policies stay separate.
- `tests/component_ownership.rs` compares acknowledged initialization and joined
  children with a nonconforming wrapper whose hidden child survives cleanup.
  `tests/non_yielding/` owns the fixture, timing policy and watchdog self-tests;
  private std-only mechanics live in workspace `test-support/process/`.
- `src/telemetry/metrics.rs` owns the opt-in bounded metric catalog; shared schema/validation in `metrics/catalog.rs`, closed
  label vocabularies are defined once in `metrics/vocabulary.rs`, name
  tables in `metrics/names.rs`, bounded key caches in `metrics/keys.rs` and
  the shared record-once guard in `metrics/terminal.rs`. Call sites use the
  always-compiled `telemetry/record.rs` shim, never feature attributes. Emit only after results are known
  and outside admission locks; never add a buffer, retry or log for metrics.
- `src/panic_hook.rs` owns `redacting_panic_hook`, the boxed hook value an
  application root installs as the first statement of `main`. Its private
  renderer receives only a location. `tests/panic_hook.rs` runs it in fresh
  child processes with secret-shaped payloads and default-hook controls.
- `src/telemetry.rs` exposes observations and the adapter dispatch seam;
  `src/scoped_dispatch.rs` owns its private pin/drop implementation and the
  `ScopedOutput` guard that destroys unclaimed task output under its producer.
  Its private `SharedOutput` keeps that protection through publication storage;
  only completed phase observation claims the result.

## Edit here for X

Keep runtime-independent application policy out of this crate. Change HTTP
translation in `batter-axum`; change SQLx composition in the example package.
Put foundation failure tests and rustdoc examples here. Runnable consumer examples
belong in `crates/batter/examples` and use the public facade.
Update the root contracts and implemented status, then record executed checks in the owning Bead.

## Invariants

Preserve child deadline clamping, downward cancellation, inert factories,
explicit replay permission, bounded process admission, and retained failures.
Keep all state mutations inside `lifecycle/state.rs`. Publish the readiness
snapshot while holding its transition guard; `Stopped` cannot move backward.
Explicit notification and cancellation happen after releasing the guard.
Readiness reads must remain available while native enqueue holds admission.
Supervisor abandonment signaling is owned from construction and transferred to
the driver; it precedes captured-value destruction and never runs finalizers.
Supervisor completion channels belong to `Supervisor::start`; expose shutdown
observers through `RunningSupervisor`, never shutdown control handles. Managed
native completion channels are created when the supervisor driver starts, and
their observers travel in managed report records. Their independently retained
owners continue after direct waiter abortion. Pending native settlement forbids
cleanup; later observation never runs previously skipped finalizers.
The earliest stop timestamp belongs to `lifecycle/state.rs`; phase entry and later
requests cannot extend it. `ShutdownSignal::abort_deadline` is that timestamp
plus the supervisor's drain and cancel allowances, recorded once at
construction: read-only observation that confers no authority, unset for
standalone handles, and never a promise that abort happens at that instant.
`ShutdownSignal::abort_deadline_before` only waits on the same clock's change
notification; keep it read-only and resolving only for an earlier deadline.
`OperationContinuation` is created only by a successful
`lifecycle::low_level::admit_root_with_continuation`; keep both under
`lifecycle::low_level`, apart from `OperationAdmission::admit_root`. Keep it
without a public constructor or `Clone`, keep `start` consuming so one
admission continues into at most one root, keep that root under process forced
cancellation alone, start it already cancelled once the admitted operation was
cancelled or passed its deadline, and never let it read or bypass readiness
for work that was not already admitted. Idempotent native stop callbacks return the earlier
native/parent timestamp. Propagate new earlier clocks to settling components and
wake active phase waits; keep the first failure cause independent of tightening.
Publish caught stop-control failures at their catch boundary, including repeated
clock updates; pending native settlement must not hide an already-observed error.
`StartingSupervisor` exposes startup observation without retaining running ownership.
Typed initializer output travels only in the one-shot handoff, never to observers
or failure reports; output rejected after a successful return is destroyed before cleanup.
Finite command scopes expose validated cleanup reservations, never a takeable
stack or process-task admission. Command cancellation is downward and cleanup
does not inherit it; callback return/unwinding cannot bypass registered cleanup.
Reserve cleanup names before supported native/adapter acquisition; the library
owns registration before release. Manual registration and generic pair factories
are lower-level boundaries, never equivalent protected acquisition.
Command signal listeners exist only while work can be cancelled: install them
synchronously before the owned start returns, poll them only in the coordinator,
and drop them after work-future destruction and before cleanup. Owned completion
moves a value out only through `into_checked`'s full report check; never share
the owned report or add a `CommandCause`, `CommandFailure` variant or report
field without a hard-cut entry.
Extracted finalizers remain explicitly awaited and must not inherit process
operation cancellation; extraction does not detach captured tokens.
Health readers create no work; the sole monitor owns and destroys its active
probe on drain. Probe failure permits recovery and does not drain the process.
Freshness is evaluated from the observation timestamp on each read; readers
never prolong writer ownership or refresh a successful timestamp.
Keep `HealthStatus` observation separate from `DependencyReadiness`; classify
every observation explicitly, with no catch-all. Overall readiness is either
Ready or Unready with a reason, and dependency reasons cannot contain Healthy.
Sample configured dependency before lifecycle so an observed drain overrides cached health.
The lifecycle_only constructor asserts no continuous dependency observation; it
creates no monitor, never removes a configured dependency, and retains condition
narrowing and the final lifecycle read.
Ask application conditions only after a ready dependency sample (or with no
configured dependency) and before the
final lifecycle read; they can only narrow a ready decision to a named
condition reason.
Stop admission before cancellation. Harvest ready tasks before escalation;
do not equate aborted wrappers with stopped detached work. Keep conservative
cleanup skipping after uncertain termination. Never print cause contents or
install, take or replace a global subscriber or panic hook; `redacting_panic_hook`
only constructs the value. Inside that hook, never read the payload or the
thread name, call another hook, emit tracing, panic or take the `Stderr` lock;
write nothing when descriptor 2 cannot be duplicated. Protected dispatch covers full future
destruction and nested spans. Task instrumentation retains the available parent
when its own span is filtered; create spans and look up fallback parents outside
the admission mutex. The foundation cannot depend on its adapters;
generic test support remains a leaf dependency used by tests.
Keep no await between the blocking-run decision and `spawn_blocking`. Never
expose the blocking permit or `JoinHandle`, abort the blocking task, or add
`From` conversions to `BlockingError`. Release the permit after the closure
and its captures, before the result is published. Close the blocking span and
destroy unrun work, and an abandoned value or panic payload, under the captured
dispatcher: catch a closure panic inside the blocking closure and return its
payload, like a value, in the private wrapper, and rethrow a capture's panic
from unstarted destruction in that wrapper, so Tokio never drops either
unwrapped on the blocking thread. Look up the runtime and its
time driver before admission, and poll the result wait inside that runtime. Shutdown settlement of
abandoned closures stays on the ADR-014 watch list.

## Common commands

Run from the workspace root:

```sh
cargo test -p batter-core --locked
cargo test -p batter-core --features metrics --locked
cargo test -p batter-core --doc --locked
cargo test -p batter-core --locked --test command_signals
cargo test -p batter-core --locked --test panic_hook
cargo clippy -p batter-core --all-targets --locked -- -D warnings
python3 scripts/test_matrix.py workspace
python3 scripts/test_matrix.py no-default-features
```
