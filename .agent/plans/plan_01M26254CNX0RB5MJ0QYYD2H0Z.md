# Fixture review remediation

Owning Bead: batter-kjl. Git baseline: 486e0b0f9f4c4439077418715843b30042205f7e. Existing staged implementation is retained; no commit is authorized.

## Progress
- [x] Research pinned Tokio watch semantics and harness destructive cleanup.
- [x] Centralize bounded consumer completion with recoverable pending ownership.
- [x] Isolate diagnostic pools and synchronize detached-session tests.
- [x] Add executable retry, pending recovery and diagnostic regression coverage.
- [x] Run both toolchains, live PostgreSQL, HTTP and Jig gates.
- [x] Repeat independent Claude Opus/Codex review until no actionable findings.

## Surprises & Discoveries
The watch channel deliberately retains an update issued during an active attempt; retries are broadcast and coalesce. The pinned harness admin drop uses FORCE. A later successful work-check api:test receipt satisfies root policy after the earlier stale receipt.

## Decision Log
Keep the adapter's fail-closed lease retention. Centralize consumer wait/diagnostics/recovery ownership in one private helper rather than invent another driver. A pending error owns the same run, retry control and session pool; no automatic retry or deletion. Separate diagnostic capacity and acknowledge lock setup before body completion. Keep watch broadcast semantics and test them directly.

## Outcomes & Retrospective
Completed: shared pending ownership, diagnostic isolation, explicit retry semantics, error retention and progress redaction. Final full verification and independent review adjudication are recorded below.

## Context and work
Adapter sessions.rs owns observation; runner.rs owns leases and report. Consumer support/fixture_run.rs and fixture_failures.rs currently await without a bound. Add support/fixture_completion.rs for shared bounded ownership and redacted diagnostics; use it in both callers. Add offline/private retry controls and real pending/recovery tests to the named live inventory. Update contracts/status/references/validation.

## Validation and recovery
Run cargo fmt, focused tests, both scripts/verify.sh invocations (default and RUSTUP_TOOLCHAIN=1.94.0), dedicated disposable PostgreSQL 18 live script on both toolchains, five HTTP smoke profiles per toolchain, scripts/jig work check/evidence/gates/finish. Logs under .agent/tmp/batter-kjl-review. Never alter existing external databases; remove only the task-created container. Preserve failures and repair without weakening assertions. Review the whole working-tree diff including staged changes, excluding only trusted .reviewignore paths.

## Interfaces and compatibility
No public API or dependency change planned. Clarify public retry docs. Private consumer pending error retains typed owner and concrete failures until resumed or deliberately abandoned; runtime-death limits remain explicit.


Follow-up evidence: 2026-09-10. Both toolchains passed 651 test/doctest executions and 26 live cases; all ten HTTP smokes pass. Second review found the diagnostic pool still closing around a live body. The pending owner now retains it, the outer helper returns before close, and a held-checkout test releases/resumes the same body. Private recovery retains all replacement session pools. Retry example and timing/error-preservation tests corrected. No runtime-death survival guarantee is claimed. Final independent review and fresh Jig gates remain before closure.


Final outcome: all implementation and validation completed; final independent Codex pass has no findings. Claude's remaining two low policy objections are outside the explicit runtime-death and fail-closed target contracts; adjudication is in .agent/tmp/batter-kjl-review/final-review.md. Both parent fingerprints match with complete coverage of included paths. Final matrix: 652 executions per compiler, 26 live cases per compiler, ten HTTP smokes, fresh Jig verify gate. All task-owned database containers were removed. The owning Bead is closed; plan closure records completion without source or test changes.
