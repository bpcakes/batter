# Separate subprocess evidence from deadline policy

Owning Bead: `batter-fvz`. Git baseline: `e5f2f04b2dbb349d08085caf177f662fcbc89812`
plus the user's existing staged/unstaged work. Scope is the private subprocess
harness and its documentation/CI. Preserve public APIs, dependencies, Unix scope,
PID-bound launch, independent emergency exit, process reap and conservative
lifecycle assertions. No commit or publication.

## Progress

- Research and a fresh 22-entry subprocess baseline passed.
- Behavior-preserving extraction passed all 22 original entries.
- Timestamped evidence, atomic inspection, pure deadline policy and first-kill
  validation are implemented; the scheduling-sensitive sleep control is removed.
- Three deliberate timing mutations failed their regression controls and were
  restored before verification.
- Both full Rust toolchain matrices and rebuilt default/SIGINT/deadline HTTP
  smoke tests passed on Linux x86_64 and the user-authorized macOS arm64 host.
- Contracts, platform claims, test counts and validation evidence are updated.
- All five Jig targets passed; `work evidence` and `work gates` reported the
  required verify gate fresh, receipt `receipt_01M20GH1E7CSGAHGPT3C1W9FDZ`.
- Final `scripts/jig check test` passed (exit 0), along with static package
  inspection and `git diff --check`. No source/build file changed after the
  66-file cross-host manifest verification. Implementation and verification are
  complete; the Jig finish record and owning Bead carry delivery closure.

## Surprises & Discoveries

The root problem is using polling time as event evidence and testing deadline
arithmetic with real sleeps. Startup previously parsed substrings while final
validation parsed complete protocol lines. Kill validation also
used elapsed time after capture joining rather than the kill-request time.
Rust 1.94 Unix code caches reaped statuses before subsequent kill/wait calls;
current macOS runner images include rustup. Neither established execution of
this uncommitted checkout on macOS. The user then supplied SSH access; the
final source passed both full matrices and HTTP smokes in an isolated temporary
directory on macOS 26.6.2 arm64. Recording evidence and reading its decision
clock must share the capture mutex to avoid stale-snapshot deadline races.

## Decision Log

Use Fowler Extract Function / Encapsulate Record to put bounded diagnostic bytes
and protocol evidence in one owner, first preserving current behavior. Then
separately fix behavior: timestamp complete records at capture, centralize exact
parsing, and decide event deadlines from those timestamps. Extract a small closed
wait policy with pure inputs, so boundary tests control instants rather than
sleeping near a live-process deadline. Record actual kill-request time separately
from final elapsed time. Keep live subprocess tests for the OS/Tokio contracts.
No injected-clock framework, new dependency, unsafe code, or public API is needed.

## Outcomes & Retrospective

The fix is confined to the private harness; no production architecture change
was justified. Linux passes 37 focused / 164 workspace entries and two doctests;
macOS passes 35 focused / 162 workspace entries and two doctests. Both toolchains
pass formatting, compilation, Clippy and rustdoc. Primary references, host and
source hashes, commands, mutation oracles and limitations are recorded in
`docs/validation.md`. The updated hosted macOS job remains unexecuted.

The heuristic scanner's test unwraps, shared capture
mutex and straight-line fixture length are not independently actionable defects.
The accepted issues are local evidence ownership and time-policy coupling.

## Steps and validation

1. Extract evidence handling from `non_yielding/{capture,watchdog}.rs` into a
   private module; rerun the existing focused suite to prove the structural step.
2. Timestamp complete event records; migrate startup/final validation to the
   same parser. Reject overflow and genuinely late records, but accept an on-time
   record when polling occurs late. Keep metadata bounded.
3. Extract the fixed/after-event deadline policy; replace the four-second sleep
   control with deterministic boundary cases and test absent/late/fragmented/
   misleading output. Keep the real blocked-runtime scenarios and emergency
   controls. Validate kill timestamps and status independently.
4. Expand the macOS CI definition across both supported toolchains and compile
   the workspace targets. Execute both full matrices and rebuilt HTTP smoke tests on the
   user-provided macOS host; retain the hosted-job limitation separately.
5. Update guarantees, testing counts, implemented status, primary references,
   validation evidence and the Bead. Run `bash scripts/verify.sh` and
   `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`; rebuild the HTTP example and
   run default, SIGINT and deadline smoke modes. Run Jig work check/evidence/gates,
   finish backend checks with `scripts/jig check test`, then finish this plan.

Logs go under ignored `.agent/tmp/batter-fvz-evidence/`. Narrow checks are
`cargo test -p batter --test non_yielding --locked`. Commands are rerunnable;
any mutation test must restore exact saved bytes before broad verification.
Keep prior plans as historical evidence and all prior work intact.
