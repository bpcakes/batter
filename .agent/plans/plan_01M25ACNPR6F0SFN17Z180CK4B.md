# Isolate reusable process controls and close the review loop

This ExecPlan follows `.agent/PLANS.md`; owning Bead is `batter-u0m`, reopened
after review found a Linux-only failure. `batter-zu8` remains behaviorally complete;
its missing hosted macOS target is addressed here without changing its acceptance.

## Progress

- [x] Researched Cargo package-relative environment semantics, current CI selection,
  upcoming core extraction and available Linux execution (Docker Linux arm64).
- [x] Separate generic Unix process machinery from foundation policy/self-tests.
- [x] Strengthen HTTP admission observation and record ordinary rejection outcome;
  include component ownership in macOS CI.
- [x] Run focused and full two-toolchain verification on Linux and macOS, plus smokes.
- [x] Address second-review cancellation-window and tracker findings, plus bounded
  diagnostic/poison/deadlock controls and manifest ownership clarification.
- [x] Reverify the diagnostic correction snapshot on both platforms/toolchains.
- [x] Restore a strict exited-child oracle by prioritizing finalized exit evidence;
  add expired-clock regression, check nested HTTP budgets and shorten negative control.
- [x] Reverify the strict process-control correction on both platforms/toolchains.
- [x] Retain precise final-capture diagnoses and actual forced-teardown reports;
  strengthen received-rejection trace and diagnostic-section controls.
- [x] Reverify diagnostic completeness on both platforms/toolchains.
- [x] Bound construction/readiness while retaining shutdown ownership; add a
  readiness-stall control and correct baseline dependency evidence.
- [x] Reverify startup-bound snapshot on both platforms/toolchains.
- [x] Restore prior Linux CI job policy; require non-forced completion in both
  component comparisons and verify shared-source affected-check selection.
- [x] Repeat independent Claude Opus and native Codex review until no actionable findings.
- [x] Record executable and review evidence for delivery closure. Final gate and
  finish outcomes are recorded by Jig in append-only state, not preclaimed here.

## Surprises & Discoveries

The Linux helper failure is caused by compiling foundation-specific self-tests in
the Axum package, where CARGO_MANIFEST_DIR changes. A path correction alone would
retain duplicated self-tests and a dependency on the future-moved core test tree.
The process transport, suite scenario timing and self-tests need distinct owners.

## Decision Log

Put private std-only shared sources under `test-support/process/` at workspace
level. They supply bounded capture, launch and process lifetime mechanics without
foundation/adapter dependencies or scenario names. Foundation-local thin wrappers
include the shared sources and attach existing white-box controls; only foundation
owns its non-yielding fixture and Python helper. HTTP imports shared mechanics
directly and supplies an explicit fixed deadline and its own dispatch. No public
library API, new crate, new process runner, or platform fallback is introduced.

Keep the generic controls' existing semantic assertions. Suite-specific policy
stays near the fixture; shared code takes an explicit WaitPolicy. This limits
implicit dependencies and keeps the same relative workspace location through
the planned core extraction. Include shared sources in formatting and existing
Jig input globs. No database or durable-state changes are involved.

## Outcomes & Retrospective

Implementation and both macOS matrices passed (611 executions each), as did both
Docker Linux arm64 matrices (615 executions each, including Linux-only controls).
All twenty rebuilt HTTP smoke profiles passed. Archive regression passed.
Earlier macOS evidence remains historical; the new source-hash-verified Linux
copy establishes the previously omitted behavior. The first Linux compile caught
a copied macOS metadata sidecar beside a SQL migration; removing that generated
container-only file corrected the copy without changing repository code.
Logs are in `/tmp/batter-review-fix-d9d5cW`.

Second same-scope review: Codex clean, Claude two lows (100 ms assertion window,
stale cross-scope Bead evidence). Both corrected. Additional diagnostic controls
retain yielding-timeout and teardown failures without event-lock poisoning;
virtual-time component bounds reject pending comparisons. All changes remain
test/manifest/CI/evidence-only beyond the original preserved startup lint allowance.
An old exited-child test's diagnostic ordering assumption was corrected; an old
tracing assertion failed once and then passed 30 full-library repetitions unchanged.
The latter cause is unconfirmed and must remain recorded, not claimed fixed.
Subsequent complete matrices passed on both supported toolchains: macOS 615
executions each, Linux 619 each, all zero failures/29 ignored. All twenty rebuilt
HTTP smoke profiles passed again. Final code review and fresh Jig receipts remain.
Third review: Codex clean; Claude identified oracle weakening and insufficient
diagnostic timeout margin. Both corrected in implementation/policy, not by relaxing
assertions. The strict missing-final-event control now covers expired/unexpired
startup clocks. Parent HTTP bound is eight seconds below ten-second emergency;
negative diagnostic case consumes only 500 ms. Component close metadata is corrected.
Strict correction matrices passed: 617 executions per macOS toolchain and 621 per
Linux toolchain, zero failures/29 ignored. All twenty HTTP smokes passed again.
Source hashes matched the Linux copy. Next independent review and final gates pending.
Fourth review: Codex clean; Claude three lows in observation/diagnostic oracles.
Corrections retain panic/overflow before missing-event diagnosis, require trace
for received 503 without mistaking socket closure for absent construction, and
budget teardown beyond the server phases while retaining the actual report.
New controls cover final-capture failures, forced teardown and rejection traces.
Diagnostic-completeness matrices passed: macOS 621 executions each, Docker Linux
625 each, zero failures/29 ignored, all twenty HTTP smokes, format/Clippy/rustdoc
and static repository gates. Linux source hashes matched. Fifth review and final
Jig work receipts remain pending; hosted load headroom remains unverified.
Fifth review: Codex clean with 16/7/48 focused tests; Claude two lows (startup
diagnostic gap and baseline lock wording). Startup/readiness now share one bound
and retain the running owner for teardown. A new control proves that path.
Actual budget total_allowance checks replace phase duplication. Component event
assertions use owned snapshots. Five ordinary-drain samples per platform all
closed transport; synchronized admission separately proves 503 routing.
Startup-bound matrices passed: macOS 622 executions each, Docker Linux 626 each,
zero failures/29 ignored, all twenty rebuilt HTTP smokes and static file-budget
checks. Source hashes matched Linux. Sixth review and final Jig work receipts pending.
Sixth review: Codex clean; Claude one low concerning an added Linux job timeout
shorter than existing sequential matrix bounds. Removed that addition, retaining
all fixture/matrix bounds and the existing macOS policy. Added the suggested
non-forced completion assertion. Jig's immutable affected explanation proves
all five shared Rust sources are direct inputs to API test/fmt/Clippy; no fake
crate root is needed. Historical validation now has an inline supersession note.
Final correction matrices passed on both supported compilers: macOS 622 each,
Linux 626 each, zero failures/29 ignored; all twenty rebuilt HTTP smokes passed.
Linux source hashes matched. Logs use the round7 suffix.
Seventh review: Codex clean; Claude one low in root navigation, now corrected.
The root guide and agent map identify private process sources and distinguish
them from the leaf test crate. This is documentation-only; round7 executable
evidence remains applicable. Parent timing is not a hosted guarantee; disconnect
regressions intentionally require the measured pre-release drop. Eighth review
and final Jig work receipts remain pending.
Eighth review completed clean from Claude Opus and native Codex against verified
complete fingerprint dfb4e38753ad02af95e358887c8d924b942bc6f7b25d0f15fb190c50feeeb607.
Codex reran 17 HTTP, seven component and 48 process tests successfully. Agent-map
validation passed. All remaining review questions are disclosed coverage or
future-maintenance limits, not actionable current defects. Final review evidence
and delivery notes are finalized before gate execution. Jig state records the
actual final gate results and plan closure.

## Context and execution

Inspect `crates/batter/tests/non_yielding`, `crates/batter-axum/tests/http_lifetime`
and `.github/workflows/ci.yml`. Move reusable source to `test-support/process` via
apply_patch; retain foundation wrappers/tests. Remove every Axum source inclusion
of the foundation fixture and its self-tests. Require the admission companion's
second event to carry 503/server_error, and record rejection-vs-closure evidence
without restricting the permitted ordinary-drain race.

Run focused component, non-yielding and HTTP targets. Check Cargo discovery lists
so Linux parent-death controls belong only to the foundation. Run verify.sh on
1.98.1 and 1.94.0 on macOS and in an isolated Linux Docker environment; build the
HTTP example and run all five existing smoke modes on each. Keep local container
evidence distinct from hosted CI. Update validation, status, testing, guides and
references. Run comprehensive-review with defaults Claude opus and Codex on one
immutable working-tree fingerprint; fix any actionable findings and repeat.

## Recovery and completion

Use isolated temporary Docker build/cache storage and no user-service mutations.
Keep source and test checks locked. Do not commit or publish. Preserve previous
worktree edits and append-only Jig state. Capture logs outside the repository.
After the review is clean, finalize docs and Bead state before final Jig work
check/evidence/gates/finish to avoid self-invalidating receipts.
