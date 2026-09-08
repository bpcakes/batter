# Make escalation assertions independent of worker latency

This living ExecPlan follows `.agent/PLANS.md`; Bead `batter-953` owns scope.

## Purpose

Preserve exact cooperative shutdown proofs without assuming a task runs within
25 ms of a signal. The live two/four-worker corpus must reconcile each legitimate
result with the complete report and conservative cleanup decision. Production
behavior and shutdown budgets do not change. Prepare a verified worktree for the
separately requested Claude/Cursor review.

## Progress

- [x] Confirmed the over-constrained live assertions and pinned Tokio 1.53.1 clock semantics.
- [x] Add controlled-clock success and late-completion regressions; demonstrate the old oracle rejects a legitimate abort.
- [x] Reconcile live success/abort outcomes and update contracts/status/references.
- [x] Run both Rust matrices, corpus/mutation/HTTP checks and Jig gates; record evidence and close implementation.

## Surprises & Discoveries

Modes 0/1 wait for drain/cancellation and then yield, while independent real-time
25 ms phase deadlines continue. The new delayed-completion regression failed
the former success assertion; both controlled regressions pass after correction. Live success and the exact forced-cancellation flag
are scheduling-dependent. Tokio paused time requires a current-thread
runtime and advances to the next timer only when runnable work is exhausted.
Pinned Tokio cancellation semantics permit a final poll to complete or panic
after an abort request; retain that request and its
conservative cleanup decision. A preliminary verification run is superseded by
final source validation after this additional oracle correction.

## Decision Log

Keep all four modes in live multi-threaded exploration. Pass a test-owned
completion future to the affected task so regressions can delay it explicitly.
On successful completion, require the exact completed count. Cleanup succeeds
only without an abort request; a racing recorded request still skips cleanup;
on termination, require the named abort, retained JoinError and skipped cleanup.
Preserve exact cooperative-success assertions in separate paused-clock tests.
One regression stalls the real thread beyond the phase allowance without moving
Tokio time; another sleeps past the phase using Tokio time and must reconcile
an actual abort. Do not enlarge the phase allowances or accept arbitrary errors.

## Outcomes & Retrospective

Final source validation passed on both Rust toolchains: 353 entries each, two
controlled regressions, 20 process controls, six corpora, both mutation challenges
and all HTTP modes. The CPU-contention run passed all 11 scheduling entries.
The required Jig profile passed all five targets with fresh evidence; the
final explicit backend check passed. The Bead is closed. Implementation is
complete and the worktree is prepared for the separately requested review. macOS and Python 3.9 execution remain unverified.

## Context and Work

From `/home/aa/Documents/batter`, preserve existing work against Git baseline
`5db16f6f18fd918d3d3558a68843120bf9a13b79`. Edit
`crates/batter/tests/scheduling/{escalation,families}.rs` and the parent
`crates/batter/tests/scheduling.rs`. Update `docs/{guarantees,status,testing,
references,validation}.md`. No public API, dependency or migration is needed.
The generator still consumes the same choices and runs the same live tasks;
the corrected oracle accepts only contract-permitted timing outcomes.

## Validation and Acceptance

First run the late-completion regression against the old oracle and require its
success assertion to fail. After correction, both paused-clock regressions and
the live corpora must pass. Run `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`, three fresh corpora per worker
count, both capacity mutation challenges, and HTTP smoke in default/SIGINT/deadline
modes after rebuilding its example. Record commands, source hashes, timings,
counts and limits under a fresh `validation/local/` directory and in validation
documentation. Pass Jig work check/evidence/gates and final
`scripts/jig check test`, then close the Bead and this implementation plan.

## Recovery and Review

Investigate failures rather than retrying until green. Resume live command
handles; preserve historical evidence. Do not commit or publish. After all
mutations, run comprehensive-review with the existing `cursor,claude` selection,
independent context-free forwarders and identical complete scope fingerprints.
The new Rust test files are staged so reviewers capture full patches within
their limits. Jig file-budget rejected the initial intent-to-add entries; normal
staging supplied stable index contents and the standalone gate passed. No source
or budget policy changed for this correction.
