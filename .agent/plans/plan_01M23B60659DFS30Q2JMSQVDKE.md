# Shorten local verification without removing coverage

Owning Bead: batter-tip. Baseline: fbe77addaafc8709c95d7ecf4982dd3ceea3f41d.
This plan follows .agent/PLANS.md. The user authorized implementation, not commits.

## Progress

- [x] 2026-09-09: Measured local baseline and temporary concurrency experiments.
- [x] 2026-09-09: Implemented shared bounded parallel process execution and 17 failure/coverage controls.
- [x] 2026-09-09: Integrated concurrent Rust matrix and isolated scheduling control shards; all individual assertions/deadlines unchanged.
- [x] 2026-09-09: Updated evidence reuse policy, contracts, implemented status and primary references.
- [x] 2026-09-09: Both Rust toolchains passed 445 Rust executions and 17 runner controls; all 55 Python regressions and all five HTTP profiles passed.
- [x] 2026-09-09: All five final Jig targets passed; evidence/gates report fresh passing receipts. Completion records follow without a duplicate test invocation.

## Surprises & Discoveries

Core and workspace passes enable different dependency features despite batter having
no features itself. Preserve both. Scheduling controls change process-global signal
handlers and mocks; parallel workers must be separate OS processes. Existing process
capture retains exited leaders until output EOF so later group cleanup has a valid
identity. Preserve that rule. Jig already executes independent targets concurrently.

## Decision Log

Use a small Python standard-library runner shared by the matrix and shards, reusing
existing bounded Capture and process outcomes. Observe all launched groups and report
every failure, with scoped SIGINT/SIGTERM cancellation and bounded cleanup. This is a
private Unix test runner, not an application supervision API. Keep all test assertions
and timing windows unchanged. Runtime core/workspace passes overlap; minimal library
compilation precedes them and doctests follow their successful completion.

## Outcomes & Retrospective

Implementation complete. Rust 1.98.1 verify passed in 24.723 seconds and Rust 1.94.0 in 33.576 seconds. Logs and exact timing records are in /tmp/batter-tip-validation. The warm runtime pair now takes 22.305 seconds, preserving all 439 runtime executions plus six doctests. Final Jig run run_01M23BVQK577E8NAB9SG4RMWAE passed all five targets in 24.916 seconds (api:test 24.566 seconds). Both work evidence and work gates confirm fresh passing receipts for unchanged sources. The warm full profile improved from 81.718 to 24.916 seconds, about 69.5%. Baseline warm full gate: 81.7 seconds;
concurrent runtime pair: 39.4 seconds; Python controls: serial 29.1 versus four-process
8.1 seconds. These experiments were macOS-only, not production runner validation.

## Context and implementation

scripts/verify.sh and .jig.toml duplicate a serial four-command matrix. Add
scripts/test_matrix.py as their common entrypoint. scripts/test_scheduling_process.py
contains 24 process controls and six binary-dependent launch controls, selected by
load_tests. Preserve ordinary unittest discovery and serial selection; direct full
script execution will discover and partition all selected IDs into four processes,
validate child completion against assigned IDs, and fail on missing/duplicate results.
Use a shared scripts/parallel_process.py for bounded capture and child settlement.
Add scripts/test_parallel_process.py for overlap, failure aggregation, interruption,
spawn errors, output overflow and retained child identity; add shard selection and
missing-result controls. Update Jig action inputs for every runner/helper dependency
and regenerate or synchronize the matching checked-in contract through Jig.

## Validation and acceptance

Run focused Python regression discovery, full scheduling tests, then bash
scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh sequentially to
avoid compiler artifact contention between toolchains. Build the HTTP example and
run all five smoke profiles from docs/testing.md. Run final Jig work check, evidence
and gates. Record wall times and exact executed outcomes in docs/validation.md;
update docs/testing.md, docs/guarantees.md and docs/status.md. A fresh matching test
receipt satisfies final backend verification without another invocation; do not reuse
across source/configuration/toolchain/environment changes or after unresolved failure.
An ordinary verify.sh run is not automatically a Jig receipt.

## Recovery and boundaries

On child error continue observing its peers and return failure. On interruption stop
new admission, signal owned groups, retain bounded diagnostics, and settle direct
children before restoring signal handlers. Never signal a reaped group leader.
Cancellation and hard termination do not establish cleanup of detached descendants.
Runs are repeatable; preserve append-only Jig records and Cargo-generated lockfile.
No application APIs, dependency upgrades, remote CI changes or publishing are needed.

Revision: implemented the shared owner, discovery-checked shards and matrix entrypoint, then recorded executed matrix/smoke evidence. Final gate receipts will determine completion; no extra duplicate backend test is required.

Final evidence: /tmp/batter-tip-validation/final-jig-check.json, final-evidence.json and final-gates.json. The serial --jobs 1 control path passed all 24 binary-independent controls in 24.023 seconds, explicit selection passed, invalid worker count returned status 2, and agent-map/agent-guides checks passed. No source/configuration changed after the final gate. All 17 runner controls, all 55 discovery tests, both 445-execution Rust matrices and five HTTP profiles passed. Linux/Python 3.9 execution remains unverified. No commit was made.
