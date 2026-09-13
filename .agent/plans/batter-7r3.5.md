# Add generic PostgreSQL schema and authority verification

Current guidance (2026-09-13, completed authorized recovery): the original
borrowed-API and intermediate stopped-round sections are historical. The approved
owned-lease design and downstream fit correction are implemented. Both final
toolchain matrices, both exact 47-case live runners, ten HTTP smokes, verifier
example modes and current Jig gates pass. All confirmed review findings are
resolved, including the final required-parameter correction and reviewer
confirmation. The completion section below and docs/validation.md own final
evidence. The generated Jig plan copy remains creation-time history. Changes
remain uncommitted; actual consumer adoption remains a disposable reviewable patch.

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
  lease/fixture ownership, and a downstream consumer's schema and authority checks.
- [x] (2026-09-12) Claim `batter-7r3.5`; record clean baseline and lock hash.
- [x] (2026-09-12) Recheck SQLx 0.9 and PostgreSQL 18 semantics against local
  upstream sources and primary references; record coverage boundaries.
- [x] (2026-09-12) Add generic verification data types, read-only snapshot
  execution and redacted native error retention.
- [x] (2026-09-12) Add migration, policy, authority, failure-path and
  blocked-query/disposition tests; the live controls provision restricted
  LOGIN roles, two fixture policies, and a temporary SQLx-shaped ledger.
- [x] (2026-09-12) Add a runnable generic example and update contracts, status
  and references, explicitly retaining application-specific checks.
- [~] (2026-09-13) Run the final broad two-toolchain matrices and HTTP smokes;
  both toolchains and all five HTTP profiles pass. The final comprehensive
  review did not converge within the two authorized repair rounds: Claude and
  Cursor completed the final pass, while Native Codex exceeded its bounded
  review window and was interrupted. No post-review Jig `api:test` receipt or
  completion claim exists; the Bead remains open with changes uncommitted.

## Surprises & Discoveries

- The existing adapter already exposes `PgLease::connection`, so the approved
  borrowed-connection design can avoid a second pool/lease abstraction.
- A downstream authority implementation is broader than the first generic
  policy surface: it includes application durable-history shape, exact routine
  hardening, column exceptions, trigger/function ownership,
  view/materialized-view/foreign-table behavior, and application-specific
  schema checks. Those remain application-owned and must not be silently
  claimed by Batter.
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
  Rationale: downstream applications retain extra durable-history, schema,
  routine and application grant checks. Date/author: 2026-09-12, repository
  contract.
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

The implementation milestone is complete and remains uncommitted for parent
review. The
public `batter_sqlx::verification::verify` API accepts `&mut PgConnection`,
checks the declared migration and authority policy in one explicit read-only
repeatable-read snapshot, awaits rollback, and returns structured status,
findings, unsupported surfaces and redacted native failures. Offline unit
tests cover policy validation, migration compatibility and status transitions;
the focused PostgreSQL 18.6 matrix covers allowed later rows,
missing/checksum-mismatch/unsuccessful rows, no ledger mutation, restricted
LOGIN reachability, direct/inherited/SET/ADMIN, PUBLIC, ownership,
table/column/sequence/routine grants, protected parameters, two policies,
requested missing objects, unsupported completeness and checker cancellation.
Both toolchains passed the two new live cases and the existing verifier control
against disposable/local PostgreSQL. The final broad verification and all five
HTTP smoke profiles passed on both toolchains; the active Jig plan was refreshed
after these edits. Review, Bead closure and commit remain parent-owned. The
final review stopped non-converged after repair round 2/2; see the durable
stopped-work handoff in `.agent/reviews/batter-7r3.5-stopped.md`. Do not close
the Bead or commit from this worker.

## Stopped-work reconciliation (2026-09-13)

The source and documentation freeze was reviewed against the final working
tree fingerprint `aa61591ea535ed2fc696d5b0af5a687658c71c12485be95cbea16580513c7c11`
before this metadata update. The first two comprehensive review passes had
Claude, Codex and Cursor reports; the final pass had Claude and Cursor reports,
but Native Codex exceeded 30 minutes and was interrupted. The review workflow
therefore stopped with incomplete provider coverage and non-converged findings.

Round 2 was the second and final repair round. No third repair, source/test/doc
edit, commit, task-2 work, Jig mutation or verification rerun is authorized in
this handoff. The broad validation completed before review remains historical
evidence: both `verify.sh` toolchains, ten HTTP profiles, both 29-case SQLx live
matrices, feature/offline tests, strict Clippy/formatting and direct file-budget
all passed. The final review has no fresh post-review Jig `api:test` receipt.

The durable user workflow preference remains: Astra owns planning, milestone
reviews and blockers; Luna Max workers perform routine coding, commands,
testing, logs and polling, receive complete assignments, report only at
milestones or blockers, reuse workers and wait for reports rather than polling
status repeatedly.

## Context and Orientation

`crates/batter-sqlx/src/lib.rs` owns SQLx 0.9 PostgreSQL lease disposition and
`PgLease` exposes `&mut PgConnection`. `crates/batter-sqlx/src/test_support.rs`
and `src/test_support/runner.rs` own optional PostgreSQL fixture pools and
retained cleanup reports; they are not the production verification API.
`crates/batter-sqlx/tests/` contains offline contracts and explicitly selected
ignored live cases. `crates/batter-sqlx/examples/owned_pool.rs` is the existing
adapter example. The root is Unix-only and the adapter's minimum Rust version is
1.94.

The downstream reference verifier checks an application migration ledger,
privileged function search paths, session-user-rooted role reachability,
ownership, table/column/sequence privileges, grant options and protected
parameters. Its application-specific coverage remains local.

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
generic runnable example using synthetic policy names and no application schema.
Update `crates/batter-sqlx/README.md`, `docs/guarantees.md`,
`docs/integrations.md`, `docs/status.md`, `docs/references.md` and the owning
Bead notes/acceptance only as evidence warrants. Record that downstream
applications retain checks outside Batter's declared coverage.

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
missing. Existing application-only checks must continue to run in the application.

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
The optional adapter remains independent of Axum, other lifecycle adapters and
the external postgres-test-harness provisioning package.

Revision 2026-09-12: initial implementation plan created after discovery and
parent approval of the borrowed connection/snapshot design.

## Recovery assessment and proposed completion (2026-09-13)

Consumer: the user deciding how to finish `batter-7r3.5`, then its implementing
agent. This section answers the explicit request to analyze the failed design
and propose completion. It retires into historical evidence when this Bead's
acceptance is actually met. It creates no new delivery scope or tracker item.
No Rust or test implementation changed during this assessment.

### Progress and corrected retrospective

The later three-round repair tranche ended with two high, two medium and three
low accepted findings in comment 75. Both toolchain matrices and the exact live
inventory passed, but acceptance is incomplete. Marking the assistant goal
complete described termination of the review workflow, not completion of the
implementation; it must not be used as delivery evidence. The current Bead is
correctly still in progress. No passing test count establishes absence of the
remaining defects. No fourth repair tranche was started by this assessment.

### Decisions that caused the failure

1. The original borrowed-connection decision solved hidden pool ownership but
   did not establish ownership of transaction state. `verification.rs:65`
   begins and rolls back a transaction on an arbitrary caller connection.
   SQLx 0.9's `PgTransactionManager::begin` consults its managed depth; the
   protocol transaction status is private in `connection/mod.rs`. The prior
   raw-BEGIN probe confirmed adoption/rollback of an empty transaction and
   abortion of a prior-write transaction. A caller instruction to be idle
   would retain the same unenforced boundary.
2. The snapshot decision conflated catalog-row visibility with execution-time
   relation behavior. `authority::inspect` queries version and identities
   before `migration::inspect` obtains any ledger relation lock. A catalog
   flag check is therefore not an invariant on the later SELECT. PostgreSQL
   documents both internal-catalog MVCC exceptions and the value of locking
   before the first snapshot-taking statement.
3. Captured-catalog evaluation is a small reimplementation of PostgreSQL
   authority semantics. INHERIT, SET, ADMIN and CREATEROLE were repeatedly
   repaired independently even though their combined reachability determines
   the verdict. The target-specific ADMIN change addressed one design gap;
   it did not assess the entire execution boundary. Replacing this evaluator
   wholesale with live privilege functions would reintroduce the snapshot
   inconsistency already recorded in `docs/references.md`.
4. The parameter abstraction normalized policy names without checking every
   supplying catalog's identity convention. The loader compares those names
   exactly with `pg_settings.name`; case-normalization needs one consistent
   comparison key across the join, distinct from display spelling.
5. Bounded wall time and linear graph traversal were treated as sufficient
   resource control. `load_roles` and `load_parameters` still use unbounded
   `fetch_all`. The 2,048-role and 48-relation controls exercise particular
   inputs, not a maximum retained-row or byte contract.
6. The process repeatedly ran broad validation before architectural questions
   were settled. File-budget violations discovered only at final Jig checks
   forced another layout change, full validation and review. Duplicated plans
   still describe older stopped rounds. Review agreement also was not reliable
   evidence by itself: two final fixture allegations referenced code absent
   from the reviewed snapshot. These were rejected, correctly, by inspection.

### Recommended decision: verifier-owned PgLease

Change the canonical entrypoint to accept the serving `PgPool`, an
`OperationContext` and `VerificationPolicy`, returning the report with typed
operation/native failures. Exact signature is proposed, not implemented.
The Bead already explicitly permits checker-owned `PgLease` disposition.
Acquire the lease inside the operation factory; do not accept an arbitrary
connection, an already-used lease or an arbitrary initialization callback as
equivalent protected entrypoints. Keep native transaction inspection private.
This is an API cutover in an unpublished working change, not a new framework.

On its newly acquired checkout, acknowledge rollback of residual transaction
state before starting verification; never run that reset against a borrowed
application transaction. Check the managed-depth/native behavior against SQLx
0.9 and reject/retire state that cannot be established, rather than assuming
pool checkout proves idle. Preserve the pool's session-level serving identity
and connection options. The acquisition/reset, read transaction, lock wait,
inspection, rollback and lease disposition share the total operation budget.
Only acknowledged normal completion permits pool return; native failure,
interruption and panic retire the acquired lease. A complete report containing
policy violations is distinct from interrupted or failed database execution.
Retain inspection and rollback errors together. No remote termination or
runtime-death guarantee follows from local retirement.

This deliberately stops supporting inspection of caller-local temporary tables
and arbitrary already-open transactions through the canonical entrypoint.
Application migration history and credentials remain application-owned. Convert
temporary-ledger test setup to isolated committed fixture tables while keeping
every migration oracle. Retain changed-session-authorization and SET ROLE
controls using serving-pool configuration/test setup. Do not delete the
one-slot cancellation control because ownership moved into the verifier.

### Three implementation milestones

1. **Prove the boundary before editing the authority model.** Implement the
   owned entrypoint in `crates/batter-sqlx/src/verification.rs` using the existing
   `PgLease` from `src/lib.rs`. A separate checked-out application transaction
   must retain its pending write and still commit after verification. With a
   one-slot occupied pool, verification must time out without touching that
   transaction. Test residual raw-BEGIN checkout state separately, including
   empty, written and aborted cases. Reuse the real blocked-query capacity
   control in `tests/verification_live_capacity.rs` to prove retirement and
   later pool acquisition, separately from server termination.
2. **Repair the remaining semantic causes together.** In `migration.rs` and
   the execution ordering in `authority.rs`, acquire an explicit ACCESS SHARE
   lock on the validated ledger before any snapshot-taking SELECT, including
   version/identity queries. Use `SET LOCAL row_security = off` for the read;
   this requests an error rather than filtered rows, not privileged RLS bypass.
   Preserve typed missing-ledger findings when early locking finds no table,
   and retain permission/native failures. Bound the lock wait by the same
   operation deadline. ACCESS SHARE allows ordinary migration-row writers;
   do not introduce the migrator advisory lock or permission-repair SQL.
   Prove both DDL orderings with database-observed barriers: DDL before lock
   acquisition must be reflected/rejected, and DDL after acquisition must wait.
   Keep the concurrent ACL snapshot control, moving its synchronization past
   actual snapshot creation rather than relying on the old ledger wait.
   Normalize parameter comparison keys in `authority/database.rs` and test
   TimeZone/DateStyle/IntervalStyle with and without ACL entries, allowed and
   denied implicit SET. Add explicit row and variable-size byte limits to
   catalog materialization, with cap-plus-one rejection before graph/report
   evaluation. Cover exact-cap success and over-cap failure; do not call a
   truncated catalog complete. Bound ACL expansion as well as object counts.
   Do not start a new reachable-only SQL query optimizer. Fix the example's
   typed diagnostics/owner policy and make its long parameter fixture unique.
3. **Demonstrate adoption and close against fixed acceptance.** Update all
   public call sites in the README, runnable example, `tests/postgres_live.rs`,
   verification test modules and integration/guarantee docs. Exercise two
   distinct policies through the new canonical path. For recurring authority
   rules, compare findings with actual permitted/denied PostgreSQL operations
   in disposable state; expected report values alone are insufficient. Include
   INHERIT/SET separation, ADMIN regrant, CREATEROLE and grant-option controls.
   Intentionally remove each critical guard in an isolated temporary checkout
   and require the targeted regression to fail. Then obtain independent review
   of the frozen complete change, resolve reproducible acceptance blockers and
   execute the final required validation. Do not create follow-up Beads to
   remove unmet original acceptance from this task.

### Execution and completion discipline

Use focused adapter tests and strict adapter Clippy during implementation, plus
the native file-budget check before final review. Run both complete
`scripts/verify.sh` toolchains, both exact SQLx live runners, ten HTTP profiles,
the runnable verification modes and the final Jig profile on the settled
source. Reuse fresh evidence only under the repository's existing input and
environment rules; any semantic change after review needs relevant renewed
validation and review. Do not treat another full-suite run as a repair for a
missing oracle. These commands already exist in the plan and repository.

Completion means the original migration/authority positive paths work, the
seven accepted residuals are resolved, the ownership/DDL/capacity regressions
reject their deliberately broken controls, required checks pass and independent
review has no unresolved reproducible acceptance blocker. A review-round limit
is an escalation condition, never implementation success. Source completion
does not authorize a commit or publication. This proposal does not promise a
specific number of future reviewer passes or an unexecuted usability result.

Primary semantics rechecked for this proposal: PostgreSQL 18
[MVCC caveats](https://www.postgresql.org/docs/18/mvcc-caveats.html),
[LOCK ordering and privileges](https://www.postgresql.org/docs/18/sql-lock.html),
and [row_security behavior](https://www.postgresql.org/docs/18/runtime-config-client.html#GUC-ROW-SECURITY),
plus installed SQLx 0.9.0 `connection/mod.rs` and `transaction.rs`.

Revision 2026-09-13: recorded the user-requested causal assessment and proposed
owned-lease completion path. Historical progress and decisions are retained;
the new design and its regression scenarios are proposed and unexecuted.

## Downstream fit correction (2026-09-13)

The user identified the current model consumer as the adoption reference for
future projects. Read-only inspection of its actual database and serving roots
confirms that it uses the immutable Batter pin
`b87708db06640fa9fd81f6d15cd2f2cfa538f7be`, not this uncommitted verifier.
Its working tree was clean and was not modified. Consumer-specific paths and
names stay out of this generic implementation plan; the user-facing assessment
provides direct source links. This is source evidence, not an executed adoption.

### Ownership fit and limits

The consumer constructs native serving pools through `batter_sqlx::pool_in`
with its own login and session settings, then verifies before listener binding.
Its database checks already acquire independent leases; no production
verification path inspected requires caller-local temporary tables or an open
application transaction. The proposed pool-owned entrypoint fits this use.
Use that actual serving pool, not a new owner-credential connection or an
owner session switched to a restricted role. Preserve its connection options,
server statement/lock/transaction timeouts, registered cleanup and safe native
error categories. An outer startup/command deadline must still interrupt the
whole verifier. Context propagation must not require exposing the protected
startup scope's private internals; the application can thread its existing
operation context into its private database facade.

The consumer has distinct public-serving, administrative-serving, producer and
worker policies and two separately owned migration ledgers. These are policy
data and native bundle inputs, not Batter enum variants or hard-coded object
names. Do not invent a requirement that all application/native supplemental
checks share one connection or one transaction: the current consumer already
runs separate checks. Sequential bounded calls suffice where appropriate.

### Two missing generic adoption contracts

Fixing the seven residual findings alone does not make the authority API a
useful replacement for the consumer's reusable checks. Its current SQL verifies
both directions and discovers unexpected objects; Batter only checks an upper
allowance on named objects. These are additional proposed contract requirements,
not claims that the prior declared-object contract already provided them.

1. Separate **required privileges under the current execution identity** from
   **maximum permitted authority reachable from the authenticated login**.
   Required SELECT, column UPDATE used for locking, schema USAGE and routine
   EXECUTE must be satisfied without assuming that application queries issue
   SET ROLE or administer membership first. Reuse captured catalog/ACL data but
   use a current-role inheritance calculation for requirements, not the
   potential SET/ADMIN escalation closure. Validate that requirements do not
   contradict allowances. Report missing privileges distinctly from excess
   privileges so consumers retain their existing operational categories.
2. Separate **inspection scope** from **allowed objects**. Add explicit bounded
   discovery for supported object classes across selected schemas or all
   non-system schemas of the current database. Merely adding discovery is
   insufficient: `authority/requests.rs` currently skips unconfigured relations.
   Apply scope-wide capability defaults and exact object/column overrides to
   discovered objects; this lets a consumer permit ordinary sequence reads or
   ordinary invoker-routine defaults while rejecting unexpected writes,
   ownership, grant options and privileged routine access. Preserve exact
   declared-object mode for narrower consumers. Do not let each application
   enumerate a catalog separately and then claim that both calls share one
   snapshot. Bound discovery/ACL expansion and report unsupported classes or
   capacity failure explicitly. This is a finite PostgreSQL capability policy,
   not SQL callbacks, regular-expression rules or a database-security DSL.

The relevant existing implementation points are `policy.rs::AllowedPrivilege`,
`authority/selection.rs::CatalogSelection::from_policy`,
`authority/requests.rs::inspect_relations` and the actual-grant loop in
`authority/privileges.rs`. The downstream source contains executable
revoke-and-restore required-grant controls, grant-and-revoke excess-authority
controls, unexpected-sequence ownership/grant-option controls and masked-login
tests. Port generic versions of those scenarios rather than deriving new
expected results from this implementation's report.

### What must remain downstream

Retain exact native SQLx ledger shape and primary-key requirements, application
SECURITY DEFINER/search_path/body and trigger invariants, native queue schema
compatibility, deployment grant scripts, business column exceptions and
application error/HTTP mapping. Share generic history comparison where it fits,
but do not replace native queue verification or provisioning with Batter logic.
Additional unsupported ownership classes must remain explicitly checked locally
until supported; do not claim the whole authority SQL can be deleted merely
because selected ACLs moved upstream. Application policy manifests should
remain; duplicate generic graph/ACL mechanics should shrink materially.

### Revised completion evidence

Before calling the design ready for adoption, make a concrete temporary
integration against a disposable checkout of the actual consumer using local
Batter overrides for the relevant workspace packages. Preserve its committed
dependency pin and lockfile. Exercise the real startup path with both serving
profiles, then worker/producer policy representations as relevant, including
missing required grants, unexpected-object authority, login masking, exact
ledger rejection, session settings, blocked-verifier cancellation and cleanup.
Retain distinct schema/authority failure classification before listening.
List the actual generic code removed and the justified application/native checks
retained. If the integration must retain a second generic role/ACL evaluator,
the adoption benefit has not been demonstrated.

After the first successful mapping, modify one generic policy with a new
relation, a required column privilege and an allowed routine. The shared checker
source must not need application-specific edits. This is the bounded evidence
for usefulness in later projects. All such integration/build/live checks are
proposed and unexecuted at this assessment. Add these checks to the existing
three milestones; do not begin another review loop or expand to unrelated
Batter adoption. The owning Bead records this proposal pending implementation.


## Authorized recovery execution (2026-09-13)

### Progress

The user approved the recovery and actual-consumer correction. The protected
pool/context entrypoint, pre-snapshot ledger lock, bounded catalog loads and
case-normalized parameter lookup are implemented. Required current privileges
and bounded discovery with per-kind defaults/exact PUBLIC overrides are
implemented. Combined, migration-only and authority-only entrypoints share one
private execution owner; distinct stages report distinct coverage.

Focused native ownership, both DDL orderings, mixed-case parameter, catalog
capacity, required SET/INHERIT/PUBLIC/column and discovery controls pass. An
isolated repeatable-read removal makes the concurrent ACL regression fail;
restoring it passes. The disposable actual consumer compiles on its own Rust
1.97.1 toolchain and passes both real serving profiles, six existing authority
regressions, startup/cleanup/drop controls, admin lifecycle and native ledger
corruption/search-path controls. A follow-on relation/column/routine change is
policy data only. Its original checkout and dependency pin remain unchanged.
Final broad validation, remaining guard mutations and independent review are
still pending; earlier matrix receipts are not reused for changed inputs.

### Surprises & Discoveries

The consumer's schema stage also runs under migration credentials. A combined
migration/authority checker would force a false permissive policy there, and
would fail reading a revoked ledger before its privilege stage could report
missing grants. Two small stage-specific entrypoints solve this using the same
owned executor. They do not invent a second lifetime protocol.

The old concurrent-ACL barrier blocked the new early ledger permission check
before snapshot creation. The fixture now gives the same restricted login
ownership of only its ledger/schema and observes the later exact membership
catalog read. The changed isolation-level mutation fails, proving the barrier
actually distinguishes the snapshot contract. The after-lock DDL test initially
expected UnsuccessfulMigration for an extra row; the existing comparator assigns
UnexpectedMigration to extras. The corrected oracle requires that exact version
2 finding and separately proves the installed RLS policy hides it afterward.

### Decision Log

Preserve separate native/application schema stages and safe error categories.
Use shared required/discovery policy data to remove duplicate supported ACL
queries. Unsupported object ownership remains a narrow local guard; exact native
ledger shape, privileged function search-path and queue verification remain local.
No consumer source or pin change is applied to the original checkout.

### Outcomes & Retrospective

Implementation and focused consumer evidence now address the ownership and API
causes rather than adding caller cleanup instructions. Completion is still
conditional on final checks and independent review; this record does not close
the Bead or authorize a commit/publication.

### Recurring-invariant assessment before dependent repair

The fresh independent native review found another ledger snapshot failure in an
ordinary inheritance layout: the early lock names ONLY the parent, while SELECT
reads its descendants. A native two-connection reproduction retained the parent
lock and repeatable-read snapshot, truncated the child concurrently, then read
zero ledger rows instead of the child's pre-snapshot unexpected row. This is a
confirmed recurrence of the complete-ledger observation invariant, so the
ADR-010 consumer/API assessment is initiated before another dependent repair.

The same review found unqualified catalog helper resolution under the serving
search path. A native reproduction with an application schema before pg_catalog
makes current_database() report template1 while pg_catalog.current_database()
correctly reports the serving database. The actual model consumer uses an
application-first search path, so this is a real integration concern.

Classification: both are implementation gaps within the approved library-owned
execution boundary, not application lifecycle obligations. The ledger lock must
cover the same relation set that the read includes. Catalog SQL must execute
under a trusted transaction-local resolution path, restored with rollback;
serving identity, privileges and later application session options remain intact.
Neither requires a new consumer protocol, an arbitrary callback, expanded native
responsibility, or a replacement authority engine. The application/native exact
ledger and function protocols remain where the approved assessment put them.

These repairs are within the user's approved design and original acceptance;
no additional API scope or permission is inferred. Add independent regression
controls for inherited-ledger truncation and a shadow helper selecting a database
with different ownership, and kill each omitted guard in the isolated copy.
The reviewed source stays frozen until both reports settle. The private whole-
executor boxing correction has already passed exact failing Clippy and minimum-
toolchain build in isolation; it will join this one coherent repair, followed by
final verification. Do not report the earlier narrower boxed-query build as
resolving all configurations.


### Settled recovery verification

Both complete workspace matrices passed on Rust 1.98.1 and 1.94.0 after the
whole-executor future correction and all accepted review repairs. Each exact
SQLx runner executed 46/46 cases (11 lease/migration, 14 owned-pool, 21 authority).
All ten HTTP profiles and normal/violation verifier modes passed. The latter
returned WithinDeclaredPolicy with zero findings and Violations with a nonzero
exit, respectively. Eight intentionally removed guards failed independently in
a source copy; each restored guard passed. The final repeatable-read control
was repeated against the settled SQLx source and rejected leaked concurrent ACLs.

The required Jig profile passed with api:test receipt
receipt_01M2D9F22BN104C2DVJD9QJTGP; evidence and gates inspection report all required
inputs fresh and no unresolved gates. Formatting, Clippy, contract and file-budget
siblings passed. Final tracker/documentation edits will refresh only affected
policy receipts while reusing unchanged Rust inputs and the same toolchain and
environment evidence.

The disposable actual consumer's two profiles and policy-only extension, six
existing authority regressions and strict all-target database Clippy passed again
after the final source repairs, on its own Rust 1.97.1. Earlier focused startup,
cleanup, admin lifecycle, native ledger and search-path controls remain recorded
in /tmp/batter-downstream-evidence.md. The original dependency pin is unchanged;
concurrent unrelated edits noticed in that original checkout were neither
modified nor reverted. No commit, publication or real consumer upgrade is authorized.

The final independent Codex pass has frozen complete working-tree fingerprint
30d44538e8153ba2f6a277da67a1b6ee4cbf3e411cca2c87adf18b2cf8dd7400.
Only trusted HEAD .agent exclusion applies. Closure still awaits that pass;
passing matrices alone are not treated as a design-review result.

### Required-parameter recurrence assessment before repair

The final independent Codex pass found one additional required-authority false
positive. A native regression in the isolated source copy loads PL/pgSQL with DO,
observes PostgreSQL reject SET on plpgsql.missing_parameter under its reserved
prefix, then observes the full verifier return WithinDeclaredPolicy for an
allow-and-require policy. Its expected nonpassing verdict fails on the reviewed
source. This confirms a recurrence in the coupled parameter visibility/required
phase after the visibility repair, triggering ADR-010 assessment before repair.

Classification: an implementation gap between two already-approved contracts.
Conservative excess-authority evaluation may overapproximate an unknown custom
placeholder's possible SET capability; required current-role privilege needs
positive observable evidence. The private custom flag was incorrectly reused as
that evidence. Neither another caller instruction nor an application-specific
extension registry removes this cause. Library inspection must mark a required
custom target with absent context ParameterUnobservable/Incomplete, retaining
known findings and avoiding a fabricated MissingPrivilege. Visible loaded
parameter context and ACL grants remain modeled; optional potential-authority
checks remain conservative. No new API, live SET probe, privileged auxiliary
connection, extension protocol or consumer lifetime obligation is introduced.
This is within the user's approved required/discovery scope and existing
Incomplete contract. Add native rejected-reserved-prefix, accepted-placeholder
but unknown-context, and visible-loaded-parameter controls, plus a synthetic
hidden-custom ACL case. Revalidate the affected source and obtain reviewer
confirmation before closure; the previous green matrix is historical for this
additional semantic change.


### Completion of the authorized recovery

The final parameter correction passed both full workspace matrices on Rust
1.98.1 and 1.94.0, all 47 native SQLx cases on each toolchain, ten HTTP smokes and
all four normal/violation example runs. Required Jig api:test receipt is
receipt_01M2DA5T9DFBQJC18HX2N3BEXW; all sibling gates passed with fresh evidence.
The independent reviewer confirms the last defect resolved and no coupled
finding at matched complete fingerprint
8012edd5421b835d3a8e74a9264a7c4da2e79e399dfeffe5f78f5da0256ca804.
This targeted confirmation follows the recorded complete full native pass.

The final disposable-consumer profiles, six authority regressions and strict
Clippy passed again. All original acceptance remains on this Bead and is met;
no unresolved requirement was moved to another task. The protected API removes
caller transaction/disposition coordination; required privileges and discovered
excess authority have distinct contracts. Exact application/native protocols
remain downstream. Eight isolated guard removals and the final reserved-prefix
false-pass reproduction establish discriminating failure evidence. Cleanup of
the owned disposable PostgreSQL cluster and temporary build outputs follows
recording of the final evidence; source, patch and logs remain reviewable.
