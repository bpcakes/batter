# Retained atomic failure states and completed public cutover

## Outcome and scope

Owning Beads: Batter `batter-ga1`, Runledger `runledger-runledger-simplification-audit-5qz`.
On the existing PR branches, retain the first terminal transaction cause even
when a callback ignores it, distinguish abandonment, remove the public session
transaction protocol, and make uncertain/terminal wrappers admit only their
claimed states. No migration, publication, merge, retry policy or legacy bridge.

## Progress

- [x] Inspect current runner, session, verifier and reference consumer.
- [x] T-01: foundation states and narrow error types.
- [x] T-02: manual API removal and downstream adoption.
- [x] T-03: adversarial tests and fresh consumer exercise.
- [x] T-04: final verification and coordinated PR updates prepared for push.

## Surprises & Discoveries

The schema verifier uses a separate private native SQLx transaction alias, not
the old session transaction wrapper. Delete the obsolete wrapper entirely.

## Decision Log

- Use Arc<PgTransactionError> for terminal causes shared between callback and
  runner; preserve concrete generic application errors without erasure or Clone
  requirements. The runner retains the poison cause, while its body result
  retains the callback's chosen application result.
- PgAtomicError has Begin, Rejected and Uncertain(PgAtomicUncertainty).
  PgScopeError has Application and Terminal(PgScopeFailure); terminal failure
  distinguishes transaction, recovery and abandoned operation.
- Remove public PgSession::begin and PgTransaction implementation. Private verifier
  code retains its unrelated native transaction alias. Canonical rustdoc leads
  with run_atomic; exceptional consuming owner remains low_level only.

## Outcomes & Retrospective

Implemented and compile-checked across both workspaces. PostgreSQL 18.6 focused
atomic tests (21), complete SQLx suite (84), Runledger atomic plus migrations
(1+25), both Rust verification scripts, all five HTTP smokes, reference/adapter
live suites, Runledger lint and both nine-case external consumer suites passed.
The final Jig profile found three lines of new file-budget debt in delivery.rs;
the error classification examples now live in an included rustdoc page, preserving
both compile-fail assertions and the existing limit. All five final gates passed
in run `run_01M31GKMX91J8HTFETX2H9S2Q1`; API test receipt
`receipt_01M31GYJYF9SQ37R4AYPK3S256`. The relocated reference doctests also
passed on Rust 1.94.0. Batter foundation `77d639c71f07762f2d94635ae609173894285831`
is pinned by Runledger `b3ce9c4970ad7aae66bc7896fc5c032c4f29329d`; Batter CI pins
that Runledger source. Both PR descriptions include the new regression and
fresh-consumer evidence; hosted checks must evaluate the pushed heads separately.
The no-history consumer agent selected run_atomic and compiled first try; code
and limitations are in docs/evidence/atomic-consumer-2026-09-21. It found a stale
Runledger downstream-guide native transaction example, now corrected. This is
one compile-only direct-crate sample, not database or usability-rate evidence.

## Evidence and execution graph

Facts: atomic_runner.rs stores Option<owner> and discards terminal causes;
session.rs publicly exposes begin/commit; delivery.rs wraps broad error enums.
The review is reproducible from these signatures and branches.

### T-01 — Retain first poison cause and classify disposition
- Outcome: caught terminal failures cannot erase the original runner loss reason.
- Changes: crates/batter-sqlx/src/atomic/error.rs, atomic.rs, atomic_runner.rs.
- Depends on: none
- Verify: focused library checks; new boundary-loss, recovery and repeat-call tests.
- Done when: abandoned operations differ from poisoned operations; causes share identity.

### T-02 — Complete public cutover and adopt narrow types
- Outcome: no public session manual transaction or contradictory reference wrappers.
- Changes: SQLx lib/session/docs/tests; reference delivery/HTTP; Runledger exports/tests.
- Depends on: T-01
- Verify: compile-fail examples, exhaustive matches and workspace checks.
- Done when: only low_level exposes manual transaction ownership, canonical examples use runner.

### T-03 — Exercise failure and consumer contracts
- Outcome: adversarial database tests and an unprimed consumer validate the API.
- Changes: atomic live tests, public doctests, reference negative tests and evidence notes.
- Depends on: T-02
- Verify: PostgreSQL 18 with exact version; fresh no-context agent writes application plus intent/enqueue
  and explicit outcome handling in an isolated scratch consumer; inspect and compile its output.
- Done when: required negative paths pass and exercise gaps are resolved or honestly reported.

### T-04 — Validate and update both PRs
- Outcome: coordinated feature heads and immutable companion pins remain buildable.
- Changes: evidence, tracker, relevant docs, CI companion revisions.
- Depends on: T-03
- Verify: both Batter toolchains, final Jig api:test, five HTTP profiles, SQLx/reference/adapter
  PostgreSQL suites; Runledger lint, focused live and external-consumer suites.
- Done when: local gates pass, pins point to pushed matching source commits, PRs report evidence.

## Recovery and risks

This is a source-only coordinated breaking change. Preserve unrelated edits;
repair forward on feature branches. No destructive database or Git rollback.
Keep shared cause formatting redacted; check Error::source chains and matching
Arc identity. Cancellation must leave an explicit abandoned state even when
caught inside the runner. Never weaken existing live or compile-fail assertions.
Freeze tracked inputs during final Jig checks. Provision only disposable PG18
clusters; preserve sibling-source identity when reusing verification evidence.
