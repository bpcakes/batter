# Retain fixture ownership across body failures

This ExecPlan follows `.agent/PLANS.md`. Owning Bead: batter-4jz, reopened for the user-requested review fixes. The earlier plan remains historical evidence. `batter-kjl` retains its remaining detached-session, cleanup-owner-loss and deferred-failure work; do not close it for partial delivery.

## Purpose / Big Picture

A reference probe should express acquisition and assertions without manually repeating pool closure, lease disposal and deferred drain on every return path. An owned runner will retain acquired resources outside the body task, join that task even when it panics, finish every resource, drain upstream cleanup and return all errors. Stable migration fingerprints will reuse the two schema templates and one stable rejection-control template across repeated runs.

## Progress

- [x] (2026-09-09) Research pinned upstream cleanup, cache and native pool semantics; adjudicate review findings.
- [x] Implement owned fixture runner and migrate reference consumers.
- [x] Cover failure, ownership, cache and negative-observation gaps with real PostgreSQL tests.
- [x] Update contracts and two-toolchain/live/HTTP evidence; complete final Jig gates and closure receipts with this plan.

## Surprises & Discoveries

The upstream cleanup queue has independent worker threads. Omitting drain loses completion/error observation; it does not imply queued cleanup never runs. Templates persist beyond template handles, and external shutdown is a no-op. A UUID revision causes unbounded cache identities on a reused endpoint. SQLx close waits for checked-out tracked connections and returns unit; closed pool clones can remain as read-only observations.

## Decision Log

Use a single adapter-owned runner with a borrowed fixture scope and an independently retained resource registry. The callback borrows the scope so allocation cannot escape into a detached task. Register a database lease and each native lazy pool before polling connection acquisition. Retain ownership outside the assertion-bearing body task. Join its returned failure or panic, close every registered pool, then await each lease's cleanup and finally drain/shutdown. A cancellable wait borrows the same runner handle; it does not abort work. Dropping that handle detaches the owned driver; runtime death still has no guarantee. Native detached descendants and unacknowledged retired sessions remain caller responsibilities, not an inferred quiescence claim.

Keep low-level existing APIs compatible but mark their manual sequencing boundary clearly. Use the higher-level runner in reference consumers. Keep domain SQL and catalog observations in tests. Keep the shared observer explicitly bounded at suite level, outside declared per-lease pools. Preserve both errors via generic finish/report results without printing contents.

Use stable setup revision for template tests. A callback may run zero times for a persisted cache hit; unchanged retained reuse must add zero calls. Verify state through independent reads and verify stable template names across a second suite. Do not implement another cache or delete databases by prefix.

## Outcomes & Retrospective

Implemented the owned runner and migrated reference consumers. Both supported toolchains passed 555 test/doctest executions, twelve reference live cases, ten SQLx live cases and five HTTP profiles each. Stable before/after catalog inventories retain only three intended templates. Final Jig gates and closure receipts are recorded against this plan. Documentation counts are local editing mistakes; error/cleanup sequencing is an ownership design defect in consumer composition. The fix reduces the number of places that must remember cleanup order.

## Context and Orientation

`crates/batter-sqlx/src/test_support.rs` currently exposes manual suite acquisition and fixture finish. `examples/reference-service/tests/support/fixtures.rs` repeats cleanup tails that early question-mark returns and assertions can bypass. The feature is unpublished and optional; core and generic support remain independent. The pinned harness is 3d525e6fc5745ce2e2437c7997de5cccdecff4ac. Its cleanup workers run independently; drain is a completion barrier. `scripts/reference_live.py` requires every named ignored reference test and a non-filtered successful summary.

## Plan of Work

Add a private runner implementation under the public test_support module. Define FixtureScope, FixtureDatabase access handles, FixtureRun and a report retaining body/join and all cleanup errors. FixtureSuite::start creates the body/driver tasks before returning the waiter. Add native Tokio runtime features to the optional feature. The registry holds leases and pool clones before application callbacks run, allowing panic and partial-acquisition cleanup. Do not hold its mutex across awaits or application callbacks.

Migrate the original empty-database helper and new probes to the runner. Capture external catalog observation as a separate result after cleanup, then combine it without losing the runner report. Join lock operations and release blockers before asserting observations. Replace random template revision with stable inputs. Repair only the two unrelated six-count edits and document feature-enabled offline commands.

Add live controls for partial multi-pool acquisition (a real SQL error in the second pool initialization), sibling acquisition failure, assertion panic, cancelled/resumed wait while a checked-out connection prevents cleanup, real simultaneous body/lease-cleanup error using upstream's acknowledged catalog-lock timeout method, foreign-template rejection, wrong-blocker timeout and pool-close-before-cleanup ordering. Preserve original migration and SQLx suites. Keep the existing external process watchdog. Keep actual failure injection isolated from other tests with explicit coordination.

## Concrete Steps

From `/home/aa/Documents/batter`: run `cargo test -p batter-sqlx --features test-support --locked`, `cargo clippy --workspace --all-features --all-targets --locked -- -D warnings`, and the updated reference live runner against a dedicated UTF-8 PostgreSQL 18 server. Do not export DATABASE_URL into workspace compilation; scope it to the independent SQLx live runner. Repeat reference runs on the same endpoint to prove stable cache reuse. Run both root verify.sh toolchains, both reference and SQLx live runners, rebuild HTTP and run all five smoke profiles. Inspect Jig evidence/gates, run final work check and finish after metadata is complete.

## Validation and Acceptance

A real checked-out connection must keep the database present while runner cleanup is pending; cancellation of one bounded wait must leave the same driver available, and release must permit cleanup and independent catalog absence. Failed second-pool initialization and failed sibling acquisition must still close successful pools and delete acquired databases. Body panic must be reported without skipping cleanup. Actual catalog-lock timeout must coexist with the original body error in the report; release the lock and use upstream owner-aware cleanup for the injected residual. A wrong blocker must time out; the correct blocker must be observed before release. Both cold and warmed template runs must preserve exact state and stable database identities without creating new template identities. Offline controls reject bad summary counts and omitted cases.

## Idempotence and Recovery

Keep provisioning external. Use a dedicated disposable test cluster for catalog-lock failure injection and restore locks/connections before stopping it. Never delete by prefix. Keep all returned causes and distinguish completed cleanup from pending driver observation. No arbitrary task joining, detached-session quiescence, async-drop or runtime-death guarantee is introduced.

## Interfaces and Dependencies

The native SQLx/harness boundary remains explicit. A borrowed callback returns a boxed Send future, allowing scope borrowing without permitting resource-acquisition scope escape. FixtureRun::wait borrows the handle and caches its result; a convenience consuming wait may return the report, but cancelling it loses observation, not the independently running driver. No new executor, provisioning engine, repository abstraction, global subscriber or panic hook is introduced.
