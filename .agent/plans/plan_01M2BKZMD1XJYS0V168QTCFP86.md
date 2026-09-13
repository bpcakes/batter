# Add generic PostgreSQL schema and authority verification

This living ExecPlan implements Bead `batter-7r3.5` under the `batter-7r3`
outcome and follows `.agent/PLANS.md`. The checker will let an application run
read-only startup verification against the actual PostgreSQL login it will use,
without taking migration locks, changing schema state, or hiding connection
ownership. It must be honest about the PostgreSQL 18 surfaces it inspects and
about unsupported or incomplete observations. User handoff preference: Astra
keeps planning, milestone reviews and blockers; Luna Max workers perform
routine commands, coding, tests, logs and polling.

## Purpose / Big Picture

After this change a consumer can pass a native `&mut PgConnection`, a required
migration bundle, and an explicit generic authority policy to Batter's optional
SQLx adapter. The result distinguishes a policy match, violations, and an
incomplete/unsupported inspection. A caller may keep the connection in its
existing `PgLease`, so cancellation and native disposition remain visible to
the caller. Application-specific migration names, grant manifests and extra
schema invariants remain in the application.

## Progress

- [x] (2026-09-12) Read repository and adapter guides, Bead scope, existing
  lease/fixture ownership, and the adopter's schema and authority checks.
- [x] (2026-09-12) Claim `batter-7r3.5`; record clean baseline and lock hash.
- [x] (2026-09-12) Recheck SQLx 0.9 and PostgreSQL 18 semantics against local
  upstream sources and primary references; record coverage boundaries.
- [x] (2026-09-12) Add generic verification data types, read-only snapshot
  execution and redacted native error retention.
- [x] (2026-09-12) Add migration, policy, authority, failure-path and
  blocked-query/disposition tests; the live verification control uses a
  temporary SQLx-shaped ledger and preserves it across checks.
- [x] (2026-09-12) Add a runnable generic example and update contracts, status
  and references, explicitly retaining adopter-specific checks.
- [ ] Run applicable offline/live checks, both toolchains, HTTP smokes and Jig
  evidence/gates; leave the Bead open for parent review and leave changes
  uncommitted.

## Surprises & Discoveries

- The existing adapter already exposes `PgLease::connection`, so the approved
  borrowed-connection design can avoid a second pool/lease abstraction.
- The adopter's authority SQL is broader than the first generic policy surface:
  it includes application Runledger history, exact routines, Runlimit column
  exceptions, trigger/function ownership, view/materialized-view/foreign-table
  behavior, and application-specific schema checks. Those remain adopter-owned
  and must not be silently claimed by Batter.
- SQLx 0.9 pool close and lease return do not establish remote PostgreSQL
  termination; the blocked-query regression must observe local disposition and
  remote behavior separately.
- The local PostgreSQL 18 instance exposed the authority scan's default PUBLIC
  database/schema grants and reachable role attributes as findings until the
  live policy explicitly allowed them. This confirms that the generic policy
  is additive and exact rather than an implicit safe baseline.

## Decision Log

- Decision: expose the primary checker over caller-owned `&mut PgConnection`.
  Rationale: it cannot hide a raw `PoolConnection`, and the caller can place it
  inside `PgLease` and choose return or retirement after cancellation or a
  completed read-only transaction. Date/author: 2026-09-12, approved parent.
- Decision: run catalog and ledger reads inside one explicit read-only,
  consistent PostgreSQL snapshot. Rationale: role/catalog changes after the
  observation remain outside the guarantee and all findings share one view.
  Date/author: 2026-09-12, implementation decision.
- Decision: represent unsupported and missing observations explicitly instead
  of converting them to a passing result. Rationale: no catalog access or
  uninspected requested surface can establish `WithinDeclaredPolicy`.
  Date/author: 2026-09-12, Bead acceptance.
- Decision: keep migration and authority policies generic and caller-supplied.
  Rationale: the Pancake adopter retains its extra Runledger, schema, routine
  and application grant checks. Date/author: 2026-09-12, repository contract.
- Decision: ownership allowance subsumes the owner's implicit object privileges
  for the matching policy entry. Rationale: ownership is reported as one
  explicit capability; requiring callers to enumerate every implicit owner ACL
  would obscure rather than narrow the declared authority. Date/author:
  2026-09-12, implementation worker.
- Decision: report unsupported SECURITY DEFINER bodies, extension objects and
  role defaults even when the caller permits the declared ACL result. Rationale:
  the adapter must distinguish a bounded generic verdict from application
  invariants; `require_complete` turns those limits into `Incomplete`.
  Date/author: 2026-09-12, implementation worker.

## Outcomes & Retrospective

Implementation is complete and remains uncommitted for parent review. The
public `batter_sqlx::verification::verify` API accepts `&mut PgConnection`,
checks the declared migration and authority policy in one explicit read-only
repeatable-read snapshot, awaits rollback, and returns structured status,
findings, unsupported surfaces and redacted native failures. Offline unit
tests cover policy validation, migration compatibility and status transitions;
the ignored PostgreSQL live control covers allowed later rows, missing,
checksum-mismatch and unsuccessful rows, no ledger mutation and required
unsupported coverage. A generic `verification` example and docs identify the
remaining application-owned checks. Record both supported-toolchain evidence,
Jig receipts and any live prerequisite limitations here after the final gates;
do not close the Bead or commit from this worker.

## Context and Orientation

`crates/batter-sqlx/src/lib.rs` owns SQLx 0.9 PostgreSQL lease disposition and
`PgLease` exposes `&mut PgConnection`. `crates/batter-sqlx/src/test_support.rs`
and `src/test_support/runner.rs` own optional PostgreSQL fixture pools and
retained cleanup reports; they are not the production verification API.
`crates/batter-sqlx/tests/` contains offline contracts and explicitly selected
ignored live cases. `crates/batter-sqlx/examples/owned_pool.rs` is the existing
adapter example. The root is Unix-only and the adapter's minimum Rust version is
1.94.

The downstream reference at `/home/aa/Documents/pancake-studio` has
`crates/pancake-studio-db/src/schema.rs` plus `schema/unsafe_role.sql` and
`schema/role_ready.sql`. It checks an application migration ledger, privileged
function search paths, session-user-rooted role reachability, ownership,
table/column/sequence privileges, grant options and protected parameters. Its
application-specific coverage remains local.

## Plan of Work

Add a default `batter_sqlx::verification` module with public rustdoc for
migration expectations, extra-migration compatibility, identifier/object
policy inputs, structured findings, unsupported surfaces and the final status.
Use bound values or validated catalog OIDs for relation/column references; do
not interpolate unchecked SQL identifiers. Keep native `sqlx::Error` values in a
redacted adapter error and keep `Display`/`Debug` safe.

Start one read-only repeatable snapshot on the borrowed connection. Read and
validate the selected migration ledger's shape and rows against the required
bundle and explicit extra policy. Then inspect `session_user`, effective role
reachability through inheritance/SET/ADMIN, role attributes, PUBLIC grants,
ownership, object/column/sequence privileges and grant options, protected
parameter authority, and the declared routine/security-definer surface. Report
uninspected extensions and other unsupported PostgreSQL 18 surfaces explicitly.

Add offline policy/data tests and ignored PostgreSQL 18 live cases for every
Bead acceptance control, including a one-slot blocked query whose waiter is
cancelled while the caller retains explicit connection disposition. Add a
generic runnable example using synthetic policy names and no adopter schema.
Update `crates/batter-sqlx/README.md`, `docs/guarantees.md`,
`docs/integrations.md`, `docs/status.md`, `docs/references.md` and the owning
Bead notes/acceptance only as evidence warrants. Record that Pancake retains
checks outside Batter's declared coverage.

## Concrete Steps

Run from `/home/aa/Documents/batter`:

    br show batter-7r3.5 --json
    scripts/jig work start --title "PostgreSQL schema and authority verification" --body-file .agent/plans/batter-7r3.5.md --json
    cargo fmt --all -- --check
    cargo test -p batter-sqlx --features test-support --locked
    cargo test -p batter-sqlx --all-targets --all-features --locked
    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh

Run the explicit SQLx live runner only with its documented disposable
PostgreSQL 18 prerequisites. Run the documented five HTTP smoke profiles on
both toolchains. Inspect `scripts/jig work evidence` and `scripts/jig work
gates` before reusing receipts; run the final `scripts/jig work check` and
required gates for the active plan. Do not commit or close the Bead here.

## Validation and Acceptance

Offline tests must prove policy construction, safe identifier handling,
migration compatibility decisions, structured status transitions, redacted
errors and that the public API does not require `PoolConnection` ownership.
Live tests must prove missing/checksum-mismatched/unsuccessful migrations,
explicitly allowed later migrations, no DDL or migrator-lock acquisition,
direct/inherited/SET/ADMIN authority, session-user masking, ownership and
grant-option rejection, table/column/sequence and protected-parameter checks,
PUBLIC grants, two unrelated policy sets, and explicit incomplete/unsupported
results. A blocked one-slot query must show local cancellation/disposition and
bounded pool close separately from eventual backend termination.

The result must never claim universal database security, repair ACLs, infer
rollback or remote cancellation, or pass when requested catalog access is
missing. Existing adopter-only checks must continue to run in the adopter.

## Idempotence and Recovery

All schema verification is read-only and may be rerun against the same database.
Live tests use an explicitly configured disposable PostgreSQL instance and leave
role/schema policy cleanup to their owning fixture. If a live observation times
out, retain the native connection/fixture owner and report the pending state;
do not manufacture success by dropping it. If verification fails to compile,
fix the checked-in API and tests before broad verification. Preserve unrelated
worktree changes and do not reset or overwrite generated lockfiles.

## Interfaces and Dependencies

Use the existing SQLx 0.9 PostgreSQL/runtime-tokio dependency and `PgLease`
without adding a core or generic-test-support SQLx dependency. The primary
checker accepts `&mut sqlx::PgConnection` plus explicit policy/configuration and
returns a structured report or a redacted native/operation error. It may use
SQLx transactions and catalog queries but must leave transaction completion and
connection reuse acknowledged before the caller returns a lease to the pool.
The optional adapter remains independent of Axum, Runledger and the external
postgres-test-harness provisioning package.

Revision 2026-09-12: initial implementation plan created after discovery and
parent approval of the borrowed connection/snapshot design.
