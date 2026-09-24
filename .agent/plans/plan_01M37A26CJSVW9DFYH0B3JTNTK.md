Bead batter-f3ky. Reproduce PR 13 merged-tree CI errors, merge master 67f258d, migrate policy unit/live controls to OperationOwner, preserve assertions, run both toolchain verification and HTTP smokes plus the live control, then push and observe hosted CI.

## Repair

CI run 35867025062 compiled the PR merge with master, whose operation-authority
cutover removed root construction and cancellation from `OperationContext`.
Merged master 67f258d, reproduced the failure, and migrated the policy unit/live
callbacks to retained `OperationOwner` references. The wrappers still receive
only contexts, and all mapped-error, native-cause, provisional-output, preflight,
and unpolled-future assertions remain intact. Repair commit: 5529f5a.

## Validation history

The default-toolchain all-target check, two policy-context unit tests, and live
commit-cancellation test passed. The live case exercises both wrapper variants.

Two full Rust 1.98.1 verification attempts then failed in unchanged native
Runledger tests. The first hit the two worker-loop promotion timeouts; the exact
workspace-built worker-loop binary subsequently passed all eight tests unchanged.
The second observed `Leased` instead of `Succeeded` in the heartbeat/progress
row-lock test; that exact binary subsequently passed the case unchanged against
PostgreSQL 18.6. No load-related cause or runtime defect has been confirmed.
Full diagnostics remain under `/tmp/batter-f3ky-validation`, with the standalone
diagnostic logs under `/tmp/batter-f3ky-*.log`; Bead comments retain the failures.
Remaining local validation uses `RUST_TEST_THREADS=4`, preserving assertions and
time limits. Hosted CI retains its configured concurrency.

Both local verification scripts subsequently passed: Rust 1.98.1 in 528.47
seconds and Rust 1.94.0 in 521.68 seconds. Each toolchain also passed the complete
all-target compilation check, both policy-context unit tests, the live
commit-cancellation case, and all five HTTP smoke profiles.

Hosted Rust verification run 35875284194 passed all seven jobs on repair commit
5529f5a: Linux verification on 1.94.0, 1.98.1 and stable; macOS subprocess/HTTP
checks on both pinned toolchains; and native Runlimit PostgreSQL checks on both
pinned toolchains. Jig integration run 35875284340 also passed. These are observed
hosted results, distinct from the local verification recorded above.

Final Jig `work check` passed in 485.64 seconds. Read-only evidence and gate
inspection confirmed fresh passing receipts for all five required targets,
including `api:test` receipt `receipt_01M37D0G9MKRW3WHRHZ9PXHBXY`.
The disposable PostgreSQL container was stopped, and Bead `batter-f3ky` was closed
with both the successful results and initial local failures retained.
