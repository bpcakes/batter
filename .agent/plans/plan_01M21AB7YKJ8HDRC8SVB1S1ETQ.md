# Preserve process ownership through SIGINT

This ExecPlan follows `.agent/PLANS.md`. Bead `batter-953` owns the scope.

## Purpose

A user interrupt during subprocess cleanup must retain diagnostics, bounded
termination/reaping, and descriptor closure. The current helper catches
KeyboardInterrupt during observation but runs cleanup outside that boundary.
Python can raise a signal exception between arbitrary instructions, so extending
one catch block does not protect the complete ownership interval. Replace that
mechanism with scoped, non-raising SIGINT observation. Correct stale duplicated
process-control counts without adding a documentation-generation system.

## Progress

- [x] Revalidate current code and research Python signal, wait and selector semantics.
- [x] Add real-SIGINT regressions and demonstrate the cleanup failure before fixing it.
- [x] Implement scoped interruption, unconditional resource closure and documentation corrections.
- [x] Execute focused controls, both Rust matrices, repeated corpora and mutation/HTTP checks; record final source evidence.
- [x] Complete required Jig gates and final backend check.
- [x] Confirm fresh receipts and close the task.

## Surprises & Discoveries

The last review reproduced one SIGINT during `_settle` escaping its caller's
finally block, leaving cached child status unobserved and stderr open. Python
3.12 documentation explicitly warns that signal-raised exceptions can interrupt
context entry, exit and library internals. Installed Python 3.12.3 subprocess code
also treats KeyboardInterrupt as interactive abandonment, not this helper's
ownership protocol. Selector close is explicit; pipe EOF is separate from reaping.
All current helper call sites are standalone, main-thread tooling.

## Decision Log

Install a private non-raising SIGINT recorder before acquiring a child; use it as
a stop request in normal observation but allow cleanup to use its original
absolute deadline. Restore the previous default SIGINT handler after all owned
resources have been released. Reject non-main-thread use or a non-default SIGINT
handler before child creation, as already done for incompatible SIGCHLD status
ownership. These are private tool requirements, not new Batter library APIs.
Retain explicit KeyboardInterrupt failure handling for deliberately raised
exceptions in setup callbacks; do not use it as the OS signal protocol.

Resource closure must run in a nested finally even if settlement fails. Check
selector and each stream independently, preserving I/O error categories. Keep
kill-before-reap group identity, output caps and incomplete-EOF reporting intact.
No signal should restart a cleanup deadline. Test signal-handler restoration on
normal exit, setup failure and cleanup interruption, plus rejection before launch.

Remove the repeated numeric standalone counts in the Jig-command instructions;
keep executable discovery and the main coverage description authoritative.

## Outcomes & Retrospective

Both cleanup subcases rejected the old implementation because SIGINT escaped
without an outcome. Four focused controls now pass: cleanup with one/repeated
signals, setup/observation/close interruption, signal-owner rejection, and callback
failure with handler restoration. A close-failure control also retains both
selector and stream errors while releasing resources. The full matrix now passes on both Rust toolchains (359 entries each), normally
and optimized with 26 Python controls, six fresh corpora, the two-CPU contention
run, both mutation challenges and all three rebuilt HTTP smoke modes. All 20
source hashes and all 24 mutation replay records were audited. All five required Jig targets and the final backend check now pass (359 entries,
zero failures/ignored). The verify gate was fresh with no missing or failed receipts. The work plan
and Bead are closed; changes remain uncommitted. No production source/API or dependency changes are needed.
Linux evidence cannot establish macOS, hosted or Python 3.9 execution. No commit
or publication is authorized.

## Context and Milestones

Work in `/home/aa/Documents/batter` at baseline
`5db16f6f18fd918d3d3558a68843120bf9a13b79`. Preserve all existing staged,
unstaged, untracked and unrelated Beads changes. `scripts/scheduling_process.py`
owns capture, group identity, waits and evidence. `scripts/test_scheduling_process.py`
owns the real-process controls and Cargo-supplied Rust launch tests. Caller tools
are `scripts/stress_scheduling.py` and `scripts/check_scheduling_mutation.py`.

First add a real SIGINT regression during cleanup with an outside-group pipe
writer that remains directly owned by the test. Retain the selector and child
objects so garbage collection cannot hide leaked resources. Use controlled
observation checkpoints to deliver one and repeated signals; require the original
cleanup deadline, preserved checkpoint/error flags, reaping and closed descriptors.
Retain a before-fix failure log. Then implement the signal ownership boundary and
cover normal observation interruption, acquisition/closure interruption, restoration
and precondition rejection. Keep fixture cleanup active when assertions fail.

## Validation and Acceptance

Run focused Python controls normally and optimized. Run `bash scripts/verify.sh`
and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`, three fresh full corpora per
worker count, the two-CPU contention target, both capacity mutation challenges,
and rebuilt HTTP smoke in default, `--signal SIGINT` and `--deadline` modes.
Retain exact commands, before/after logs, environment and source hashes under
ignored `validation/local/batter-953-sigint-ownership/`. Update guarantees, status,
testing, references and validation with observed facts and explicit limitations.

Finish documentation and plan outcomes before final work-gate receipts, because
Jig invalidates them after documentation changes. Run required `scripts/jig work
check`, final `scripts/jig check test`, evidence/gates and work finish. Close the
Bead after work finish and flush its export. Do not repeat passing tests unless
source changes or failed/stale evidence requires it.

## Recovery

Research new semantic questions before another implementation round. Never retry
failed tests to green. Resume live process handles and preserve failure logs.
Do not use a PID file or unowned process to deliver signals; the test owns and
reaps every fixture. Do not install library-global handlers or add a process
framework; the SIGINT scope belongs only to this private synchronous tool helper.
