# Own startup through cleanup and service handoff

This living ExecPlan follows `.agent/PLANS.md`. Bead `batter-7r3.3` owns delivery.
The worktree includes completed, uncommitted SQLx adapter work; preserve it.

## Purpose / Big Picture

Applications should initialize native resources, register cleanup and start a
Supervisor without losing cleanup when a caller stops waiting. The HTTP and
PostgreSQL examples will share owned startup, Unix signal installation and
retained shutdown-failure handling. Resources and policies remain explicit.

## Progress

- [x] (2026-09-09) Verify task readiness and inspect core and example ownership.
- [x] Add cleanup reservations and owned startup with retained failure reports.
- [x] Share Unix signal installation and shutdown report interpretation.
- [x] Exercise cancellation, panic, registration and handoff failure boundaries.
- [x] Adopt helpers in SQLx and HTTP compositions and update contracts/evidence.
- [x] Run both Rust matrices, example smokes and live SQLx checks.
- [x] Audit functional acceptance, source changes and retained validation evidence.
- Final Jig gate and closure results are recorded by `work check` / `work finish`
  in this plan's append-only history after the final metadata update.

## Surprises & Discoveries

CleanupStack::push consumes the factory even on invalid/duplicate names. A
reservation can validate before acquisition and make later registration
infallible. Existing RunningSupervisor already retains shutdown ownership apart
from waiters, and last-owner drop requests drain. The HTTP example currently
stringifies unsuccessful shutdown reports; this must be replaced by retention.

## Decision Log

Use a separately spawned startup coordinator holding the Supervisor. Its
initializer borrows a startup scope via a boxed native Send future, so the
coordinator can recover the supervisor after completion, cancellation or unwind
without a shared service container. Stage names are explicit application metadata.
Catch initializer construction/polling and destruction unwinds while keeping
original causes; do not suppress the panic hook or imply cleanup after aborting
panics/runtime death. Preserve cleanup independence from operation cancellation.

Signal consumption starts with the registered critical component, not during
earlier startup work. Installation is immediate and errors are startup failures.
Native process-wide signal disposition is not restored by dropping listeners.

Reserve cleanup names before polling acquisitions. A reservation exclusively
borrows its stack and registers a captured finalizer synchronously after success;
no asynchronous bracket, resource registry or automatic acquisition is added.
Native resources acquired but not yet registered retain their native Drop limits.

Keep running-driver ownership out of cloneable startup observations. Transfer the
RunningSupervisor through one owned receiver; a lost receiver drops that owner
and requests drain while the existing driver still owns cleanup. Report startup
failure separately with a concrete application error and complete CleanupReport.

## Outcomes & Retrospective

Implemented owned startup, validated cleanup slots, pre-readiness signal
installation and retained shutdown interpretation. Both compositions use the
helpers; native resources and policy remain explicit. Fifteen focused regressions
cover waiter/owner and handoff loss, registration rejection with prior cleanup,
LIFO error preservation, deadline, factory/poll/destructor panic, and readiness.
A separate injected signal installation failure retains its OS cause and cleanup.
Both Rust matrices and all HTTP/PostgreSQL smokes/live cases passed. File-budget
check passed with unwaived size warnings (lifecycle: 766; startup tests: 513 lines).
Final Jig gate/closure outcomes live in the plan history. No new macOS/hosted
execution or stronger transitive cleanup guarantee is claimed.

## Context and Orientation

`crates/batter/src/cleanup.rs` owns finalizers; `lifecycle/driver.rs` owns running
drivers and completion reports. Add focused startup and Unix signal modules
under `crates/batter/src`, exposed through normal modules. Keep HTTP serving in
the adapter example and SQLx pool policy in the PostgreSQL example. Generic
test support stays a leaf; the core gains no adapter dependency.

## Plan of Work

First add a validated CleanupSlot and Supervisor reservation forwarding method.
Build an inert startup specification with an explicit start boundary, a single
handoff owner, cloneable observation, and an initializer scope containing stage
metadata and access to the unstarted supervisor. Bound initialization by an
OperationContext and process drain, destroy the initializer before cleanup, and
retain failures/panics alongside explicit cleanup outcomes. Successful
initialization arms readiness and starts the existing owned driver. A lost
handoff requests drain rather than abandoning resources.

Install SIGTERM/SIGINT listeners synchronously during initialization and move
them into a registered critical component. Installation failure stays a startup
failure. Add a small safe error wrapper for unsuccessful retained shutdown
reports, used by both examples instead of duplicated string conversions.

Use deterministic barrier tests on both sides of acquisition/registration and
running handoff. Verify exactly-once LIFO cleanup, original errors plus cleanup
failures, inert factories, drain-before-ready, owner loss, borrowed waiter
cancellation, panic, and shutdown report preservation. Reuse existing lifecycle
failure tests for abort/unjoined semantics rather than changing them.

## Concrete Steps

From `/home/aa/Documents/batter`, run focused core startup/cleanup tests during
development. Final verification is `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`. Build the HTTP example on each
toolchain and run `scripts/smoke_http.py` with default, --signal SIGINT, --deadline,
--warn-filter and --warn-filter --deadline. Run native PostgreSQL live example
tests and both process smokes against the existing local PostgreSQL 18.6 fixture,
then `bash scripts/test_sqlx_live.sh` to preserve the completed adapter contract.
Run Jig work check/evidence/gates and finish using fresh api:test evidence.

## Validation and Acceptance

Only successfully acquired and registered resources receive finalizers, in LIFO
order. Rejected name reservations invoke no acquisition; failure after a prior
acquisition retains its cleanup and original registration error. Cancelling a
waiter does not cancel owned cleanup. Owner drop requests termination; no
completion report invents asynchronous Drop. Initializer completion cannot revive
draining readiness. Both native examples use the public helpers and pass process
SIGTERM/SIGINT smokes. New public APIs have rustdoc and compiling/runnable usage.

## Idempotence and Recovery

Preserve pre-existing SQLx work and Beads state. Do not commit or publish. No
database migration or provisioning belongs here. Record evidence before the
final Jig profile, since later documentation changes invalidate its digests.

## Interfaces and Dependencies

Use existing Tokio 1.53.1, watch/oneshot channels, OperationContext,
CleanupStack and RunningSupervisor. Add Tokio's native Unix signal capability
without any Windows fallback. Concrete initializer E remains owned in startup
failure reports with safe Debug/Display and deliberate trusted cause inspection.
CleanupSlot borrows its stack and consumes itself on finalizer registration.
