# Preserve subprocess identity through cleanup

This living ExecPlan follows `.agent/PLANS.md`; Bead `batter-953` owns delivery.

## Purpose

The scheduling tools must retain ownership of a numeric process-group identifier
until their last signal decision. Failure controls need startup margin while
preserving their exit-status, checkpoint and bounded-termination assertions.
After implementation closes, run the requested independent Claude/Cursor review.

## Progress

- [x] Confirmed the defect and checked Python, Linux and Apple wait semantics.
- [x] Added rejecting ownership controls and deferred reaping through cleanup.
- [x] Added startup margin, delayed-start coverage and the outer watchdog bound.
- [x] Record full validation and pass the repository gates.
- [x] Prepare the completed implementation for the separate comprehensive review.

## Surprises & Discoveries

`Popen.poll()` reaps an exited leader. A surviving output writer can leave its
group; after reaping, the old numeric identifier may be reused. Both new ownership
assertions rejected the prior implementation: one caught a signal after explicit
wait, the other observed cached status 0 before group signalling. All 15 updated
Python process controls passed in 22.448 s. A one-second pre-checkpoint delay fails
under the old 0.3-second budget and passes with the three-second budget.

## Decision Log

Defer polling until output EOF or group termination. `Capture.complete` centralizes
that ordering; cached reaped status independently forbids signalling. This retains
an alive or zombie leader without platform-specific wait APIs. Preserve distinct
child-exit and output-EOF evidence, including escaped writers. Use a three-second
Python control budget and explicit cleanup/startup slack. The outer Rust bound is
3 + 5 + 4 seconds; its three-second lower bound and all semantic assertions remain.

## Outcomes & Retrospective

Twenty controls pass normally and optimized; both Rust matrices pass 349 entries.
Six corpora, both mutation challenges and HTTP smoke modes pass. All five Jig
gates passed and closing receipts were refreshed after the Bead update.
The final `scripts/jig check test` also passed. Production behavior, APIs and
dependencies are unchanged; macOS is unverified. This implementation is ready
for the separately requested review.

## Context and Work

Work from `/home/aa/Documents/batter`, Git baseline
`5db16f6f18fd918d3d3558a68843120bf9a13b79`, preserving the existing dirty worktree.
`scripts/scheduling_process.py` owns process observation and cleanup;
`scripts/test_scheduling_process.py` owns real-child controls;
`crates/batter/tests/scheduling.rs` checks the externally bounded Rust fixture.
Update `docs/{guarantees,status,testing,references,validation}.md` alongside them.
An inherited pipe test records the leader's unreaped status at the group signal;
a separate reaped-child control intercepts signals so no unrelated group is hit.

## Validation and Acceptance

Run Python process controls normally and optimized with the compiled scheduling
binary, then three fresh corpora per worker count and capacity mutation challenges
on Rust 1.98.1 and 1.94.0. Run `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`. Build the HTTP example and run
`scripts/smoke_http.py` in default, SIGINT and deadline modes. Exact commands and
results are retained by `/tmp/verify-batter-953-reap-order.py` under
`validation/local/batter-953-reap-order/`; summarize executed evidence in
`docs/validation.md`. Pass Jig work check/evidence/gates and finish backend checks
with `scripts/jig check test`. Close the Bead and implementation plan before
capturing the comprehensive review's immutable working-tree scope.

## Recovery and Dependencies

Do not retry failed tests until green; investigate each failure. Resume live
command handles before restarting. Use fresh evidence directories and preserve
historical plans/logs. No migrations, new packages, commits or publication are
needed. After closing this implementation, run comprehensive-review with
`--reviewers cursor,claude`; preserve identical complete scope fingerprints and
report both frozen results. Further findings remain review output, not an implied
claim that the change is clean.
