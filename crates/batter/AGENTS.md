# Foundation guide

## Purpose

Own native Tokio operational contracts. Read the root guide and
[guarantees](../../docs/guarantees.md) before changing failure behavior.

Follow the root [Unix-only platform policy](../../AGENTS.md#platform-scope).
Windows support and non-Unix fallbacks are out of scope.

## Key entrypoints

- `src/lib.rs` defines the public modules and boundary error aliases.
- `src/lifecycle.rs` and `src/lifecycle/` own process tasks and shutdown reports.
- `src/lifecycle/tasks.rs` privately owns direct-task joins, metadata, outcome
  recording and escalation bookkeeping. The coordinator uses operations and
  an owned summary, never the underlying collections.
- `src/lifecycle/report.rs` owns the shutdown report type and retained-outcome summary;
  its public path remains `batter::lifecycle::ShutdownReport`.
- `src/lifecycle/managed.rs` and `managed/` own adapter-facing inert native factories,
  library initialization acknowledgement and independently retained native settlement.
  Application composition uses the native adapter; descendant accounting stays native.
- `src/lifecycle/state.rs` owns all readiness/admission facts and transitions;
  its private snapshot writer requires the admission mutex guard.
- `src/settings.rs` and `src/settings/` own explicit source/bound/redaction
  mechanics; application schemas and native adapter constructors stay outside.
- `src/operation.rs`, `src/retry.rs`, and `src/admission.rs` bound application work.
- `src/health.rs` and `src/health/` own dependency sampling and read-only observations.
- `src/cleanup.rs` drives explicit LIFO finalizers and validates acquisition reservations.
- `src/startup.rs` and `src/startup/` own initialization, cleanup and driver handoff.
- `src/command.rs` and `command/` own finite callbacks and independently retained
  LIFO finalization, with separate work/cleanup outcomes and an optional total reserve.
- `src/completion.rs` privately owns the snapshot-before-wait mechanism shared by
  command, startup and process completion observers; their public policies stay separate.
- `tests/component_ownership.rs` compares acknowledged initialization and joined
  children with a nonconforming wrapper whose hidden child survives cleanup.
  `tests/non_yielding/` owns the fixture, timing policy and watchdog self-tests;
  private std-only mechanics live in workspace `test-support/process/`.
- `src/telemetry.rs` exposes observations and the adapter dispatch seam;
  `src/scoped_dispatch.rs` owns its private pin/drop implementation.

## Edit here for X

Keep runtime-independent application policy out of this crate. Change HTTP
translation in `batter-axum`; change SQLx composition in the example package.
Put foundation failure tests in `tests` and core usage examples in `examples`.
Update the root contracts, implemented status, owning Bead, and validation for behavior changes.

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
requests cannot extend it. Idempotent native stop callbacks return the earlier
native/parent timestamp. Propagate new earlier clocks to settling components and
wake active phase waits; keep the first failure cause independent of tightening.
Publish caught stop-control failures at their catch boundary, including repeated
clock updates; pending native settlement must not hide an already-observed error.
`StartingSupervisor` exposes startup observation without retaining running ownership.
Finite command scopes expose validated cleanup reservations, never a takeable
stack or process-task admission. Command cancellation is downward and cleanup
does not inherit it; callback return/unwinding cannot bypass registered cleanup.
Reserve cleanup names before acquisition and register immediately after success.
Extracted finalizers remain explicitly awaited and must not inherit process
operation cancellation; extraction does not detach captured tokens.
Health readers create no work; the sole monitor owns and destroys its active
probe on drain. Probe failure permits recovery and does not drain the process.
Freshness is evaluated from the observation timestamp on each read; readers
never prolong writer ownership or refresh a successful timestamp.
Stop admission before cancellation. Harvest ready tasks before escalation;
do not equate aborted wrappers with stopped detached work. Keep conservative
cleanup skipping after uncertain termination. Never print cause contents or
install a global subscriber/panic hook. Protected dispatch covers full future
destruction and nested spans. Task instrumentation retains the available parent
when its own span is filtered; create spans and look up fallback parents outside
the admission mutex. The foundation cannot depend on its adapters;
generic test support remains a leaf dependency used by tests.

## Common commands

Run from the workspace root:

```sh
cargo test -p batter --locked
cargo test -p batter --doc --locked
cargo clippy -p batter --all-targets --locked -- -D warnings
scripts/jig check test
```
