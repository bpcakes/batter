Owning Bead: batter-a63. Baseline: e518b6b52a88a460edfb6f7c41107e71932b6c70.

## Progress

- Preserved the reviewed uncommitted changes in a recovery stash, fetched origin/master, and fast-forwarded to the two newer upstream commits.
- Reconciled the process capture and matrix runner, typed SQLx errors, source/exit tests, tracker export, validation history and append-only Jig state.
- Replaced the callback-timing assertion with real signal requests and runnable/SIGSTOP child controls. Added parent interruption before readiness and trailing-field rejection.
- Focused smoke controls (13), runner controls (22) and SQLx exits passed. Both full Rust matrices passed 473 Rust executions plus 35 Python controls, with three live database tests explicitly ignored per matrix. All five HTTP profiles passed on each toolchain. All five Jig gates passed in run_01M23K01N3VCX7RWSMGEHSKYWK; evidence/gates confirmed fresh, and the final scripts/jig check test passed. The verified changes are ready for the authorized commit/push.

## Surprises & Discoveries

The remote advanced after the reviewed snapshot. It adds a bounded concurrent matrix runner with rolling failure-output tails and redacted SQLx process/startup/shutdown errors. Both overlap local work. Neither history may be discarded. Beads additive reconciliation imported four remote issues while retaining seven local-only issues.

## Decision Log

Keep the upstream matrix runner and add the SQLx smoke controls to its first phase. Share Capture.retain across both ordinary reads and readiness reads, preserving tail output for parallel commands and prefix-only capture for protocol checks. Keep typed error wrappers and the exact Error: process failed diagnostic, while preserving explicit ExitCode handling and bounded configuration subprocesses. Store all upstream retention/redaction assertions in a sibling test module. The deadline control records an actual os.kill call and uses SIGSTOP only on the exclusively owned unreaped child; no handler scheduling is required.

## Outcomes & Retrospective

Both toolchain matrices, all ten HTTP smokes, all five Jig gates and the final backend test passed. Final evidence/gates matched the current worktree. Bead batter-a63 is closed. Git delivery follows this work-record close; the commit and remote branch provide that evidence. The user explicitly authorized committing and pushing the reviewed work plus this correction. No deployment or package publication is included.

## Execution and validation

Run python3 scripts/test_smoke_postgres.py -v and cargo test -p batter-example-postgres-lifecycle --locked for focused checks. Run bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh; each must pass formatting, all portable test matrices, Clippy and warning-denied rustdoc. Build the HTTP example separately on each toolchain and execute the five docs/testing.md profiles. Record executed evidence and close/sync batter-a63. Run scripts/jig work check for this plan, then inspect evidence and gates. Preserve a final successful api:test receipt, finish the plan and commit the complete reconciled tree. Fetch and push normally; never force push. Verify remote HEAD and a clean checkout.

## Recovery and compatibility

Recovery stash f8d818f831b639033eee279a7270d10544ea30fe retains all pre-integration work and remains available. No dependency graph or persisted application state changes are needed. Keep generated Cargo.lock untouched. Do not replace append-only state history or tracker exports with one branch's copy. If origin advances again, integrate before pushing without losing either side. Live PostgreSQL, macOS and hosted CI require separate executed evidence; no new runs are inferred here.

The first integration check caught a redundant report dereference, and the first full matrix caught the old two-command prerequisite assumption plus a smoke fixture running inside the output-bound control. Corrected the source borrow and fixtures without relaxing output limits or retained failure assertions. Both final matrices and the Jig profile passed after those corrections.
