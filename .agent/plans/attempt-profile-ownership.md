# Shared profile ownership for authentication attempts

Owning Bead: `batter-979`. User authorized the previously escalated API correction.

## Outcome

Admission and completion establish the same explicitly declared PostgreSQL role,
schema and custom settings.

## Scope

Preserve native Runlimit storage, budgets, observers,
failure counting, audit atomicity and acknowledged-outcome retention. Do not
implement the downstream D-09 cutover, provisioning, tenant features or passkeys.
No migrations, releases or PR merges are authorized by this work.

## Progress

- Merged Runledger #21 pin adopted; focused adapter compilation passed.
- Shared profile pool checkpoint `e9c5b04` committed; focused foundation checks passed.
- Runledger PR #22 delegates to it; full native tests and packaged smoke passed.
- Attempt runner cutover and 14 live tests passed. Old unprofiled-completion
  mutation fails the new regression. Full matrix, Jig and native review pending.

## Surprises & Discoveries

## Evidence

Fact: `AttemptRunner::run` admits through native pool hooks but completes through
unprofiled `run_atomic_in`, which discards their role/path. Prior PostgreSQL 18
reproduction observed restricted admission and privileged application execution.
Fact: Runledger's `database.rs` already owns all three mandatory pool hooks,
including release normalization for SQLx fast acquisitions. Native Runlimit
admission only needs that pool; no new limiter execution seam is necessary.

## Decision Log

## Design

Extract the existing pool/profile owner into `batter-sqlx::PgProfiledPool` and
delegate Runledger's existing public database API to it. Require this owner in
AttemptRunner, reject multiple fallback schemas before execution, and retain the
same explicit profile through the context-aware atomic completion. An optional
profile setter or completion-only fix leaves mismatched admission representable.
Copying Runledger's hooks into the attempt adapter would duplicate the boundary.
Generic profiles may still describe multiple trusted schemas; native adapters
enforce their stricter single-schema contracts. Native admission may temporarily
tighten timeouts to its own budget. Profiles are policy, not endpoint attestation
or a sandbox against intentionally hostile application SQL.

## Execution graph

### T-01 — Shared policy-owned pool and retained profiled atomic work
- Outcome: one foundation implementation owns mandatory pool normalization.
- Changes: SQLx pool owner, atomic context wrapper, docs and failure tests.
- Depends on: none
- Verify: SQLx offline tests, doctests, live PostgreSQL 18 pool/profile controls.
- Recovery: unpublished foundation checkpoint; keep consumer adoption blocked.
- Done when: arbitrary pools cannot construct the owner, and fast paths restore policy.

### T-02 — Native adoption without duplicated hook machinery
- Outcome: existing Runledger database consumers retain behavior and signatures.
- Changes: native database wrapper and reviewed foundation source pin.
- Depends on: T-01
- Verify: database_profile suite, native tests/Clippy, source guard and consumer smoke.
- Recovery: additive follow-up PR; never rewrite the merged PR or foundation history.
- Done when: native profile regressions and exact-source guard pass.

### T-03 — Profile-bound attempt admission and completion
- Outcome: both attempt phases execute under one declared policy.
- Changes: constructor, completion call, callers, role/schema regressions, docs/pins.
- Depends on: T-01, T-02
- Verify: custom-schema restricted-role success/rejection and RLS controls; drift
  rejection; existing ten live attempt tests; both Rust matrices, HTTP smokes,
  final Jig receipts, native review over stable range, hosted CI.
- Recovery: PR #3 stays draft until validation; no downstream pin/adoption yet.
- Done when: original privileged-execution trigger fails and legitimate atomic
  audit/retry behavior passes with no unresolved review findings.

## Rollout and recovery

Foundation files are checked against an immutable native pin. Commit the tested
foundation checkpoint first, then pin it in the native follow-up, then advance
Batter's native dependency. Do not weaken source checks to build an intermediate
graph. Keep full Git history. No production data changes. Configuration failures
fail before verification/application callbacks; connection cleanup uses existing
retirement semantics. Any newly discovered wider architectural issue is escalated.

## Verification

Each task's commands and falsifiable acceptance are listed above. Original role
loss and a legitimate restricted-role control must both be exercised on PG18.
No claim of a fixed security boundary until all relevant gates pass.

## Risks

The foundation/native source pin requires a coordinated follow-up PR; owner is
the implementation agent. Containment is draft PR #3 and no downstream adoption.
Pool policy does not attest endpoint identity or concurrent privilege changes.

## Outcomes & Retrospective

Pending execution. Security-boundary investigation is independent; final native
review remains required. No claim of downstream deployment or load readiness.
