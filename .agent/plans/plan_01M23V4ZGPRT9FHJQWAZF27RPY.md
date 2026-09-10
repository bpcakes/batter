# Compose isolated PostgreSQL fixtures

This living ExecPlan follows `.agent/PLANS.md`. Owning Bead: batter-4jz.

## Purpose / Big Picture

Reference tests will reuse adapter-owned native pool/database ownership, clone retained upstream templates, and prove isolated writes and actual lock waits. Provisioning and template caching remain upstream. The optional feature keeps the harness out of ordinary adapter dependencies.

## Progress

- [x] (2026-09-09) Inspect audited Bead and pinned upstream source; claim delivery.
- [x] (2026-09-09) Implement optional fixture ownership, declared connection limits, ordered fingerprints and lock observation.
- [x] (2026-09-09) Replace duplicated reference ownership; seven required live cases include three new fixture cases.
- [x] (2026-09-09) Both toolchains passed full verification, seven reference and ten SQLx live cases, and five HTTP profiles. Jig verify passed; contracts/evidence updated and delivery completed.

## Surprises & Discoveries

Upstream persists templates across starts as well as caching handles weakly. The initializer-count test therefore uses a fresh per-invocation revision shared by its comparisons. An initial SQL_ASCII disposable cluster rejected existing migration text; reinitialize the test cluster explicitly as UTF-8. Scope DATABASE_URL to the independent SQLx live runner: exporting it to workspace compilation makes upstream SQLx macros query the empty admin database instead of their checked-in metadata. A suite must retain them for reuse. Deferred drain covers prior submissions only. Pool closure does not acknowledge detached backend termination.

## Decision Log

Use a suite local to each Tokio test, holding the harness and template handles. Concurrent fixtures in a test share that suite; no static runtime-bound pools. Store all fixture pools together with the upstream lease. Require callers to join work before finish. Keep cancellation-retained teardown in batter-kjl.

## Outcomes & Retrospective

Implementation and final verification are complete. Both toolchains ran 552 test/doctest executions with zero failures and 20 explicitly ignored live cases; all seven reference and ten independent SQLx live cases and five HTTP profiles passed on both toolchains. Jig verify passed all five targets. The disposable server was stopped after live validation. Four original compatibility cases remain, with three new fixture cases. Native error formatting is redacted; returned body-error cleanup and offline arithmetic/fingerprint/redaction checks are executable.

## Context and Orientation

`crates/batter-sqlx` owns the optional SQLx adapter. Its `PgLease` is a checked-out connection, distinct from an upstream disposable `DatabaseLease`. `examples/reference-service/tests/support/mod.rs` currently repeats native pool closure and lease cleanup. The existing four ignored cases and exact inventory in `scripts/reference_live.py` are compatibility evidence. The upstream harness is pinned at 3d525e6fc5745ce2e2437c7997de5cccdecff4ac with default features disabled.

## Plan of Work

First add public `test_support` behind an optional Cargo feature. Define ordered borrowed migration/bundle inputs, using upstream framed FingerprintBuilder. Define a checked connection plan from native pool options and reserved standalone capacity. Validate before acquiring a database or pools. A suite retains native upstream templates, and returns fixtures owning all configured pools and one database lease. On returned connection failure close prior pools and await lease cleanup, preserving both failures. Fixture finish awaits pool closure before lease cleanup and combines body/cleanup failures. Errors redact native contents in formatting but retain inspectable causes. Expose bounded observation of a known waiter/blocker pair through a caller-owned independent SQLx connection.

Next migrate the existing empty-database helper without changing migration probes. Add retained-template reuse/changed-input/isolation and one-slot lock-operation cases in reference tests. The consumer applies exact declared SQL and closes initializer pools on success and returned error. Each test runtime owns its suite. Tests join operations before fixture finish, independently query pg_database after cleanup, and drain only after producers finish. No retired session is needed for the new happy-path lock test. Keep all four compatibility cases and all ten SQLx disposition cases.

Finally update package guides, integration/guarantee/status/testing/compatibility documentation and validation evidence; update the Bead and living plan. Finish Jig after reviewing fresh evidence. Do not commit or publish.

## Concrete Steps

From `/home/aa/Documents/batter`, use `cargo test -p batter-sqlx --all-features`, then `bash scripts/test_reference_live.sh` with explicitly selected local disposable PostgreSQL 18 POSTGRES_TEST_ADMIN_URL. Run the independent `scripts/test_sqlx_live.sh` with DATABASE_URL. Repeat live runners with RUSTUP_TOOLCHAIN=1.94.0. Run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`. Build the HTTP example and run smoke_http.py default, --signal SIGINT, --deadline, --warn-filter and --warn-filter --deadline. Inspect Jig evidence/gates, run work check, and finish when all applicable gates pass.

## Validation and Acceptance

Offline tests reject overflowing/over-budget plans before opening resources and prove fingerprint sensitivity to ordered inputs, SQL and revision. Live tests prove initializer reuse count, independent changed-input readback, simultaneous distinct database identities and isolated row values with affected-row assertions. A real one-slot SQLx operation waits behind an acknowledged advisory lock; an independently budgeted observer checks pg_blocking_pids for those exact backend identities. Explicit release permits completion, followed by independent data readback. Every fixture pool is closed before cleanup; an independent catalog connection observes database absence. Runner controls reject missing/skipped cases and preserve process timeout. Linux executed evidence does not establish macOS execution.

## Idempotence and Recovery

External harness creates unique disposable databases. Await normal cleanup and then deferred drain. Ordinary owner loss may invoke destructive upstream Drop and is not cancellation-safe. Do not wrap finish in a timeout and claim retained ownership. Do not delete databases by prefix. Preserve returned native errors without logging their contents. On verification failure repair implementation, never weaken the semantic oracle.

## Interfaces and Dependencies

Public module `batter_sqlx::test_support` provides `MigrationInput`, `MigrationBundle`, `template_spec`, `ConnectionPlan`, `FixtureSuite`, `DatabaseFixture`, and bounded `observe_blocked`. Native PgPoolOptions/PgPool and upstream harness/template types remain explicit adapter-level boundaries. Optional dependencies are the pinned harness and generic batter-test-support. Reference tests enable the feature as a dev dependency. The core and generic support do not acquire SQLx/harness dependencies; fixture support does not acquire Runledger.

Revision 2026-09-09: implementation added a returned-body-error cleanup case and canonical retained-template lookup. Catalog absence is checked before deferred drain. Validation setup now uses explicit UTF-8 and scopes DATABASE_URL to its live runner. These changes prevent persisted templates, deferred cleanup or ambient compile-time database selection from producing misleading evidence.

Completion evidence: `.agent/tmp/batter-4jz/` contains both verification logs, live runs, HTTP profiles, dependency graphs and failed development runs. `docs/validation.md` records exact commands, environment, counts and limitations. No commit or publication was performed.
