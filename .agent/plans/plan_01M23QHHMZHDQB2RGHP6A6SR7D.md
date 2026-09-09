# Own dependency health sampling and fresh observations

This ExecPlan follows `.agent/PLANS.md`. Bead `batter-7r3.1` owns scope and
acceptance. Preserve the completed, uncommitted SQLx/startup work in this tree.

## Purpose / Big Picture

A service should sample a dependency once on its own schedule while many HTTP
readiness reads inspect a fresh cached result without querying the dependency.
The monitor is one ordinary supervised future. Failed or timed-out probes make
the dependency unready but permit recovery without draining the whole process.

## Progress

- [x] (2026-09-09) Verify readiness, claim the task and inspect native lifecycle APIs.
- [x] Add validated timing policy, sole monitor owner and typed read-only snapshots.
- [x] Prove concurrency, freshness, recovery and destruction/drain boundaries.
- [x] Use the monitor in the HTTP composition and update public contracts/examples.
- [x] Execute both Rust verification matrices and all HTTP process smoke profiles; correct and rerun the 1.98.1 rustdoc step.
- [x] Audit functional acceptance and record executed validation evidence.
- Final Bead/Jig gate and closure outcomes are recorded in the tracker and this
  plan's append-only history after the final metadata update.

## Surprises & Discoveries

Tokio 1.53.1 timeout polls its inner future first and cannot preempt non-yielding
work. Use explicit drain/cancel/deadline precedence and check before publication.
An unescaped `Arc<E>` in a rustdoc comment failed the first 1.98.1 documentation
step. Backticks corrected it; the exact rustdoc command passed afterward. Runtime
checks and Clippy had already passed, with no semantic change in the correction.

The existing Supervisor owns direct task destruction and cleanup sequencing;
health does not need another task owner or detached background task.

## Decision Log

Use a HealthPolicy with explicit probe budget, completion-to-next-probe delay,
maximum observation age and scheduling margin. All durations must be positive,
representable, and checked together; maximum age must cover delay plus one probe
budget plus margin. Staleness is computed by readers from monotonic observation
time, never refreshed by reads or an owner's continued existence.

Use a non-cloneable HealthMonitor containing a native FnMut probe factory and a
cloneable HealthReader with shared private publication state. A short mutex
publishes result/time and writer liveness together; no callbacks, application
formatting or replaced-error destruction occur under that mutex. Preserve the
last concrete application error behind Arc without requiring E: Clone. Redact
Debug. Unknown, failure, timeout, stale and stopped states are distinct.

The monitor run future is inert until polled, integrates with ShutdownSignal,
acknowledges initialization when the sampling loop becomes runnable, and owns at
most one native probe. Readiness approval remains separate from dependency health.
Drain closes probe admission and drops the active future before monitor completion.
A final drain check prevents late completion from reviving a stopped writer.
Writer drop invalidates readers, including unpolled run abandonment. Native panic
propagates as a critical task failure; returned probe errors/timeouts do not.

Schedule each delay from completion, avoiding interval catch-up. Keep timeout
around the whole supplied future, including application acquisition/query stages.
No framework, health registry, adapter/database dependency, async Drop, detached
child ownership or remote cancellation acknowledgement is introduced.

## Outcomes & Retrospective

Implemented the monitor and typed observations, with fifteen focused core
regressions and a controlled HTTP health-readiness regression. Existing live
HTTP lifecycle/telemetry tests pass with the monitor registered. Both Rust runtime matrices, formatting, Clippy, doctests and final rustdoc pass;
all five HTTP profiles pass on each toolchain. The first 1.98.1 rustdoc step failed
on an unescaped type in a comment; the comment was corrected and only that
failed check rerun. Final Jig records supply fresh default-toolchain backend
evidence. New macOS/hosted execution remains unverified. A point-in-time snapshot is not a perpetual readiness
certificate; each readiness decision must request a fresh snapshot and also
check process lifecycle readiness.

## Context and Orientation

Foundation implementation belongs in `crates/batter/src/health.rs` and focused
`health/` modules if useful. `lifecycle.rs` exposes ShutdownSignal and Supervisor.
`scoped_dispatch.rs` already protects owned future polling and destruction.
Place deterministic tests in `crates/batter/tests/health.rs` with test-owned
fixtures. The HTTP example in `crates/batter-axum/examples/http_service.rs`
combines dependency observation with process state at its readiness endpoint.
Update `docs/guarantees.md`, `docs/usage.md`, architecture, status, references,
testing, validation and owning guides as appropriate.

## Plan of Work

First implement timing validation and read-only publication/snapshot types with
rustdoc. Then implement the inert monitor and consuming supervised run future.
Use paused clocks and explicit barriers to test normal sampling, failure recovery,
expiration with a stalled owner, reader concurrency and cancellation boundaries.
Finally adopt the API in the runnable HTTP root without changing transport/body
ownership or existing completion observations. Its ordinary dependency probe is
an explicit demonstration, not a claimed real database health check.

## Concrete Steps

From `/home/aa/Documents/batter`, run focused `cargo test -p batter --test health
--locked` and relevant HTTP example tests during implementation. Run
`bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`.
On each toolchain build `cargo build -p batter-axum --example http_service --locked`
and run `python3 scripts/smoke_http.py --binary target/debug/examples/http_service`
in default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and
`--warn-filter --deadline` modes. These require loopback/subprocess permission.
Use `scripts/jig work check`, inspect fresh api:test evidence and gates, and
finish the plan. Finalize metadata before the final profile to preserve freshness.

## Validation and Acceptance

Two thousand concurrent reads must produce no extra probe attempts. Max active
probes stays one, and delayed scheduling does not produce a catch-up burst.
Unknown/failed/timed-out/stale/stopped snapshots are unready. Clock advances
expire a successful sample even if its writer is stalled. Later success recovers
from failed/timeout probes. Drop of the sole writer invalidates retained readers.
Drain during a probe, delay, and immediately before publication prevents later
success; active future destruction precedes monitor completion and cleanup.
Invalid zero/overflow/incompatible timings reject before any factory invocation.
Runnable HTTP use and deterministic tests must establish these claims.

## Idempotence and Recovery

No database provisioning or persistent schema changes. Preserve prior changes
and failure tests; repair failures without relaxing assertions. Do not commit,
push, publish or deploy. Keep local logs under `.agent/tmp/batter-7r3.1/`.

## Interfaces and Dependencies

Expose HealthPolicy, HealthMonitor<F,E>, HealthReader<E>, HealthSnapshot<E>,
HealthStatus, ProbeObservation<E> and ProbeOutcome<E> from `batter::health`.
Use native std Arc/Mutex and Tokio 1.53.1 Instant/sleep/select. The monitor creates
no tasks; applications register its run future as a critical component. Private
fields prevent caller mutation of writer state or fabricated live observations.

Final update (2026-09-09): implementation and functional acceptance are verified;
exact commands and the corrected rustdoc outcome are recorded in validation.
Final workflow evidence is kept in the plan history to avoid changing checked
inputs after its final gates.
