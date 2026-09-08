# Bound non-yielding lifecycle evidence with child processes

This ExecPlan belongs to Bead `batter-fvz` and follows `.agent/PLANS.md`.
Beads owns delivery scope and acceptance. The Git baseline is
`e5f2f04b2dbb349d08085caf177f662fcbc89812`.

## Purpose

Run real non-yielding Tokio tasks without risking the Cargo test runner. A
parent OS process enforces elapsed wall time, retains child evidence, and kills
and reaps stuck children. Tests distinguish a cooperative shutdown, a report
that honestly retains unjoined work, and a blocked runtime unable to report.

## Progress

- [x] (2026-09-08) Inspect the clean baseline, repository guides, existing tests,
  the ready Bead and resolved Tokio 1.53.1 semantics; claim `batter-fvz`.
- [x] (2026-09-08) Implement child fixtures, independent watchdog, captured
  diagnostics, missing-milestone rejection and parent-unwind cleanup.
- [x] (2026-09-08) Run focused cases and reject temporary regressions hiding
  unjoined work or invoking cleanup while the direct task remains live.
- [x] (2026-09-08) Update contracts, test instructions, status, pinned primary
  references and validation evidence.
- [x] (2026-09-08) Run both Rust verification matrices and all three HTTP smoke
  modes; all pass on the final test source.
- [x] (2026-09-08) Pass all five Jig targets, inspect fresh required gate
  evidence, and pass the final backend `scripts/jig check test`.

## Surprises & Discoveries

The existing `noncooperative_async_task_is_aborted_and_reported` returns a pending
future, which yields to Tokio. It cannot prove the behavior of a poll that never
returns. Lifecycle phase coordination is in `crates/batter/src/lifecycle.rs`;
`lifecycle/driver.rs` owns the separately spawned coordinator and report monitor.
Tokio 1.53.1 documents that runtime destruction can wait indefinitely for work
that does not yield. Therefore the child must remain killable even after it
produces its shutdown report.

The first two-worker fixture failed its report milestone after requesting drain
from the blocking task, consistent with Tokio's non-stealable LIFO wake slot.
Moving the request to an OS thread after an entry acknowledgement made report
progress observable. The final eight-entry focused run passed in 5.01 seconds;
the parent killed/reaped the stuck children and removed captures.
An initial compile error used the wrong cleanup-outcome variant; the assertion
now uses the existing `Succeeded` variant, with no production change.

A child can emit expected stall milestones and then panic on an OS thread while
its runtime remains blocked. The parent now rejects captured default panic-hook
diagnostics, with a live subprocess regression proving this path. The blocked
runtime also requires an external two-second observation marker after drain,
so a late startup cannot turn a short observation into evidence that its
one-second timer and configured shutdown allowance were exceeded.

## Decision Log

Use a dedicated integration test executable and invoke its exact child entry
with a scenario environment variable set only on `Command`. Keep fixtures under
`crates/batter/tests/non_yielding/`; no public API or dependency changes are needed.
The parent uses `std::process` and `std::time::Instant`, independently of Tokio.
Capture output in a unique temporary directory so full pipes cannot block child
progress. Guard child ownership through errors and unwinding; explicitly kill
and wait before deleting capture files.

Use an indefinite OS-thread park inside a directly registered async task. This
does not yield its Tokio poll and avoids burning a CPU core. A two-worker runtime
allows the owned driver to report unjoined work; a current-thread runtime cannot
advance its timer/driver once its only runtime thread is blocked. Use explicit
startup/drain evidence rather than sleeps to initiate scenarios. Parent elapsed
limits are test containment, not a new Batter preemption guarantee.

An external OS thread requests drain after task entry, avoiding the LIFO wake
slot described above and allowing the same startup/request protocol on both
runtime types. The current-thread case explicitly polls a one-second timer
before starting its driver and observes two seconds after the drain request.
The parent limit is five seconds with 10 ms polls. Preserve the child's default
panic hook and reject its panic diagnostics even after expected milestones.

## Outcomes & Retrospective

Implementation and the requested runtime evidence are complete. Eight focused
entries pass in 5.01 seconds. Both Rust 1.98.1 and 1.94.0 matrices pass 114 core
entries, 135 workspace entries and two doctests, Clippy and rustdoc. All three
HTTP smoke modes pass. Static package/link checks pass, and a process/capture
scan found no leftover fixture resources. All five Jig targets and the final
backend test matrix pass; the required evidence gate is fresh and passed.
Production code and the lockfile are unchanged. Delivery acceptance is proved;
Beads and Jig's append-only closure events own completion state. No publication,
database work or commit is part of this task.

## Context and Orientation

`Supervisor::start` spawns an owned driver and completion monitor. Registered
critical tasks acknowledge initialization with `mark_started`. Drain stops
admission; forced cancellation follows the drain allowance; abort and observation
follow a further allowance. `ShutdownReport` retains requested aborts, unjoined
direct tasks and skipped cleanup. It is unsuccessful after uncertain termination.
Joining an OS child after killing it says nothing about application finalization.

## Plan of Work

First add synchronous parent tests in `crates/batter/tests/non_yielding.rs` with
a child entry filtered by `--exact`. A child without the scenario environment
does no work. The cooperative case must observe forced cancellation, direct
future destruction, successful dependency cleanup and normal runtime exit. The
two-worker blocking case must report exactly the blocked task as abort-requested
and unjoined, retain no invented task completion, skip the registered dependency
without invoking it, and remain stuck during runtime destruction until killed.
The current-thread case must record entry and a real drain request but no report,
cleanup or successful runtime exit before the independent watchdog kills it.

Next verify the watchdog's failure behavior: a child failure retains diagnostics,
a timeout without required milestones fails classification, and ownership cleanup
kills/reaps children and removes temporary files even on a parent error/unwind.
Do not accept timeout alone as evidence of expected non-preemption.

Finally update the relevant process contract in `docs/guarantees.md`, capability
row in `docs/status.md`, test inventory/commands in `docs/testing.md`, pinned
primary references in `docs/references.md` and executed results in
`docs/validation.md`. Leave historical validation records intact.

## Concrete Steps and Validation

From `/home/aa/Documents/batter`, run:

    cargo test -p batter --test non_yielding --locked -- --nocapture
    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
    cargo build -p batter-axum --example http_service --locked
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
    python3 scripts/check_package.py
    scripts/jig work check --plan-id plan_01M205HP71C12K7EQ44H0JFW7G
    scripts/jig work evidence --plan-id plan_01M205HP71C12K7EQ44H0JFW7G --json
    scripts/jig work gates --plan-id plan_01M205HP71C12K7EQ44H0JFW7G --json
    scripts/jig check test

Every applicable command must exit zero. Record actual test counts, toolchain,
platform and Cargo.lock hash. Capture local logs under `.agent/tmp/batter-fvz/`.
Local evidence does not establish hosted CI, other operating systems, hidden
descendant termination or database behavior.

## Idempotence and Recovery

Re-running the tests creates fresh child processes and temporary capture paths.
No external services or persisted application state are involved. Preserve the
original semantic assertions if a test fails; inspect captured stage evidence
and repair the fixture or runtime bug. Do not add self-release to the blocked
task or replace the OS watchdog with a Tokio timer. Refresh the Beads export
after status changes; do not commit or publish.

## Interfaces and Dependencies

All additions are private integration-test code using existing Tokio and standard
library facilities. `batter-test-support` remains a generic leaf and does not
own these composition fixtures. The existing production shutdown/report API and
lockfile stay intact unless execution reveals a concrete bug requiring repair.

Revised after execution: retain the external request-thread correction, elapsed
observation and panic-failure controls, the passing runtime matrix, and the
passing repository gates/final backend check. This plan is execution evidence
for `batter-fvz`; it does not create follow-up delivery scope.
