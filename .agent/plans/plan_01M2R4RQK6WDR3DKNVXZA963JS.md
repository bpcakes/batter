# Provider lease and durable retry eligibility

Owning Bead: `batter-gg8`. Baseline: `abbe6f5c27887db9c5cbc7b7bd1fb6dd9967b00f`
plus the pre-existing staged/unstaged reference changes. Preserve that index.

## Progress

- [x] Verify findings and record ADR-010 assessment and pinned upstream answers.
- [x] Implement post-lock/pre-commit authority and atomic provider eligibility.
- [x] Add isolated SQL and real-process native-completion interruption probes.
- [x] Complete both toolchain matrices, ten HTTP smokes and both live runners.
- [x] Complete Jig evidence/gates and independent re-review, update Bead.

## Surprises & Discoveries

The locking SELECT's time predicate is not a post-lock check. Lock-only blockers
do not trigger PostgreSQL's updated-tuple recheck. Later effect-row locks can
also consume the lease, requiring validation before publishing the transaction.
Provider delay formerly lived only in a completion value returned after an
application commit. Crash recovery therefore had no durable scheduling fact.
The first live fixture setup used an overlong harness project name; shortened
to the supported 16-character bound. Focused SQL and real-worker probes pass.

## Decision Log

Keep application provider protocol in the reference and queue ownership native.
Make provider persistence take timing and return the native scheduling result.
Require durable eligibility for both initial and proven-absence POST markers.
Use a new forward migration; do not add reference backfill/mixed-version policy.
Runledger zero delay preserves normal backoff. Its 500ms reserve is headroom,
not a separate enforced SQL timeout. Document lost unpersisted remote responses.
The initial controller omitted endpoint-variable forwarding. The user explicitly
selected normal Jig workflow; the controller applied no edits and terminated
SCOPE_CHANGED when required tracker/plan work resumed. Retain its review reports.

## Outcomes & Retrospective

Both complete toolchain scripts and all ten HTTP profiles passed. Each live
runner executed exactly 66 cases plus maintenance and state probes. Rust 1.94
required splitting a test helper without changing its assertions; its final
script and the refined state probe on both toolchains passed. Jig validation
`receipt_01M2R6G17PW3TME1SC9P57A61S` contains passing API-test receipt
`receipt_01M2R6G0GR9CMAPPHJD6RSVTJ6`; evidence/gates are fresh with no unresolved
gate. Both independent reviews completed against an unchanged working-tree
fingerprint. Their supporting findings were corrected: stale policy prose and
an offline 70-combination terminal-projection oracle. The untracked-file warning
is a packaging obligation, not permission to stage or a working-tree defect.
A fresh targeted follow-up review found no actionable findings and both full
verification scripts passed again. Final Jig validation
`receipt_01M2R7DGQTBPQ76VH3W6SFBYH2` includes API-test receipt
`receipt_01M2R7DG0VSWVJJ33CA5A2CRYH`; all five targets passed and evidence/gates
were fresh. Close the owning Bead and refresh the metadata-only policy checks,
reusing these unchanged Rust receipts, before finishing the Jig plan.
No agent-usability evaluation, hosted CI, publication, staging or commit is
claimed. Temporary database roles were removed and both task-owned servers
stopped, without removing their data directories.

## Execution and verification

Edit `examples/reference-service/src/delivery/worker{,/state}.rs` for the private
boundaries. `state/live_tests.rs` isolates job/effect lock waits without native
heartbeat; `tests/support/provider_effects/retry_eligibility.rs` kills the actual
worker between application and native commits. The existing live runner requires
the main inventory, maintenance case and new state library case.
Run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`.
Rebuild `http_service` on each toolchain and run default, SIGINT, deadline,
WARN-filter and WARN-filter/deadline smoke profiles. Execute both complete live
runners with the explicitly selected disposable PostgreSQL18.6 endpoints.
Use `scripts/jig work check/evidence/gates/finish` for this plan, freezing source
through each check batch. Existing exhaustive globs cover the new migration,
Rust test modules and modified runner; verify contract wiring rather than
adding redundant per-file entries. Record actual outcomes in validation docs.

## Recovery and limits

Never restore the baseline over user changes. Preserve failed test evidence,
join spawned probes before pool closure, and stop only the two task-owned
disposable servers when finished. A crash before response persistence can lose
the observed provider delay; types cannot remove that remote acknowledgement gap.
