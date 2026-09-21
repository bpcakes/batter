# Profile setup review fixes

Owning Bead: `batter-oth`. Address only PR #4's setup-error redaction and
case-insensitive setting-key validation; preserve transaction semantics.

## Progress

- [x] Trace native errors through SQLx's two pool hooks independently.
- [x] Reproduce marker disclosure and case-variant duplicate acceptance.
- [x] Protect the shared setup boundary, normalize keys, document logging limits.
- [x] Run focused PostgreSQL 18 regressions, unit/build checks and paired suites.
- [x] Refresh coordinated pins and obtain exact implementation-head CI evidence.

## Surprises & Discoveries

SQLx logs hook errors via Display. Batter already has `SqlxFailure` with safe
Debug/Display and deliberate native-cause access. Private atomic/snapshot setup
already uses that wrapper. The public setup method is the missing boundary.

## Decision Log

Reuse `SqlxFailure` inside SQLx's Configuration error at the complete public
`reset_and_apply` boundary. Keep private apply/verify and native classification
unchanged. Normalize names, never values. No review-fix-loop or redesign.

## Outcomes & Retrospective

The pre-fix unit test accepted `app.tenant`/`APP.TENANT`; PostgreSQL 18.6 live
tests reproduced marker disclosure in direct formatting and both actual SQLx
hook log events. After the fix all four focused live regressions and four
profile unit tests pass; clippy for batter-sqlx passes. The missing-role alternate
trigger returns native SQLSTATE 22023 on this server (not 42704), retained intact.
Rust 1.94.0 and 1.98.1 verification and all five HTTP smoke modes on each passed.

Independent read-only candidate review found no actionable findings; it checked
both repositories and SQLx 0.9's hook implementation without running tests. Full
Batter PostgreSQL live checks passed (88 tests, PostgreSQL 18.6). Runledger's two
profile integration tests, source-pin/migration-drift controls, lint and rustdoc
passed. A full Runledger test build exhausted the home filesystem and produced
linker failures, not test results. Its build cache was moved intact to
`/tmp/runledger-profile-build.BzeAorYB/target`; no source was removed. The rerun
passed core/PostgreSQL/runtime suites and test-support library tests, and all
nine external source-archive consumer tests passed. The temporary `target`
symlink was removed after testing; the cache remains recoverable at that path.
Rust 1.98.1 verification and five HTTP smoke modes passed. Jig `verify` and its
current-input `api:test` receipt passed. Hosted CI passed on Batter `218245d`
and Runledger `23dd188`; any later evidence-only head requires its own CI result.
The final evidence-only head's hosted status will be recorded in the PR follow-up
rather than adding a self-referential commit hash to this plan. The disposable
PostgreSQL container was stopped and removed; retained build artifacts remain
at the documented `/tmp` path. No transaction API or lifecycle behavior changed.
The paired foundation is `d7728e633b3b767edde8aea40daf358ad4917ea4` and Runledger
companion is `23dd1880f3be395ca6e53f93adc07442f43c9f78`.

## Execution and acceptance

Edit `crates/batter-sqlx/src/profile.rs`; extend `tests/profile.rs` and add an
`atomic_live/profile.rs` module registered in `scripts/sqlx_live.py`. First run
`cargo test -p batter-sqlx --test profile` and the new ignored live tests against
PostgreSQL 18: expect the old implementation to fail marker/duplicate checks.
After the patch, require safe Debug/Display, observable safe after_connect and
before_acquire log events, retained native marker causes, rejected case-variant
duplicates, and unchanged mixed-case values. Run both supported Rust verification
scripts and `scripts/test_sqlx_live.sh`, then the companion Runledger checks.
Record exact PostgreSQL version. A read-only fresh reviewer checks this bounded
diff once. Commit the foundation before repinning Runledger; commit the companion
before updating Batter's workflow pins. Do not merge or publish. On failure retain
diagnostics and correct only these changes; never reset unrelated work. Stop only
the explicitly named task-owned diagnostic container after verification.
