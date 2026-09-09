# Review follow-up: error exit policy and subprocess observation

Owning Bead: batter-xzq. Exact Git baseline: fbe77addaafc8709c95d7ecf4982dd3ceea3f41d, with pre-existing uncommitted work preserved.

## Progress

- Researched Rust Termination and lint expectations, rustdoc compile-fail semantics, and Python/Rust subprocess ownership against primary references.
- Documented an explicit ExitCode boundary without changing report Error, Debug, source identity or public APIs.
- Added exact-lint controls for raw reports and an unknown-error no-formatting exit regression.
- Reused scripts/scheduling_process.py for SQLx smoke readiness and configuration subprocess observation; added real-child protocol and watchdog controls.
- Focused checks and both raw-report mutation controls passed. Both Rust matrices passed 469 Rust executions plus eleven Python controls each, with three explicitly ignored live database tests. All five HTTP smokes passed per toolchain. All five Jig gates and the final scripts/jig check test passed. Final evidence/gates report passed and fresh after closing/syncing batter-xzq.

## Surprises & Discoveries

Result-returning main prints errors through Debug, irrespective of sanitized Display. A compile_fail doctest accepts unrelated compiler errors. The existing private Unix process owner already implements bounded capture and cleanup, so a second owner would expand the bug surface. Popen.send_signal polls first; readiness signalling instead preserves the owned unreaped child identity until EOF or group cleanup.

## Decision Log

Preserve concrete internal failures and let applications choose exit output. Require exact unused_must_use lint expectations alongside pedagogical compile-fail examples. Add only a readiness phase to the existing process owner; keep SQLx fixture policy in its example. Capture overflow, missing readiness, incomplete cleanup and nonzero exit fail the smoke. Synthetic protocol controls do not prove database closure. Add Python dependencies to Jig test inputs and execute controls in both verification entrypoints.

## Outcomes & Retrospective

Implementation, both Rust/HTTP validation matrices, all five Jig gates, and the final backend check are complete. Final evidence/gates report passed and fresh. Bead batter-xzq is closed. Final command results and mutation evidence are recorded in docs/validation.md.

## Execution and validation

Run python3 scripts/test_smoke_postgres.py -v and the shared process-owner controls. In a disposable snapshot, run cargo check -p batter --test process_ownership --locked once unchanged and once with each raw report must_use removed; only the latter two should fail for unfulfilled expectations at the raw-report expressions.

Run bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh. Build the HTTP example separately with each toolchain and execute all five profiles in docs/testing.md. Then record validation, close/sync batter-xzq, run scripts/jig work check, inspect evidence/gates, run the final scripts/jig check test, and finish this plan after successful current receipts. Repair failures without weakening semantics. Do not mutate repository inputs during Jig checks.

## Recovery and compatibility

No dependency, persisted-state or public-API cutover is required. All edits stay uncommitted; preserve prior work. The original worktree snapshot is /tmp/batter-review-followup-before-1a26rx2z. Mutation snapshots and logs are disposable; never remove must_use in the working checkout. No live PostgreSQL, macOS or hosted CI result is inferred from local Linux tests. Publication, deployment and commits are outside this task.

Final successful gate run: run_01M23FFYEEQVXJ2HQZE4W4NW8G. Jig marks all target receipts stale after a tracked documentation/tracker update; legacy partial refresh rejected native file-budget, and target partial refresh with plan-id rejected its prepared context. The supported full work-check path passed afterward. No harness policy or semantic tests were relaxed. The final source and lockfile are unchanged from both successful Rust matrices; only validation/tracker records were finalized afterward.
