# Make scheduling subprocess failures bounded and observable

This ExecPlan follows `.agent/PLANS.md` and belongs to reopened Bead batter-953. The Git baseline is 5db16f6f18fd918d3d3558a68843120bf9a13b79. The existing uncommitted scheduling implementation, documentation and tracker records are the starting worktree; preserve them. No commit or publication is authorized.

## Purpose / Big Picture

Ordinary Cargo discovery must never activate a blocking child through an ambient environment variable. Every started test/compiler process must have an explicit owner, bounded observation and cleanup, and retained output on every failure path. Fix the shared subprocess protocol, without changing Batter production APIs or widening the existing non-yielding suite.

## Progress

- [x] 2026-09-08: Confirm four review failures against current sources and probes; research Python 3.12 subprocess, process-group and pipe semantics and Cargo lock/config policy.
- [x] 2026-09-08: Implement explicit child launch, PID validation, bounded startup and ordered deadlines.
- [x] 2026-09-08: Share bounded process outcomes across profile/build/version commands; consume replay results directly and retain logs before classification.
- [x] 2026-09-08: All 18 process/launch controls pass normally and optimized; the original review probes established the prior failures.
- [x] 2026-09-08: Recheck open questions, document contracts/references/results; both Rust matrices, six fresh scheduling corpora, both mutation challenges and three HTTP modes passed.
- [x] 2026-09-08: Required Jig verify gates passed with fresh evidence; final explicit backend test passed in 40.234 seconds.
- [x] 2026-09-08: Complete the source/evidence audit, close batter-953, refresh final gates, pass the final backend check and finish the work plan; changes remain uncommitted.

## Surprises & Discoveries

The earlier EOF/reap probe raised after 5.106 seconds and discarded its checkpoint. The overflow fixture produced one WATCHDOG_RESULT marker but zero standalone summary lines. Exporting BATTER_SCHEDULING_CHILD changed inert discovery from exit 0 to exit 101 on EOF. The initial launch read happens before the emergency thread is armed, so an open stdin can hang indefinitely. Python Popen context exit calls wait without a timeout; a finally block containing a bounded wait does not make that context exit bounded. TimeoutExpired carries partial output but the mutation tool currently drops it before writing its logs. The existing two-core eight-test probe passed; no production concurrency defect or timing flake was demonstrated.

## Decision Log

Treat these as a test-harness ownership/result-model defect with a local framing omission, not a Batter runtime abstraction defect. Add a small scheduling-specific Python process owner used by the runner and mutation build helper. Represent expected launch, deadline, reap and pipe failures as data with bounded output; do not use Popen context exit or an unbounded communicate/wait. Separate child exit from pipe EOF, use a private Unix process group for owned compiler/test descendants, and explicitly report incomplete observation rather than claiming cleanup of escaped descendants.

Authorize Rust child mode by exact argv and a bounded, PID-bound launch record; ignore ambient scenario flags as authority. Arm the emergency protection before reading stdin and bound startup separately. Keep parent-stdin EOF as fallback termination. Cap the external watchdog at 140 seconds, retain a five-second observation/reap allowance and the 149-second child emergency backstop. This gives ordered containment deadlines within the original task ceiling. Preserve inherited build/runtime environment where needed; an environment allowlist is not a substitute for launch authorization.

Let the mutation checker call the same structured scheduling function directly rather than parse its human output. Render a standalone final JSON summary at the CLI boundary and preserve bounded compiler output plus failure metadata before classifying the oracle. Copy the repository Cargo configuration for fidelity; locked compatible dependencies remain unchanged.

## Outcomes & Retrospective

The first implementation round passed all nine Cargo entries and 16 Python controls. A second research/inspection round confirmed that CPython can synthesize status zero with ignored SIGCHLD: a live child exiting 7 was accepted. A new guard refuses non-default SIGCHLD without mutation, and two added controls cover this and partial evidence after close failure. The final matrix and all focused controls passed. Each toolchain executed 349 successful Rust entries (163 core, 184 workspace, two docs), and both rejected six capacity mutants while accepting six originals. All six fresh corpus runs passed and HTTP SIGTERM/SIGINT/deadline checks passed. Required Jig gates passed with fresh evidence, and the final explicit backend test passed in 40.234 seconds. The final source/evidence audit matches the implemented fixes and all original scheduling requirements; batter-953 is closed and the work plan finished successfully after refreshing closing evidence and passing the final backend check.

## Context and Orientation

`scripts/stress_scheduling.py` launches the Rust fixture and currently mixes pipe capture, waits, cleanup and summary formatting. `scripts/check_scheduling_mutation.py` copies and alters process capacity only in a temporary workspace; its subprocess exceptions currently bypass evidence writes. `crates/batter/tests/scheduling.rs` is the Cargo entry; `scheduling/profile.rs` currently trusts an environment variable and blocks before arming fallback exit. The existing `tests/non_yielding/launch.rs` demonstrates exact-argument/PID-bound authorization, but retains its own scenario-specific protocol.

## Plan of Work

First make a dedicated scheduling launch module and real-child controls for inert discovery, malformed/missing authorization, parent disappearance and deadline ordering. Then add `scripts/scheduling_process.py` for explicit bounded subprocess observation and immutable results, use it in the scheduling runner and mutation checker, and remove text parsing from their internal boundary. Keep captured output capped while continuing pipe draining and distinguish observed status, output EOF, timeout, overflow, and failed cleanup. Add Python regressions with real children that retain pipes, close streams before exit, exceed capture, or time out after output. Use narrowly injected OS errors only for branches real processes cannot deterministically provoke.

Before each follow-up implementation round, inspect primary semantics and any new uncertainty. Update `docs/references.md`, `docs/testing.md`, `docs/guarantees.md`, `docs/status.md`, and `docs/validation.md`; keep task scope/status in Beads. Add the Python prerequisite to the verification preflight and ensure the new process-control tests are discoverable in required checks.

## Concrete Steps

Work from /home/aa/Documents/batter. Run focused Python unittest discovery and `cargo test -p batter --test scheduling --locked`. Run three fresh two-worker corpora and three four-worker corpora, then the original/independent-capacity mutation challenge on Rust 1.98.1 and 1.94.0 using new evidence directories. Run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`. Build `cargo build -p batter-axum --example http_service --locked` and run `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` with default, --signal SIGINT and --deadline modes. Run `scripts/jig work check`, inspect evidence/gates, finish backend verification with `scripts/jig check test`, and finish the work plan.

## Validation and Acceptance

An exported child flag leaves normal discovery inert. Exact child argv without a complete valid launch record fails under a native startup deadline rather than reading forever. External timeout and its reap allowance precede emergency exit. Spawn, timeout, pipe and reap failures retain bounded checkpoints and an independently parseable summary. Killing a process group is containment only; escaped pipe owners remain a documented limitation with explicit failure evidence. Mutation build/replay timeouts preserve available logs and cannot count as successful mutant rejection. Both normal corpus and existing controls remain passing; known-bad independent-capacity variants still fail the intended assertion. New controls should fail the original implementation for the reviewed reasons.

## Idempotence and Recovery

Use temporary directories for process fixtures and mutation workspaces, always retain owned process handles and kill/reap them on test failures. Evidence directories are new per run. Keep all .agent/state JSONL records append-only. Do not rewrite the completed earlier plan as though these corrections had already been present. No dependency update, production API, Windows support or live database work is included.

## Interfaces and Dependencies

Use Python 3 standard-library subprocess/selectors/os and Rust std/Tokio already selected in Cargo.lock. No new library package or runtime abstraction is needed. The process helper owns commands only for this scheduling toolchain; its result is test evidence, not an application report serialization format. Preserve concrete application error identity/redaction in the existing Rust families.

Follow-up research round 2026-09-08: Python 3.12.3 source confirms Popen.poll handles ECHILD with returncode 0. A live negative probe printed requested_exit=7, reported_status=0, accepted=true before the guard. Close failures also need to remain outcome data. The process owner now rejects unobservable status ownership before spawning and preserves capture after close errors. Deadline/configuration questions are resolved; no user input is needed.

Gate wiring update: api:test and api:test-locked now include the four scheduling Python files in their inputs, because Cargo invokes that code and Python-only changes must invalidate evidence. The process owner defers type annotations to avoid adding a Python 3.10 requirement; the tooling uses Python 3.9+ features already present in mutation tooling.

Validation update 2026-09-08: Exact evidence is in validation/local/batter-953-containment/commands.json and docs/validation.md. All source hashes match the executed snapshot. Initial contract checks rejected an accidentally broadened file-budget input caused by a textual edit matching a profile entry; it was corrected, and native isolated regeneration exactly matches the final contract. No policy or semantic assertion was weakened.

Completion audit 2026-09-08: The four reviewed failures have explicit protocol/result fixes and executable controls; the further SIGCHLD and close-error paths are tested. Original scheduling counts, capacity mutation rejection, both Rust matrices and HTTP behavior remain passing. Source hashes still match the final executed snapshot. No user input is needed; macOS/hosted execution remains unverified and changes remain uncommitted.

Completed 2026-09-08: Work finish returned success with plan receipt receipt_01M20ZC0AAHJTCBPWKKZ5B7YHY and session receipt receipt_01M20ZC0AWY5E3JQQM596H2JDX. Both the original implementation acceptance and the root-cause correction goal have executed evidence. No pending implementation or required verification remains.
