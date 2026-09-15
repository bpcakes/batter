# Refresh final readiness closure evidence

This ExecPlan is a living document maintained under `.agent/PLANS.md`. It is a
tracker-only follow-up to closed Bead `batter-isq` and completed implementation
plan `plan_01M2HWT4TYBHKZ8E57V9135B0F`. Its Git baseline is
`aeab19992a14adf7be0a493ba18f031172fd1d59`.

## Purpose / Big Picture

The readiness implementation and its complete verification are finished. The
owning Bead then received its required evidence comment and was closed, changing
`.beads/issues.jsonl` after the original Jig plan had been finished. Because the
repository-wide contract and file-budget gate compare the exact final worktree,
that expected tracker mutation made the closed plan's gate stale. This follow-up
refreshes only those repository checks and proves the final exported tracker
state is covered. It makes no source, API, behavior, documentation, or Bead
change.

## Progress

- [x] (2026-09-15 07:12Z) Detected that the post-plan Bead comment and closure made the exact-tree repository gate stale; Rust inputs and their passing receipts remained unchanged.
- [x] (2026-09-15 07:15Z) Ran `scripts/jig work check`; Jig kept evidence plan-scoped and therefore executed all five targets rather than reusing receipts across the closed-plan boundary. Every target passed under target-validation receipt `receipt_01M2HYQR1CF6ZV517WKKGCAGST`.
- [x] (2026-09-15 07:15Z) Confirmed fresh evidence for all 22 product/tracker paths, no unresolved gate, no lockfile change, and no source/documentation mutation during the follow-up; ready to finish without another Bead edit.

## Surprises & Discoveries

- Observation: Jig session-state files do not participate in the product-tree comparison, but the Beads export does.
  Evidence: after the original plan was finished and Bead `batter-isq` closed, `scripts/jig work evidence --plan-id plan_01M2HWT4TYBHKZ8E57V9135B0F` changed from fresh to stale while `git status` showed no later source or documentation edit.
- Observation: target receipts are not reused across separate plan identities, even when their declared inputs match a just-completed plan.
  Evidence: `scripts/jig work check --plan-id plan_01M2HYK4GYMDB7WG4RWQD5AGAX` executed `api:clippy`, `api:fmt`, `api:test`, `repo:contract`, and `repo:file-budget`; all passed in run `run_01M2HYP0B8Y45KA0Q5JG3NPGFP`.

## Decision Log

- Decision: create this narrow follow-up plan instead of reopening or altering the completed implementation plan.
  Rationale: Jig explicitly directs follow-up changes after a plan is closed into a new work plan, and the only remaining change is final tracker evidence.
  Date/Author: 2026-09-15 / Codex
- Decision: do not rerun Cargo validation unless Jig identifies changed Rust inputs.
  Rationale: the Bead comment and status changed no declared Rust input; the full Rust 1.98.1 and 1.94.0 matrices and all ten HTTP smokes already passed for the same product bytes.
  Date/Author: 2026-09-15 / Codex

## Outcomes & Retrospective

The final worktree, including the closed Bead and its evidence comment, passed
all five Jig targets under target-validation receipt
`receipt_01M2HYQR1CF6ZV517WKKGCAGST`. Rust receipts could not be reused across
the new plan boundary, so strict Clippy, formatting, and the complete API test
matrix executed again and passed alongside repository contract and file-budget
checks. No source, API, behavior, documentation, dependency, lockfile, or Bead
state changed during this follow-up.

## Context and Orientation

The implementation lives in `crates/batter/src/readiness.rs`,
`crates/batter/src/health/observation.rs`, and
`crates/batter-axum/src/readiness.rs`. It was fully implemented, documented, and
validated by the preceding plan. `.beads/issues.jsonl` now contains both the
pre-existing user-owned history and the new closed `batter-isq` record with its
evidence comment. This plan must preserve all of it.

The original final Jig gate was fresh under repository receipt
`receipt_01M2HYES9HN6NV75DCADHYET3J` before the Bead close. This follow-up exists
only because exact-tree evidence properly noticed that later tracked mutation.

## Plan of Work

Run the default connected check for this plan. Jig should reuse `api:clippy`,
`api:fmt`, and `api:test` because their declared inputs did not change, and it
should execute `repo:contract` and `repo:file-budget` against the final tree.
Then read both evidence and gates. They must report the required `verify` gate
fresh with no unresolved gates. Review `git diff --check`, the absence of a
`Cargo.lock` diff, the closed Bead, and the final status. Finish this Jig plan;
do not edit the Bead or product tree afterward.

## Concrete Steps

Work from `/Users/aa/Documents/batter`:

    scripts/jig work check --plan-id plan_01M2HYK4GYMDB7WG4RWQD5AGAX
    scripts/jig work evidence --plan-id plan_01M2HYK4GYMDB7WG4RWQD5AGAX
    scripts/jig work gates --plan-id plan_01M2HYK4GYMDB7WG4RWQD5AGAX
    git diff --check
    git diff --name-only -- Cargo.lock
    git status --short

If evidence is fresh, update only this excluded plan record with the result and
finish through `scripts/jig work finish`. Do not run another `br` mutation.

## Validation and Acceptance

Acceptance requires a fresh `verify` gate over the final tree with no unresolved
gate. Repository contract and file-budget targets must pass. Unchanged Rust
receipts should be reused rather than rerun. `git diff --check` must pass,
`Cargo.lock` must have no diff, Bead `batter-isq` must remain closed, and no
source or documentation byte may change during this follow-up.

## Idempotence and Recovery

The checks are read-only and safe to rerun. If Jig finds an unexpected changed
Rust input, stop and inspect rather than claiming reuse. If a repository check
fails, repair only the evidenced tracker or policy problem under a new scoped
change; do not weaken the contract or reset user-owned Beads history.

## Artifacts and Notes

The preceding plan recorded both complete toolchain matrices, ten process smoke
profiles, focused tests, compile-fail coverage, and the implementation design.
This follow-up adds no new behavioral claim.

## Interfaces and Dependencies

No interface or dependency changes are permitted. `Cargo.lock` must remain
unchanged. The only intended artifacts are Jig plan/session/receipt records.
