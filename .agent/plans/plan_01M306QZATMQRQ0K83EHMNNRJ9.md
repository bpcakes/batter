# Owned PostgreSQL scopes — batter-psi

## Outcome and scope

Deliver a reusable PostgreSQL 18 transaction owner and read-only snapshot runner
in `batter-sqlx`, without a Runledger dependency. Application code and downstream
libraries execute native SQLx queries within consuming scopes. Runledger domain
SQL stays upstream. The user authorized coordinated adoption in sibling Runledger
on `feat/batter-owned-transactions`; no publishing or migration changes.

## Progress

- Repository inspected; branch `feat/owned-postgres-transactions` created.
- Owned foundation, Runledger wrapper, schema snapshots and reference submission
  cutover implemented. Public resource views and optional SQLx Runledger bridge removed.
- All 74 SQLx live controls passed, including 11 new owned-scope adversarial
  controls. Runledger integration, all 66 reference cases and two additional
  maintenance/provider controls passed on PostgreSQL 18.6.
- All five HTTP process smokes passed. Both toolchain workspace matrices and
  doctests passed; Clippy found enlarged submission errors, repaired by boxing
  retained scope causes without discarding either error. Final default Clippy,
  rustdoc and reference-submission regression pass. Final MSRV script in progress.
- Runledger: 25 migration/schema tests, 47 enqueue/intent/requeue tests (one
  existing ignored stress case), nine source and packaged external-consumer
  tests, producer/worker example, all-target check and full lint/doc pipeline pass.
- Source branches pushed at user request. CI pins Batter foundation 34bdd640
  and Runledger implementation 4a973a3; additional Batter commits change only
  CI/docs and reference error layout, not foundation identity.
- First Jig run executed tests successfully but correctly rejected freshness
  because edits occurred during execution; rerun with a frozen worktree before
  opening the linked PRs. No weakened or fabricated receipt is accepted.

## Surprises & Discoveries

- Master now includes the core/facade split and pinned native-resource bridge.
- SQLx 0.9 query results discard command tags; validate a non-aborted transaction
  immediately before COMMIT, with no user code between validation and completion.
- XID continuity does not detect rollback to an earlier savepoint. Each scope
  needs its own parent savepoint, released with all descendants before returning.
- Existing tracker DB import fails semantic verification of batter-310. Use br
  JSONL-only mode to preserve the authoritative merged tracker (batter-psi).

## Decision Log

- `PgAtomicTransaction` owns a retiring lease; no executor, Deref, raw resource,
  public constructor or Clone. Begin normalizes then starts READ COMMITTED READ
  WRITE and assigns the top-level XID. Each operation consumes the owner.
- `application` errors are terminal; `operation` returns `(owner, Result<T,E>)`
  only after successful completion or acknowledged savepoint rollback and fresh
  continuity validation. Terminal errors retain primary and recovery causes.
- Scope savepoints use private random identifiers; release removes descendants.
  Arbitrary SQL is not an adversarial SQL sandbox: explicit early COMMIT may
  already have effects, deliberate discovery/manipulation of internal savepoints
  is outside the contract. Boundary loss never returns a reusable owner.
- `PgReadOnlySnapshot::inspect` owns acquisition, normalization, REPEATABLE READ
  READ ONLY, a scope guard, validation, and rollback before returning the result.
  Domain-specific evidence is constructed by its consuming library.
- The user rejected a legacy bridge and authorized the coordinated Runledger
  feature branch. Remove borrowed views outright. `batter-runledger` reexports
  the Runledger owner; Runledger depends only on `batter-sqlx -> batter-core`.
  Sibling path dependencies are a local coordinated-review arrangement, not a
  published independent-consumer claim. Existing native persistence APIs are
  low-level surfaces, excluded from the protected owner contract and prelude.
- Generate savepoint names locally so inspectors can lock authoritative schema
  objects before the first snapshot-bearing query. Probe optional/missing
  relations with recoverable lock savepoints; never certify an unpinned object.
- Reuse Runledger's existing migration bundle fingerprint in snapshot evidence.
- Disk exhaustion interrupted the first full matrix/Clippy attempt. Removed only
  generated external-consumer target artifacts; rerun failed gates after recovery.
- Public results use redacted formatting and retain native errors for explicit
  inspection. Cancellation destroys the owner but does not return an error to
  a caller that dropped the future, or prove remote rollback.

## Execution graph

### T-01 — Owned generic scopes
- Outcome: consuming transaction/snapshot APIs enforce resource disposition.
- Changes: crates/batter-sqlx/src/atomic*, snapshot*, lib.rs and public docs.
- Depends on: none
- Verify: compile-fail docs, PostgreSQL 18 boundaries, savepoint recovery, commit
  uncertainty, snapshot consistency and cancellation tests.
- Recovery: additive API; disposable fixture only.
- Done when: strong owner cannot be forged or recovered from terminal failure.

### T-02 — Acyclic integration
- Outcome: all SQLx feature combinations are independent of Runledger.
- Changes: SQLx manifest/session bridge removal, batter-runledger owner reexport,
  facade feature consumer, reference service and compatibility docs.
- Depends on: T-01
- Verify: feature graph and coordinated Runledger live composition test.
- Recovery: no persisted format changes; both branches and consumers move together.
- Done when: facade runledger still compiles; SQLx has no Runledger dependency.

### T-03 — Delivery evidence
- Outcome: executable contract and recorded results.
- Changes: live inventory, docs/references.md, docs/status.md, guides and Bead.
- Depends on: T-01, T-02
- Verify: scripts/verify.sh on Rust 1.98.1 and 1.94.0, SQLx live runner against
  postgres:18 (record SHOW server_version), five HTTP smokes and fresh Jig gates.
- Recovery: preserve failures and correct code; never weaken checks to pass.
- Done when: required checks pass and limitations are documented.

## Outcomes & Retrospective

Implemented and verified. Linked PRs: [Batter #2](https://github.com/bpcakes/batter/pull/2)
and [Runledger #20](https://github.com/bpcakes/runledger/pull/20).
Final Rust 1.94.0 verify.sh passed. Rust 1.98.1 final matrix/Clippy/format pass
in Jig run `run_01M309KZ1MZMMKC2V0ZSXMT1CJ`, with API test receipt
`receipt_01M309XKYE431DNK10CQRB14BA`; separate warnings-as-errors rustdoc passed.
All five required Jig targets succeeded on a frozen tree; external Runledger
source remained unchanged. Hosted macOS checks passed on 1.94.0 and 1.98.1.
Publishing is not part of this task: foundation crates remain unpublished, and
Runledger release must wait for their publication. Review uses paired sibling
checkouts, with immutable CI revisions and explicit packaged-smoke patches.
