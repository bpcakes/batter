# Retained provider boundary regression proof

Owning Bead: `batter-zfe`, discovered from `batter-ws3`. User authorized normal
Jig workflow; preserve the index and all existing changes. No commits or backfill.

## Progress

- [x] Recheck both review findings against current source and existing tests.
- [x] Add negative provider identity and future-delay reconciliation probes.
- [x] Run focused probes, both full Rust/live matrices and ten HTTP smoke profiles.
- [x] Independent scoped review and current Jig receipts; Bead/plan closure follows.

## Surprises & Discoveries

Current runtime implements both invariants. Tests must distinguish worker load
rejection from later request-construction rejection, and keyed GET eligibility
from POST eligibility. There is no demonstrated recurrence of a runtime defect
and no basis for changing the production API in this task.

## Decision Log

Use the existing real-provider outcome fixture and production child to prove
zero provider requests after independent retained payload/key corruption. Exercise
both owner-scoped service reads. Add a future-delay case with actual GET absent
and accepted outcomes; preserve SQL dispatch fences. Add a focused pure planner
assertion so a regression is diagnosed before requiring PostgreSQL.

## Execution and validation

Extend `tests/support/provider_effects.rs` with one private module and keep the
existing 66-entry live inventory. Ordinary planner tests live in
`src/delivery/worker/tests.rs`. No runtime or migration change is planned.
Use scratch `/tmp/batter-provider-proof.L0VWIo`; restart the existing disposable
PostgreSQL18 clusters on35471/35472 with a task-owned role, and clean up only
that role and those server processes after checking zero remaining databases
and sessions. Retain their data directories.
Run both `scripts/verify.sh` toolchains and explicit `scripts/test_reference_live.sh`,
five rebuilt HTTP smoke profiles per toolchain, independent scoped review, and
Jig work check/evidence/gates. Freeze code during independent review and Jig.

## Outcomes & Retrospective

Implementation is test-only. Focused live outcome coverage passed in 36.57s.
Both complete Rust matrices and ten rebuilt HTTP profiles passed. Both full live
matrices passed (92.89s / 95.94s), plus maintenance and state probes. The first
minimum-toolchain run failed an existing pool-setup probe; isolated and full
unchanged reruns passed. Its precise cause remains unconfirmed and the original
failure is retained in docs/validation.md. Two independent scoped reviews found
no actionable issue. All five Jig targets passed and are fresh; api:test receipt
is `receipt_01M2RH08AHXX96MJEETRFYQT3M`. Task roles were removed after zero
database/session checks; both servers stopped with directories retained. New
nested test files are already covered by exhaustive Jig input roots; no contract
changes are required. No production code, migration or public API changed.
