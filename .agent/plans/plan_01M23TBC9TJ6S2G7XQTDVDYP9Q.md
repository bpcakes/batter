# Reconcile reference compatibility with committed operational helpers

Owning Bead: batter-4t6, user-requested integration follow-up.
Baseline: 5bf943526274ff349561648133f4ed06fb17837d.

## Progress

- [x] Preserve the original compatibility delivery in a recovery stash.
- [x] Fast-forward this branch to main and resolve shared files without changing main's Rust implementation.
- [x] Retain all four matrix prerequisites and prove failure propagation for each.
- [x] Reconcile package/contract documentation and generate Cargo.lock with Cargo.
- [x] Verify both Rust matrices, both live suites, HTTP and PostgreSQL smokes.
- [x] Record execution scope and audit preserved changes.
- Final gate and session closure are recorded in Jig's append-only state.

## Surprises & Discoveries

Main's 5bf9435 includes completed SQLx, startup and health deliveries, on top of
7601916's shutdown-report/process fixes. Both sides added a third prerequisite
independently. The reconciled matrix has four; its failure-table tests cover
each prerequisite and both runtime profiles. Shared documentation had stale
example-only SQLx wording that now names the optional adapter.

## Decision Log

Keep compatibility probes separate from reusable operation helpers. Preserve
main's Rust source byte-for-byte, all reference test sources, both documentation
histories and append-only receipts. The combined graph has six unpublished
Rust-1.94 packages and one SQLx 0.9 version. No new upstream API is introduced.
Main's checkout and external library sources are not edited. No new commit,
publication or deployment is authorized.

## Outcomes & Retrospective

Both verify.sh runs passed 548 Rust test/doctest executions, zero failures and
17 explicitly ignored live cases, plus 38 Python controls. Four reference and
ten adapter live cases passed on both toolchains against dedicated PostgreSQL
18.6. The three lifecycle live cases and five HTTP plus two PostgreSQL smoke
profiles passed on rebuilt default-toolchain binaries. Linux-only evidence;
historical macOS/hosted scope is not broadened. See docs/validation.md.

## Validation and recovery

Reproduce from this worktree with bash scripts/verify.sh and
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh. Select a disposable PostgreSQL 18
endpoint for scripts/test_reference_live.sh and scripts/test_sqlx_live.sh; repeat
both on each toolchain. Build the HTTP and PostgreSQL binaries and run the five
HTTP profiles and SIGTERM/SIGINT PostgreSQL profiles in docs/testing.md.
Inspect scripts/jig work evidence and work gates, then work check for this plan.
A fresh passing api:test receipt is required before work finish.

The original delivery remains in stash 0ca572ccc3163e23b483b5b4b9333ab7227cfb8a
(named Reconcile batter-4t6 with main 5bf9435). Do not apply it over the reconciled
files. Inspect or extract it in isolation if recovery is needed. Verification
logs are in ignored .agent/tmp/reconcile-5bf9435/. Cleanup only the dedicated
batter-reconcile-5bf9435 container after observing no remaining harness databases.
