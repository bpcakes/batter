# Reconcile closure outcomes and clean up partial capture setup

This ExecPlan follows `.agent/PLANS.md`; Bead `batter-953` owns delivery scope.

## Purpose

Make the scheduling harness distinguish an admission guarantee from a deadline's
legitimate task termination, and release operating-system selector resources even
when capture initialization raises. The two review findings concern test/tool
behavior; no production API, dependency or platform change is needed.

## Progress

- [x] Revalidate the two findings and research pinned Tokio/Python semantics.
- [x] Add focused regressions and demonstrate their failure on the current code.
- [x] Fix partial capture ownership and reconcile live closure outcomes without weakening admission or report assertions.
- [x] Update contracts/status/references/validation and complete both Rust matrices, corpus/mutation/HTTP checks and Jig gates.

## Surprises & Discoveries

Review reproduced a closure failure on an unchanged seed-17 binary: delaying
forced-closure observation by 1.25 seconds yielded a legitimate `Terminated`
receipt that the test unwrapped. The case allows five seconds but cancellation
allows one. Separately, ten injected capture-construction failures left ten
selector descriptors open until cyclic garbage collection, despite reaping all
children. Python's selector contract requires explicit close. Tokio 1.53.1 permits
completion racing an abort request; every recorded request still requires skipped
dependent cleanup. Paused time auto-advances idle timers, enabling exact deadline
regressions without requiring fast operating-system scheduling.

## Decision Log

Keep the post-closure admission rejection strict. Reconcile each admitted parent
and descendant receipt with named completed/aborted report outcomes, and preserve
the original task-failure identity when that mode closes admission. Treat abort
requests racing successful completion conservatively. Register a dependent
finalizer so the scenario checks actual cleanup versus explicit skipping. Add a
test-local pause after closure observation to exercise deadline expiry; keep
ordinary generated choices unchanged.

Make `Capture.__init__` close its already-created selector on any exception before
rethrowing it. Use real child processes and real selectors in the regression,
injecting setup failures and interruption; retain the selector object so garbage
collection cannot conceal missing close. Verify the underlying selector is closed
and child kill/reap/error reporting still occurs. No resource-management framework
or new dependency is needed.

## Outcomes & Retrospective

Both focused regressions rejected the old behavior and pass after the fixes.
The Python control covers four partial setup failures with retained real selectors;
the Rust controls cover both closure causes before and after cancellation expiry.
No production API or dependency change was needed. Assertion helpers keep the
closure actor, receipt accounting and report/error checks within repository lint
limits without suppressions.

Full final evidence is in `docs/validation.md` and ignored
`validation/local/batter-953-closure-capture/`: both Rust matrices pass 359 entries
with no failures or ignored tests; 22 Python controls pass normally and optimized;
six fresh corpora, the two-CPU contention run, both mutation challenges and all
three rebuilt HTTP smoke modes pass. All 20 source hashes match before/after the
matrix and after the final backend check. All five Jig work targets and the final
`scripts/jig check test` pass. Linux/Python 3.12.3 execution does not establish
macOS/hosted or Python 3.9 execution. Commit and publication are not authorized.


## Context and Milestones

Work from `/home/aa/Documents/batter`, baseline
`5db16f6f18fd918d3d3558a68843120bf9a13b79`. Preserve all existing staged/unstaged
changes and unrelated Beads records. `scripts/scheduling_process.py` owns capture
and bounded process observation; `scripts/test_scheduling_process.py` owns its
real-process controls. `crates/batter/tests/scheduling/closure_races.rs` owns the
live closure race, `scheduling.rs` its discoverable regressions, and `families.rs`
the seeded invocation. Read the current report and receipt contracts in
`crates/batter/src/lifecycle.rs` and its `process.rs` before editing the oracle.

First add the two focused regressions, run them against the existing behavior and
retain failures for the intended assertions. Then add the cleanup guard and the
receipt/report reconciliation. Exercise both force- and task-failure closure,
normal completion and completion delayed beyond cancellation. Assertions must
continue checking exact task names/counts, error categories and cleanup decisions;
no unexpected panic, business failure or unjoined work becomes success.

## Validation and Acceptance

Run the focused Rust closure tests and the Python capture-construction control
before and after the fixes. Run `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`, three fresh full corpora per worker
count, both capacity mutation challenges and rebuilt HTTP smoke in default,
`--signal SIGINT` and `--deadline` modes. Run Python controls normally and with
`PYTHONOPTIMIZE=1`. Retain command logs, negative controls, environment and final
source hashes under a new ignored `validation/local/` directory. Update
`docs/guarantees.md`, `docs/status.md`, `docs/testing.md`, `docs/references.md` and
`docs/validation.md`. Finish with required Jig work checks/evidence/gates and
`scripts/jig check test`, close the work plan while its receipts are fresh, then
close the Bead and flush its export.

## Recovery

Do not retry failures to green or change source during the final matrix. Resume
confirmed live command handles. Preserve unexpected failures as evidence and
research any new semantic question before another implementation round. Keep
all fixtures, selectors and child processes explicitly owned through cleanup.
