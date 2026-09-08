# Preserve inherited SIGINT policy in scheduling tools

This living ExecPlan follows `.agent/PLANS.md`. Bead `batter-953` owns delivery.

## Purpose

Foreground and non-interactive background invocations of the scheduling tools
must both work. SIGINT ignored by a Unix launcher is policy, not evidence of a
competing signal handler. Preserve ignored SIGINT in the owner and its children;
retain deferred interruption and bounded cleanup when the prior disposition is
the Python or Unix default. Also make the unjoined fixture's elapsed-time promise
executable rather than accepting any eventual SIGKILL.

## Progress

- [x] Reproduce background Cargo failure and research primary signal contracts.
- [x] Add regressions and retain their before-fix failure evidence.
- [x] Correct signal policy, add elapsed-time checks, and update contracts.
- [x] Execute and audit the full required validation matrix.
- [x] Finish all five Jig targets and the final backend check.
- [x] Verify fresh receipts and close the plan and Bead.

## Surprises & Discoveries

The current identity guard rejects SIG_IGN before launching anything. A real
`/bin/sh` background Cargo invocation fails `two_worker_scheduling_corpus` with
exit 101 and `unsupported-sigint-owner`, while the foreground target passes.
CPython 3.12.3 installs its raising SIGINT handler only over the Unix default.
Python 3.12 documents SIG_IGN and SIG_DFL as ordinary dispositions, distinguishes
unknown native handlers, and returns the previous disposition when replacing it.
GNU Bash documents ignored SIGINT for asynchronous commands without job control;
local `/bin/sh` reproduces the same inheritance. POSIX's online page was blocked
by HTTP 403, so do not claim its contents were retrieved. The unjoined watchdog is
configured for 12 seconds, but its tests currently accept the default 140 seconds.

## Decision Log

The signal failure is a local abstraction error: the guard confuses inherited
policy with competing ownership. Accept Python default, SIG_DFL and SIG_IGN;
reject custom/unknown handlers and non-main-thread use. Leave ignored signals
ignored, including across child exec, rather than turning background SIGINT into
cancellation. Only the two defaults use the scoped non-raising recorder. Restore
the exact previous disposition after resource release on every exit path. Keep
the SIGCHLD prerequisite, child identity, EOF evidence and cleanup deadlines.

Tests that deliberately send SIGINT establish their own default disposition and
restore the inherited one afterward. Separate background-shell controls must
prove genuine SIG_IGN inheritance in both owner and child, send real SIGINT to
both, and still finish successfully. A real Rust replay verifies the tool entry
point. The elapsed oracle uses an independently stated 12-second contract plus
five seconds for cleanup and four for interpreter/startup overhead. Preserve
existing exit, checkpoint, receipt and conservative-cleanup assertions.

## Outcomes & Retrospective

The old helper failed ten compatible-disposition subcases across three new controls. All seven focused SIGINT controls now pass, and the foreground scheduling target passed all fifteen Rust entries, including the late-duration rejection and thirty Python controls. Ignored SIGINT remains ignored in the owner and child; Unix-default interruption is deferred through cleanup. The final matrix passed all 23 commands: foreground/background Cargo and thirty Python controls in normal/background/optimized modes, six fresh corpora, two-CPU contention, both Rust toolchains (361 entries each), both capacity mutation challenges and all three rebuilt HTTP smoke modes. The twenty source hashes match before/after/current, and all twenty-four mutation records preserve the intended oracle and complete process evidence. All five Jig targets and the explicit final backend check passed (361 test executions, zero failed/ignored). All five receipts were fresh with no missing, stale or failed gates. Work finish succeeded, Bead batter-953 was closed and its export was flushed. Changes remain uncommitted.
No production Rust API or dependency changes are needed. Linux evidence cannot
establish macOS, hosted CI or Python 3.9 execution; those remain explicitly unverified.

## Context and Milestones

Work in `/home/aa/Documents/batter`, baseline
`5db16f6f18fd918d3d3558a68843120bf9a13b79`. Preserve the existing mixed staged,
unstaged, untracked and unrelated Beads work. `scripts/scheduling_process.py`
owns child acquisition through capture/termination/reaping and signal restoration.
`scripts/test_scheduling_process.py` owns Python controls and Cargo's Rust fixture
launch controls. `crates/batter/tests/scheduling.rs` owns external fixture assertions.

First add background inheritance, compatible disposition restoration and native
SIG_DFL interruption controls, plus a background real-fixture replay. Run these
against the old helper and retain the failures. Then change only the signal guard
and selected scoped disposition. Exercise success, spawn failure, callback error,
watchdog timeout and interruption; retain the existing repeated-SIGINT cleanup
regressions. Add parent-measured elapsed assertions to both unjoined fixtures and
a synthetic late-duration rejection control that does not sleep for 140 seconds.

Update `docs/guarantees.md`, `docs/testing.md`, `docs/status.md`,
`docs/references.md` and `docs/validation.md`. Explain compatible signal policies,
restoration, independently checked latency and unverified environments. No
persisted application state or migration changes occur.

## Validation and Acceptance

From the repository root, run focused Python controls before and after the fix,
then `cargo test -p batter --test scheduling --locked` in foreground and through
`/bin/sh` with background execution and explicit wait-status propagation. Run the
Python controls normally, optimized, and in a background shell. Retain exact
commands and evidence under ignored `validation/local/batter-953-inherited-sigint/`.

Run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`,
three fresh corpora each at two/four workers, two-CPU contention, the capacity
mutation challenge on both Rust toolchains, and rebuilt HTTP smoke in default,
SIGINT and deadline modes. Audit real test counts, mutation rejection reasons,
source hashes and the unchanged lockfile. Passing stress is non-exhaustive.

Finish docs before `scripts/jig work check`; run `scripts/jig check test` as the
final backend check. Inspect fresh work evidence/gates, finish this plan, close
`batter-953`, and run `br sync --flush-only`. No commit or publication is authorized.

## Recovery and Interfaces

Use standard Python signal/subprocess APIs and native Unix shell behavior. Keep
helpers private. No new dependency or generic process framework is necessary.
Keep test-created subprocesses owned and reaped even on assertion failure. Reuse
live verification handles, retain failed evidence and research any new semantic
question before another implementation round. Do not retry failures to green.

Created after the review to correct inherited policy and the missing timing oracle.
