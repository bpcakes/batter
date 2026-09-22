# Atomic consumer failure policy — batter-a0qb

Implement requested item 2 as its own PR stacked on PR 12. Items 3 (fail-fast)
and 4 (native-query helpers) remain separate subsequent deliverables; helpers
will use the fixed consumer error type. No persisted schema or dependency change.

## Progress

- Inspected the foundation runner, native intent/queue phases and failure tests.
- Added policy-based runners and fixed-error SQL scopes; both adapter crates compile.
- Added failure tests, consumer examples, contract/status updates and native Send-future proof.
- Full verification passed on Rust 1.98.1 and 1.94.0, macOS arm64.
- SQLx live suite passed 105/105 on each toolchain against PostgreSQL 18.6;
  HTTP build and all five smokes passed on each toolchain.
- Required Jig gates passed; publication follows evidence closeout.

## Surprises & Discoveries

The existing runner retains the first native loss cause independently of callback
handling. Mapping outside that state machine preserves this enforcement. The
application rejection in a failed recovery is separately owned: pass the complete
PgScopeFailure to policy, rather than retaining only its native loss cause.

## Decision Log

Use additive run_atomic_with/run_atomic_profiled_with entry points and a single
PgFailurePolicy<T> trait with associated Error. T names the final transaction
output so commit uncertainty can retain it without type erasure or a static bound.
All five failure methods are mandatory. Ordinary recovered and rolled-back
rejections pass through unchanged. Retain the existing runner as the one lifecycle
implementation. Native phase wrappers consume the foundation scope and preserve
the one-way queue transition. No Clone or Error bound on consumer errors.

ADR-010 review: no public constructors, resource extraction or completion methods;
scope lifetime prevents escape; lost-owner state survives mapped/caught errors;
queue phase has no intent method. Policy controls application error meaning and
may deliberately discard data, which cannot be proved by local types; every
uncertain outcome must nevertheless be implemented explicitly. Arbitrary SQL
remains a documented escape hatch, not a sandbox. Cancellation/panic return no
new rollback claim. Validate these claims with compile-fail and native PG tests.

## Outcomes & Retrospective

Implemented item 2 on branch feat/sqlx-failure-policy, ready for its separate PR
stacked on PR 12. Existing APIs remain compatible. Items 3 and 4 remain subsequent
work; this plan does not claim their implementation.

Executed evidence: bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash
scripts/verify.sh passed, including native Docker tests, doctests, lint and rustdoc.
The exact SQLx live inventory passed all 105 cases on each toolchain using a
dedicated disposable postgres:18 container reporting 18.6. Both HTTP builds and
all five smoke commands per toolchain passed. Cargo.lock is unchanged.

Jig run run_01M35HEDWWDWFQHB49QNE7F8CQ passed api:test, api:clippy, api:fmt,
repo:contract and repo:file-budget. Final api:test receipt:
receipt_01M35J0TV30ZA7VXH1KS9DYXR6. Tracker/docs closeout requires only the cheap
whole-repository policy refresh; source inputs and toolchain identity are unchanged.
An independent read-only contract review found no correctness or scope blocker;
it did not run tests and is not counted as execution evidence.

Initial full linting found conditional non-Send generic futures and test unwraps.
The native policy module now explains its local lint allowance (matching the
existing runner's lending/local callback support), while the executed consumer
workflow must also compile as Send for concrete Send inputs. Test diagnostics use
expect; no semantic assertions were relaxed. An initial live invocation omitted
the required admin endpoint, then an offline test placed in the exact live target
caused its zero-filtered-test check to fail. The offline test now belongs to the
private context unit suite; the exact live inventory passes without relaxing it.
The first Rust 1.94.0 full workspace run encountered a Docker host-port resolution
failure in existing job_read_scope. Its focused three-test rerun and complete
verification rerun passed without code or fixture changes. No Linux verification
claim is made for this implementation.

## Work and validation

Foundation: crates/batter-sqlx/src/atomic_policy.rs delegates atomic_runner.rs.
Native: runledger/runledger-postgres/src/atomic/policy.rs keeps named operations
on existing private transactional SQL implementations. Reexports in both adapters.

Add tests for inference using native question-mark, known rollback, recovery,
caught terminal failure, dropped operation, paired rejection/recovery causes,
commit uncertainty and rollback uncertainty; test required policy methods and
phase ordering with doctests. Run native Runledger policy integration against its
Docker fixture. Update exact SQLx live inventory, package contract, guarantees,
status and owning Bead. No dependency changes needed.

Run focused cargo tests and PG18 live tests first. Then bash scripts/verify.sh and
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh, HTTP build plus five smokes for both
toolchains per docs/testing.md, and scripts/jig work check with this plan. Inspect
work evidence/gates before rerunning a passed command. Do not edit tracked inputs
while Jig checks run. Record only executed evidence and platform/version scope.

## Recovery

Changes are additive and have no migrations. Revert this branch to remove the new
entry points; existing callers remain source compatible. Keep original errors and
tests when repairing failures. Push and open a separate PR; do not merge.
