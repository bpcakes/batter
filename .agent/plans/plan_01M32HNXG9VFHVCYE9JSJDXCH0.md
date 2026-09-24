# Operation authority cutover

Owning Bead: batter-tc9w.1. PINNED_BASE: 4820e57899e63943a7458e9057eba34903bc2c83.

## Purpose
Make received operation contexts observation/execution capabilities without shared-scope cancellation or independent root construction.

## Progress
- Task and epic creation/claim were committed before implementation.
- Merged `origin/master` through `70cc6a0` in `4820e57`; this review starts from that merge commit.
- Initial implementation was committed as `9f04aeb` with a clean tree. Round 1 native review found five migration/build findings. Repairs are validated and await an additive fix commit; the pinned base has not moved.

## Surprises & Discoveries
Native codex review --base accepts the full pinned ancestor SHA. Tracker reconciliation imported seven missing rows without changing existing issue payloads. The earlier external Runledger foundation pin obstacle ended when native Runledger sources moved into this workspace; the post-merge workspace check reached unfinished operation tests instead.

Round 1 review reached all targets and found private cancellation calls in tests, a missed standalone Runlimit consumer, an external-doctest path, a stale rustdoc link and unused imports. The API-level test repair makes process admission return an owner so callers can retain explicit cancellation authority. A later additive tracker reconciliation restored four JSONL-only rows and three newer rows into the local database without deleting or rewriting existing issue payloads.

Jig's first work check found `startup_signals.rs` grew beyond its prior file budget. A local context helper preserved the same independent root budgets, and the targeted signal tests and file-budget gate passed. Two later direct Rust 1.98.1 runs hit different native Runledger Testcontainers startup failures; both exact failing cases passed immediately in isolation, and a third full direct run passed. No native test or Docker guard was weakened.

## Decision Log
Use non-cloneable OperationOwner with explicit context projection; retain explicit cancellation only on owners and library-owned command controls. Use opaque RootDeadline for independent roots/admission; derive children from complete parent contexts. Root creation remains an explicit application-root authority. No global ambient-parent claim. Drop of a standalone owner does not add implicit cancellation/cleanup.

## Outcomes & Retrospective
Pending the additive fix commit and cumulative native review. The final-tree `verify.sh` matrix passed on both Rust 1.98.1 and 1.94.0 on macOS arm64. All five documented HTTP process smoke modes, the 12 HTTP example tests, and the `process_owned` and `operation_budget` examples passed after those runs. Jig's `api:test`, Clippy, formatting, contract and file-budget gates passed with fresh plan evidence. Focused core tests, Runlimit process-cancellation tests, strict workspace Clippy, rustdoc and the standalone Runlimit facade consumer also passed after the first repair set. No hosted execution or fresh-agent usability evaluation is claimed.

## Context and orientation
operation.rs owns deadline/cancellation boundaries; lifecycle/capability.rs admits roots. command/driver.rs owns retained finite work. Adapters consume the context without requiring cancellation authority.

## Plan of work
Change the capability API, migrate workspace callers, add independent failure and compile-fail tests, update current contracts/status/migration guidance. Consumer migration is part of this task so the review boundary builds coherently.

## Concrete steps
Implement OperationOwner and RootDeadline; restrict context construction/cancellation; derive controlled children; preserve command-specific cancellation. Migrate call sites and execute focused checks followed by both verify.sh toolchains, HTTP smokes, and final Jig gates. Commit implementation before native review of the pinned cumulative diff. Commit any repair in a new commit before another full review.

## Validation and acceptance
Contexts cannot cancel shared scope or create roots; child cancellation isolates parent/siblings and deadlines clamp. Root witnesses do not reset absolute time. Preserve inert factories, interruption precedence, retained SQLx completion, cleanup isolation and destruction dispatch. No review findings remain and worktree is clean before closure. Fresh-agent usability evaluations remain unexecuted.

## Idempotence and recovery
No storage migration. Changes remain on the feature branch. Never rewrite commits during this task. Stop on recurring findings, architectural escalation, unexpected worktree changes or unavailable full-scope native review; summarize and remove the external findings file.

## Interfaces and dependencies
Core cannot depend on adapters. OperationContext remains the adapter argument type; OperationOwner creates contexts and retains cancellation authority. OperationAdmission creates explicit process-associated roots from RootDeadline. Rust 1.94 and 1.98.1 remain supported.


## Previous stopped attempt

The previous attempt stopped before review at base `49805716154083774f40ea0e02d062fecfd1e44a` because the then-external pinned Runledger source guard rejected modified Batter foundation sources. That failure and the decision to stop remain in Bead `batter-tc9w.1`. The later workspace import removed that external source-identity check from the current dependency graph. No cached dependency was edited and no guard was bypassed.

After the merge, `cargo check --workspace --all-features --all-targets --locked` reached the partially migrated tests and failed on calls to the newly private `OperationContext::cancel`. The current review workflow commits the intended work from the new pinned base, then repairs verified findings in separate commits. No passing validation or review is claimed here.
