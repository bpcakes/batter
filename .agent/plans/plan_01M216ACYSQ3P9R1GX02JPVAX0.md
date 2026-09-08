# Retain fixture ownership and budget every phase

This living ExecPlan follows `.agent/PLANS.md`; Bead `batter-953` owns scope.

## Purpose

Make the test harness preserve the same lifetime distinctions it checks in the
library. The unjoined fixture must publish its report before the watchdog kills
its deliberately blocked runtime. The separate pipe writer must remain a direct
child of the test harness until cleanup signals and reaps it. Production code,
public APIs, dependencies and platform scope remain unchanged.

## Progress

- [x] Revalidate current findings and research Python/Unix process ownership.
- [x] Add a delayed-start regression, show the current three-second deadline fails, and derive a dedicated unjoined deadline.
- [x] Replace the orphan/PID-file fixture with a directly owned writer and cover already-exited and exceptional cleanup.
- [x] Update contracts, implemented status, references and validation; run both Rust matrices, corpus/mutation/HTTP checks and Jig gates.

## Surprises & Discoveries

A two-second startup probe consumes the current watchdog before the unjoined
fixture finishes its mandatory drain/cancel/abort-reap phases. Seven seconds
retained the required report checkpoint, confirming a harness budget defect.
The former escaped writer outlived its direct parent and was later signalled
using only a PID file. Linux and Apple wait documentation confirm that waiting
releases child resources; a directly owned, unreaped child retains its identity.
Python accepts an existing pipe descriptor as stdout, allowing two independently
owned children to share a pipe without orphaning either one.

## Decision Log

Give unjoined fixtures startup slack plus the complete five-second case allowance
plus the existing three-second hang-observation allowance. Keep the immediate
stuck fixture's existing contract. Add the replayable `unjoined-delayed-start`
negative fixture, delaying runtime creation by two seconds; its parent must still
observe the full unjoined report before SIGKILL. First run it with the former
three-second budget and require the missing-checkpoint assertion to fail.

Construct the escaped writer as a direct Popen child in its own session. Pass a
test-owned pipe to it and the observed child; close the test's write descriptor
before observation. Retain the writer's Popen through every success/exception
path, use the shared no-post-reap group-signal guard, then wait with a bounded
timeout. Eliminate fork/orphan/PID-file cleanup. Preserve separate facts for
observed-child exit, pipe EOF, escaped-writer lifetime and final cleanup.

## Outcomes & Retrospective

The old deadline failed the delayed-start regression with missing report evidence
after 3.001 seconds. The derived twelve-second deadline passes without changing
report or exit assertions. All three focused writer ownership controls pass,
including cleanup after an observation exception and no signal after reaping.
Both Rust matrices passed 355 entries each. The 21 Python controls passed normally
and optimized; all six fresh corpora, both mutation challenges and three rebuilt
HTTP smoke modes passed. The two-CPU contention run passed all twelve scheduling
entries. Source hashes match across matrix execution and the current source.
All five required Jig checks passed with fresh evidence, and the explicit final
`scripts/jig check test` passed 355 entries. Logs and hashes are retained under
`validation/local/batter-953-fixture-lifetimes/`; `docs/validation.md` records
commands, counts and limitations. Both demonstrated harness defects are fixed.
macOS/hosted and Python 3.9 execution remain unverified. No production API,
dependency, commit or publication change was made.

## Context and Work

Work from `/home/aa/Documents/batter` against Git baseline
`5db16f6f18fd918d3d3558a68843120bf9a13b79`. Preserve existing staged/unstaged work
and unrelated Beads edits. Main files are `crates/batter/tests/scheduling.rs`,
`scheduling/{launch,profile}.rs`, `scripts/{stress_scheduling,
test_scheduling_process}.py` and `docs/{testing,guarantees,status,references,
validation}.md`. All helpers remain test-local. No new public API or dependency
is needed. Research is recorded against Python 3.12.3 implementation and primary
Python 3.12, Linux and Apple process/pipe documentation.

## Milestones

The deadline milestone adds `fixture_watchdog` in
`crates/batter/tests/scheduling.rs`, deriving the unjoined allowance from startup,
`support::CASE_LIMIT` and hang observation. The delayed fixture is authorized by
`scheduling/launch.rs`, sleeps before runtime creation in `scheduling/profile.rs`,
and remains replayable through `scripts/stress_scheduling.py`. Run
`cargo test -p batter --test scheduling delayed_unjoined_start_preserves_report_evidence --locked -- --exact --nocapture`.
The old three-second watchdog must fail for missing report evidence; the fixed
parent must pass after observing the same report checkpoint and SIGKILL.

The ownership milestone replaces the orphan/PID-file path in
`scripts/test_scheduling_process.py` with `owned_pipe_writer` and bounded
`stop_owned_fixture`. Run
`python3 -m unittest discover -s scripts -p test_scheduling_process.py -v`.
Sixteen controls must pass, including incomplete EOF while the outside-group
writer still runs, cleanup after an observation exception and no signal after
reaping. The Cargo binary adds five launch controls. Finally execute the complete
matrix below and retain source hashes with its command logs.

## Validation and Acceptance

Run the delayed-start Rust test first against the old budget and then the fix.
The latter must retain the report, pending receipt, skipped cleanup and watchdog
SIGKILL. Run Python ownership controls with normal and optimized Python. Verify
cleanup after a body error and after an already-waited writer; no signal may
follow reaping. Run `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`, three fresh corpora per worker
count, both capacity mutation challenges, and rebuilt HTTP smoke in SIGTERM,
SIGINT and deadline modes. Record command logs and final hashes under a fresh
`validation/local/` directory. Finish with Jig work checks/evidence/gates and
`scripts/jig check test`. Close the Bead and implementation plan only with the
required evidence; commit and publication are not authorized.

## Recovery

Keep failed probes as negative evidence; never retry to green. Resume confirmed
live command handles and do not change source during a verification matrix.
The earlier review was invalidated by unrelated tracker writes; any further
same-scope review must use a checked isolated snapshot with matching fingerprints.

Revision: all implementation and validation milestones are complete. The old
deadline's failure is preserved as negative evidence; full matrices, controls,
mutation challenges, HTTP smoke and repository gates passed on the final source.
Retaining a child handle and budgeting each lifetime phase removed both defects
without changing the production model or weakening report assertions.
