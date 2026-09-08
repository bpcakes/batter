# Harden non-yielding test process boundaries

Owning Bead: `batter-fvz` (reopened for review fixes). Git baseline:
`e5f2f04b2dbb349d08085caf177f662fcbc89812`. The earlier implementation and
completed plan remain in the working tree; do not discard them or rewrite their
historical validation. This execution changes private test infrastructure and
its contracts/evidence, and removes non-Unix signal fallbacks from examples.
The final platform scope is Unix-only, including Linux and macOS; Windows is
unsupported and not planned. Library behavior, APIs and dependencies stay fixed.

## Progress

- [x] Read repository/package guides and Fowler Rust refactoring references.
- [x] Baseline: all eight non_yielding integration entries pass locally.
- [x] Research GitHub cancellation and Rust process/pipe semantics.
- [x] Replace ambient launch state and parent-only lifetime ownership.
- [x] Validate actual process results and add adversarial controls.
- [x] Run both supported toolchains and all three HTTP smoke modes.
- [x] Remove Windows/non-Unix paths and record the Unix-only platform policy.
- [x] Re-run both toolchains and HTTP smoke after the platform cutover.
- [x] Run final Jig gates and backend completion test.
- [x] Update Bead, contracts, status and validation.

## Surprises & Discoveries

GitHub documents cancellation signals and eventual process-tree termination.
Runner v2.337.0, commit 397b032cbf865e9c3ddfab89d533ec19325e1273, instead
sends Unix signals to one PID and its NixKillProcessTree calls Process.Kill()
without the tree argument. JobExtension separately scans inherited tracking IDs
at job finalization. Neither mechanism is a test-parent-death guarantee.
Rust Child::kill may succeed after exit; wait status is the source of truth.
std::io::pipe is stable since 1.87, below the workspace minimum 1.94.
Only the Linux Python regression uses a child subreaper, keeping Cargo's process state
untouched. Its actual parent-only SIGKILL probe passed and reaped the orphan.
An initial compile check rejected catch_unwind around a moved JoinHandle; a
joined, deliberately panicking owner thread exercises the same unwind path
without asserting UnwindSafe or retaining unwound state.

## Decision Log

- The findings concern harness ownership and evidence, not Batter supervision.
  Keep the non-yielding task and production code unchanged.
- Apply Fowler's Split Phase and Encapsulate Record to separate launch,
  containment and observed evidence. These fixes intentionally change harness
  failure behavior; do not describe them as a pure behavior-preserving refactor.
- Replace temporary capture directories with a continuously drained pipe and
  bounded memory. Reject overflow, so truncation cannot hide later failures.
- Authorize the exact fixture entry through stdin with its actual child PID.
  Retain the writer in the parent; EOF terminates the entire fixture outside
  Tokio. Arm a longer emergency OS-thread deadline before reading launch data.
- Record requested kill separately from the status returned by wait. On Unix
  require SIGKILL for expected watchdog termination, without non-Unix fallbacks.
- The project owner excludes Windows from support and future plans. Linux and
  macOS remain in scope. Remove the Windows CI/exit-code branch and all three
  existing non-Unix signal fallbacks. Record this in README, root/package
  AGENTS, package docs/rustdoc, architecture, operations, contracts and ADR-007.
- No trait framework, new public API, library panic hook or Tokio task release.
  Scanner length/unwrap warnings in assertion-heavy fixtures are not defects.

## Outcomes & Retrospective

The private harness now separates launch, process ownership and captured
evidence, with no production or dependency change. Eighteen integration entries
pass locally; both supported toolchain matrices pass 124 foundation entries,
145 workspace entries and two doctests. All three HTTP smoke modes pass. Five
harness mutations were rejected and restored. All five Jig targets passed and
the verify gate was fresh. Final receipts are refreshed after tracker/docs
closure because their changed worktree fingerprint invalidates earlier ones.
Both compiler matrices and all three rebuilt HTTP smoke modes also passed
after the platform clarification. Windows paths and CI were removed; Linux and
macOS remain in scope. Final Jig receipts are fresh with all five targets and
the required verify gate passed; the final `scripts/jig check test` passed.
Bead `batter-fvz` is closed with the platform policy in its acceptance criteria.
The 18 subprocess entries include actual parent-only SIGKILL/adoption/reaping,
and the earlier five rejected mutations demonstrate that the new negative
controls detect their intended regressions. No library behavior, public API or
dependency changed. macOS execution remains unverified; Windows is explicitly
unsupported and not planned. No commit, publication or deployment occurred.
No commit/publication authorized.

## Context and orientation

`crates/batter/tests/non_yielding.rs` owns scenario assertions. Its fixture
module owns actual lifecycle work; watchdog owns child processes. Relevant
contracts are in docs/testing.md, guarantees.md, status.md and references.md.

## Plan of work and concrete steps

1. Keep the eight baseline entries green while replacing watchdog internals.
   Use a private launch protocol and bounded output reader; keep all existing
   report, live-resource and cleanup assertions.
2. Add real subprocess controls for natural success/failure near termination,
   forbidden milestones, output overflow and launch authorization. Exercise
   parent-pipe closure and the independent emergency deadline. Add a Linux
   abrupt-parent-death probe with an adopting reaper so it leaves no zombies.
3. Keep focused macOS CI coverage, clearly unexecuted locally. Remove all
   Windows targets and non-Unix implementation branches. Native Unix signal
   registration still precedes readiness acknowledgement in every example.
4. Run focused tests after each coherent change. Run bash scripts/verify.sh and
   RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh. Build the HTTP example with
   cargo build -p batter-axum --example http_service --locked; run
   scripts/smoke_http.py with its binary in default, --signal SIGINT and
   --deadline modes. Run python3 scripts/check_package.py and git diff --check.
5. Finalize tracker/docs before scripts/jig work check, work evidence and work
   gates for this plan. Finish backend verification with scripts/jig check test
   and finish the work session. Store logs in .agent/tmp/batter-fvz-review/.

## Validation and acceptance

All prior lifecycle assertions remain meaningful. Inherited scenario state
cannot run blocking work in ordinary tests. Parent disappearance terminates
an entered blocked fixture; normal paths kill/reap and retain diagnostics.
Natural exits cannot be described as observed SIGKILL. Output truncation cannot
produce a passing result. Failure probes themselves have external bounds and
cleanup ownership. Both supported toolchains and repository gates must pass.
Do not claim hosted CI, macOS execution or application cleanup from local Linux
tests. Windows support is explicitly excluded rather than an open test gap.

## Idempotence and recovery

All changes are local/reversible. Never reset the working tree or overwrite
the earlier completed plan. On failure, preserve the failing logs, repair the
smallest responsible change, and re-run its focused checks before widening.
Kill and reap owned fixture children on test errors. Do not weaken semantic
assertions or silently skip an unavailable prerequisite.

## Interfaces and dependencies

Private test-only protocol; Rust 1.94 minimum, Tokio 1.53.1 remains resolved.
No persisted application data, adapters, external service, new dependency,
or compatibility migration is involved. The Linux orphan-process regression
may use the already-required Python 3 standard library in an isolated process.
