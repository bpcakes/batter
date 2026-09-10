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
- `src/lifecycle/state.rs` owns all readiness/admission facts and transitions;
  its private snapshot writer requires the admission mutex guard.
- `src/operation.rs`, `src/retry.rs`, and `src/admission.rs` bound application work.
- `src/health.rs` and `src/health/` own dependency sampling and read-only observations.
- `src/cleanup.rs` drives explicit LIFO finalizers and validates acquisition reservations.
- `src/startup.rs` and `src/startup/` own initialization, cleanup and driver handoff.
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
Completion channels belong to owned drivers: create them only in `start` and
expose shutdown observers through `RunningSupervisor`, never shutdown control handles.
`StartingSupervisor` exposes startup observation without retaining running ownership.
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
