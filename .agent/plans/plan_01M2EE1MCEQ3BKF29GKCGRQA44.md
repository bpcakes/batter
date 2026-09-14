# Add protected SQLx verification safeguards

This ExecPlan is a living document. Keep `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` current while implementing it.
Maintain it under `.agent/PLANS.md`. The owning delivery record is Bead
`batter-g19`; Beads owns scope, acceptance, priority, status, and dependencies.

## Purpose / Big Picture

After this change, an application can ask Batter to verify three common
PostgreSQL setup invariants without copying catalog queries into its composition
root: the exact SQLx 0.9 migration-ledger shape and selected history, exact
`search_path` configuration on every scoped `SECURITY DEFINER` routine, and the
absence of current-database ownership reachable from the authenticated login.
Each request uses the caller's `OperationContext` and Batter's existing owned
lease, read-only repeatable-read transaction, snapshot ordering, rollback, and
failure disposition. The visible proof is a public example plus offline and
PostgreSQL 18 tests where conforming state passes and one changed invariant
produces a structured finding or incomplete result without applying DDL.

This plan implements only the safeguard work assigned to `batter-g19`. The pure
exact-role manifest/compiler/renderer is already delivered. A later Bead owns
real pinned-consumer adoption and provisioning equivalence.

## Progress

- [x] (2026-09-14) Settled and committed prerequisite `batter-7r3.5`; terminal
  low residuals are separately tracked as `batter-mzi` and `batter-lod`.
- [x] (2026-09-14) Claimed `batter-g19`, started Jig plan
  `plan_01M2EE1MCEQ3BKF29GKCGRQA44` at baseline
  `fbfa041eb1945085650f776c45532bca89ee5b72`, and inspected the current executor,
  migration policy, report, role graph, PostgreSQL 18 catalogs, and SQLx 0.9.0
  migration source.
- [x] (2026-09-14) Add validated public request types and pure history/shape evaluators with
  exhaustive failure-path unit tests.
- [x] (2026-09-14) Extend the one private executor with optional SQLx-ledger, schema-setting,
  and reachable-ownership inspections without changing legacy entrypoints.
- [x] (2026-09-14) Add deterministic PostgreSQL 18 cases, runner inventory, guard-removal
  evidence, and the runnable example update.
- [x] (2026-09-14) Update contracts, status, references, validation, crate guidance, Jig
  scopes if needed, and the owning Bead.
- [x] (2026-09-14) Run both supported toolchain matrices and all ten rebuilt HTTP smokes; the
  exact 61-case live suite on a disposable PostgreSQL 18.4 container, Jig gates,
  and the requested comprehensive all-reviewer review/fix loop. The first live
  execution exposed and the supporting closure repaired two test-only oracle
  defects; the complete rerun and both toolchain matrices pass.
- [x] (2026-09-14) Reached the requested terminal-low path, retained the final
  documentation chronology gap as `batter-y8e`, and prepared every task change
  for the user-authorized final commit and clean-checkout check.

## Surprises & Discoveries

- Observation: `batter_sqlx::verification::execute` already owns exactly one
  lease and one reset/snapshot/rollback sequence, while `Inspection` selects the
  work inside it.
  Evidence: `crates/batter-sqlx/src/verification.rs` functions `execute` and
  `inspect_checkout`; new wrappers can extend `Inspection` instead of adding a
  second executor.
- Observation: the current low-level ledger checker intentionally validates only
  `version`, `checksum`, and `success`, and its public structs are exhaustively
  constructible.
  Evidence: `crates/batter-sqlx/src/verification/migration.rs` and
  `policy.rs`; the high-level SQLx shape must be additive and must not silently
  tighten `MigrationPolicy`.
- Observation: SQLx 0.9.0 creates six non-dropped user columns and a primary key
  on `version`; its `Migration` exposes version, checksum, and `MigrationType`,
  but the adapter does not enable SQLx's `migrate` feature.
  Evidence: local registry sources `sqlx-postgres-0.9.0/src/migrate.rs` and
  `sqlx-core-0.9.0/src/migrate/{migration,migration_type}.rs` at the locked graph.
  The public manifest will accept Batter-owned expectations and therefore will
  not broaden crate features or retain SQL bodies.
- Observation: PostgreSQL 18 represents routine-local settings as `pg_proc.proconfig`
  (`text[]`) and database-local role ownership as `pg_shdepend` rows with
  `deptype = 'o'`; `objsubid` identifies subobjects.
  Evidence: PostgreSQL 18 `pg_proc` and `pg_shdepend` primary documentation.
- Observation: the first comprehensive review found that an empty result set
  could not distinguish an existing schema with no definers from a misspelled
  schema, and that protected fragments were appended after the authority
  evaluator's last capacity checkpoint.
  Evidence: matching Claude, Codex and Cursor reports on working-tree fingerprint
  `d9cc7cb307289ede4e71c0d2a937b55d91efee7fe0954a2cc0a050ab7db998ad`.
- Observation: the third comprehensive pass found that the first ledger-race
  repair proved name-to-lock identity only once, then discarded the OID and let
  both history readers resolve the name again after the snapshot.
  Evidence: matching all-reviewer reports on fingerprint
  `f560a8f7187be73bf2d4f356686380f8833ec668618e4fe7aa85ee46aaf1f539`;
  Cursor rated the resulting unlocked replacement scan high severity. This is
  the ADR-010 recurring-invariant trigger and is recorded on `batter-g19`.
- Observation: the fourth comprehensive pass found that the cursor still used
  the policy name in an SQL string literal and independently resolved the name
  for identity and row-source planning. With `standard_conforming_strings=off`,
  a backslash-plus-apostrophe identifier could change parsing; stale regclass
  resolution could also attest the old OID while an empty replacement supplied
  the row source.
  Evidence: scope-stable all-reviewer pass
  `f978ec53bc5243ba86eed354b0cab7b7fee3aa1a9501895c3c536f0a8cbc0a3a`;
  Codex rated the literal defect high and Claude independently found both root
  causes. The repeated design concern and replacement boundary are recorded in
  Bead comment 114.
- Observation: the next all-reviewer pass reported a high ownership path based
  on stored membership in `pg_database_owner`, plus a stale live-test barrier
  and legacy type-namespace mismatch. PostgreSQL 18 primary documentation says
  the proposed membership cannot be granted in either direction, but the model
  still accepted such an impossible catalog row as ordinary evidence.
  Evidence: scope-stable fingerprint
  `29637f231630fcfa89108f4a160be80076f2f07f9638ebff4cb22e74af07deac`,
  PostgreSQL 18 predefined-role documentation, and Bead comment 115.
- Observation: the following pass found a real fail-open in the same catalog
  classifier: malformed known non-owner rows and the impossible tablespace-to-
  role form were skipped before structural validation. It also found low
  cursor-name, test-pause, snapshot-wording and example-input defects.
  Evidence: scope-stable fingerprint
  `adac311ba3f3eba366f2cdc9f11049aa32027b48c19ffd3de35cb495c58ae106`;
  PostgreSQL 18 `pg_shdepend`, `DECLARE`, `FETCH`, and `pg_cursors` contracts.
- Observation: the terminal pass on the next repaired tree found two coupled
  ownership-selection defects: missing role identities could look unreachable,
  while well-formed unreachable owners consumed the relevant-row limit before
  graph selection. It also found that cursor `42501` conflated protected-heap
  SELECT denial with replacement denial and that schema-only coverage omitted
  the unsupported body surface.
  Evidence: matching scope fingerprint
  `ac7d9a23a92cd5064ba0dda7d4d39a4c90a1990c56b6c9c2dfa98a06d7edf76a`;
  Claude and Codex medium findings, Cursor no findings, and restored mutation
  failures for both ownership guards.
- Observation: the fresh terminal pass on fingerprint
  `ff22d9e94e1a01e4bf1e481c6c9e8f1a7982e04ad00d25e4cd802c0819aaa016`
  left no high- or medium-severity implementation defect. A subsequently
  provisioned PostgreSQL 18.4 live run exposed a multi-command prepared fixture
  call and a barrier assertion that still named the metadata query preceding the
  retained-OID redesign.
  Evidence: the initial exact runner passed 59 cases and failed those two;
  PostgreSQL statement logging located the barrier in the post-lock attestation.
  Both focused cases and the complete 61-case rerun passed after test-only fixes.
- Observation: the second focused closure pass produced one valid low
  documentation-chronology gap and one invalid high control-flow report. The
  high report assumed that an INSERT-only role cannot take `ACCESS SHARE`, but
  PostgreSQL 18 permits INSERT holders to take `ROW EXCLUSIVE` or any
  less-conflicting mode. The exact server accepted `ACCESS SHARE` with INSERT
  true and SELECT false, and the strengthened focused case plus complete
  61-case runner passed on the unchanged tree.
  Evidence: PostgreSQL 18 `LOCK` privilege documentation, the disposable 18.4
  transaction probe, and the final exact live-runner execution. The valid low
  is retained as `batter-y8e`; no production or test-logic repair followed.

## Decision Log

- Decision: add private owned request components and borrowed public
  `VerificationRequest` composition, while preserving the exact layouts and
  signatures of legacy low-level policies and entrypoints.
  Rationale: safeguards are opt-in and semantically separate from serving ACL
  ceilings; one internal `Inspection` value can still drive one executor.
  Date/Author: 2026-09-14 / Codex.
- Decision: `SqlxMigrationManifest` accepts a qualified ledger, `SqlxLedgerMode`,
  and Batter `MigrationExpectation` values rather than native SQLx migrations.
  Rationale: this retains only version/checksum metadata, avoids feature
  unification, and lets callers consistently filter Simple/ReversibleUp inputs
  at their application boundary.
  Date/Author: 2026-09-14 / Codex.
- Decision: installed-subset comparison is set membership, not prefix ordering.
  Rationale: every installed row must be successful and match an expected
  checksum, but later or earlier expected versions may remain absent.
  Date/Author: 2026-09-14 / Codex.
- Decision: schema policy selects explicit schemas and one exact stored string,
  initially `search_path=pg_catalog, pg_temp`; it does not parse equivalent GUC
  syntax.
  Rationale: exact stored configuration is the consumer invariant and avoids a
  misleading general PostgreSQL settings normalizer.
  Date/Author: 2026-09-14 / Codex.
- Decision: ownership findings retain class catalog, object OID, and `objsubid`
  as bounded identifiers. Unknown but interpretable classes are violations;
  malformed, unknown-form, or over-capacity rows are incomplete/error.
  Rationale: ownership denial does not require a second ACL evaluator, but it
  must not silently skip catalog classes it cannot name semantically.
  Date/Author: 2026-09-14 / Codex.
- Decision: one `Evaluation` instance is borrowed by authority evaluation and
  then checkpoints cumulative findings plus bounded work from every protected
  fragment. Constructors collect only through the first overflow sentinel.
  Rationale: composing independently bounded inputs must not bypass the existing
  global work/report contract or consume an unbounded iterator before validation.
  Date/Author: 2026-09-14 / Codex.
- Decision: selected schemas must exist, and ownership-specific verification is
  incomplete when a capability-reachable superuser or predefined role makes
  `pg_shdepend` ownership records non-exhaustive.
  Rationale: both are missing-evidence states; treating either as an empty clean
  inventory would be a false pass.
  Date/Author: 2026-09-14 / Codex.
- Decision: redesign the protected-ledger boundary around a retained relation
  identity rather than another timing check. Lock acquisition must return the
  OID it protected; later metadata and shape work use that OID, while one
  self-attesting cursor declaration early-binds both the caller-supplied name's
  OID and row source and rejects either identity if it differs from the lock.
  Rationale: the adapter owns this protocol and the public request already frees
  consumers from coordinating it. The former `Ready` flag permitted every later
  caller to forget which relation was locked, so a local recheck would only move
  the same race window.
  Date/Author: 2026-09-14 / Codex.
- Decision: a planned cursor must attest the relation locks its declaration
  acquired rather than ask another independently parsed name expression to
  identify the row source. The declaration uses the retained numeric OID as its
  identity constant, quotes the policy name only as a RangeVar, and rejects new
  non-catalog relation locks outside the protected heap/index/TOAST family.
  Rationale: this removes policy text from SQL literals under every string GUC,
  detects an empty replacement that cannot return `tableoid`, and retains the
  transaction-local portal that binds FETCH to the declaration's plan.
  Date/Author: 2026-09-14 / Codex.
- Decision: represent the implicit current-database-owner to
  `pg_database_owner` edge separately from captured `pg_auth_members` rows and
  fail ownership inspection incomplete if any stored row touches that role.
  Rationale: PostgreSQL 18 forbids those stored grants, so they are
  uninterpretable evidence rather than another supported reachability path;
  this preserves the real current-owner finding without allowing malformed
  catalog state to use the implicit-edge exception.
  Date/Author: 2026-09-14 / Codex.
- Decision: validate every retained shared-dependency address before dependency
  classification, exclude only structurally valid ACL/initial-ACL/policy rows
  before the bound, and make impossible tablespace-to-role evidence incomplete.
  Rationale: known irrelevant rows should not exhaust capacity, but their type
  label must not conceal malformed catalog evidence.
  Date/Author: 2026-09-14 / Codex.
- Decision: generate a bounded cursor name, prove it absent from the current
  session's `pg_cursors` inventory, and retain its numeric suffix with the
  protected relation OID through FETCH.
  Rationale: a caller-owned holdable cursor can survive the executor's initial
  rollback; a fixed internal name therefore was not an owned namespace.
  Date/Author: 2026-09-14 / Codex.
- Decision: select only capability-reachable, malformed, unknown, or
  missing-role shared dependencies before the ownership row bound, then validate
  all database, membership, predefined-role, and retained-owner identities
  against the same role snapshot.
  Rationale: capacity must count evidence relevant to the requested conclusion,
  but catalog inconsistency must remain visible and incomplete rather than look
  like an unreachable clean owner.
  Date/Author: 2026-09-14 / Codex.
- Decision: keep the live repair supporting-only by executing GRANT and REVOKE
  separately and changing the descendant oracle to identify the first
  post-lock `pg_class`/`pg_locks` attestation query.
  Rationale: SQLx deliberately rejects multiple prepared commands, and the
  retained-OID redesign moved the protected phase without weakening the
  descendant-lock invariant. Neither correction changes verifier behavior.
  Date/Author: 2026-09-14 / Codex.

## Outcomes & Retrospective

The target outcome is implemented as one additive protected API over the
existing executor, with legacy behavior preserved by the same retained-OID
path. Both toolchain matrices, all ten HTTP smokes, strict package Clippy, the
123-test package matrix, all restored guard tests, and the exact 61-case
PostgreSQL 18.4 live runner pass. Jig run
`run_01M2EZFCVRGPWVGBWQQT9V4KJ5` passed all five targets on the supporting
closure inputs. Terminal low follow-ups are `batter-k2d`, `batter-1rt`,
`batter-g9l`, `batter-m8x`, `batter-86p`, and the final focused-closure
documentation record `batter-y8e`. The final focused pass did not establish
formal closure convergence because its one correction allowance was already
consumed; the user-selected low-only path records that residual and proceeds to
commit. The final handoff records the commit identifier and clean-checkout
result.

## Context and Orientation

The crate is `crates/batter-sqlx`. `src/verification.rs` exports public types and
functions and privately owns `Inspection`, `execute`, and `inspect_checkout`.
`src/verification/migration.rs` locks and reads a generic three-column ledger.
`src/verification/authority.rs` and its submodules load identities, memberships,
objects, ACLs, and evaluate one captured snapshot. `src/verification/report.rs`
defines structured findings and supported/unsupported surfaces. `src/verification/
policy.rs` owns validated names and legacy policies. Live cases are explicitly
registered through `tests/verification_live.rs`; `scripts/sqlx_live.py` rejects
inventory drift because Cargo has `autotests = false`.

A protected request means an async public wrapper that accepts `&PgPool` and the
caller's `&OperationContext`, performs validation inside that context, and
delegates to the existing owned lease/transaction path. A captured snapshot is
the PostgreSQL repeatable-read view established only after protective relation
locks. Reachable ownership means ownership held by the authenticated login or a
role reachable through the existing INHERIT, SET, ADMIN-derived, or mixed role
graph; it is not rooted at a masked `current_user`.

The repository is Unix-only and supports macOS and Linux. Default Rust is 1.98.1
and the minimum is 1.94.0. PostgreSQL provisioning stays external. Tests may
create fixtures only through the existing credentialed disposable live harness.

## Plan of Work

First add a `request` module under `src/verification/` containing
`SqlxLedgerMode`, `SqlxMigrationManifest`, `SchemaInspectionPolicy`, and
`VerificationRequest`. Constructors own and validate names, expected checksums,
schema lists, and exact setting text. Duplicate versions, empty or oversized
inputs, and temporary schema requests fail with existing or narrowly added
policy errors. Add `verify_sqlx_migrations`, `verify_exact_role`, and
`verify_request` wrappers in `verification.rs`; validation remains inside
`context.run`. `CompiledExactRole` gains only private safeguard data plus a
focused builder/method for blanket current-database ownership denial.

Next generalize `Inspection` into borrowed optional components. Before fixing the
snapshot, lock each requested ledger. For `InstalledSubset`, an absent lock is
accepted only if the fixed snapshot also proves the relation absent; appearance
without a protective lock returns incomplete rather than reading it. Present
ledgers retain `ACCESS SHARE`, inheritance rejection, `ONLY` row reads, and RLS
rejection. Add a new SQLx-specific inspector that validates exactly the six
canonical non-dropped, non-null columns by `pg_type` namespace/name identity and
an exact primary-key column list of only `version`, then evaluates exact or
installed-subset rows with the existing checksum/row bounds.

Then capture schema settings and ownership within the same transaction and
shared evaluation budget. For every `pg_proc.prosecdef` routine in an explicitly
selected schema, inspect all stored `proconfig` entries without filtering by
EXECUTE privileges or routine kind. Require exactly one entry equal to the
configured search-path string; accept unrelated settings. Bound routine count,
entry count, and retained bytes before allocation where possible. Add a distinct
finding and supported surface for this exact configuration check while retaining
the unsupported body/trigger-semantics boundary.

For ownership, reuse the already captured `RoleGraph`. Load the current database
owner plus current-database `pg_shdepend` rows whose referenced catalog is
`pg_authid` and dependency kind is owner. Count all returned rows before
filtering. Compare each owner role OID against capability reachability rooted at
the authenticated login. Report the database itself and each reachable owned
address once. Ignore known non-owner dependency kinds only after proving their
form; preserve class OID, object OID, and `objsubid` for coarse findings.
Unknown/interpretable ownership classes remain violations, while malformed or
capacity-exhausted evidence fails incomplete/error. Do not inspect other
databases or tablespace ownership.

Finally add pure tests beside each evaluator and deterministic live modules for
SQLx ledger shape/history, schema settings, ownership, cancellation, and
concurrency. Register every live case and update the runner's exact inventory.
Run one temporary guard-removal mutation for the ledger-shape predicate, exact
search-path comparison, and ownership-row evaluation; each named test must fail,
and exact source bytes must be restored. Update README, crate guide,
`docs/guarantees.md`, `docs/integrations.md`, `docs/status.md`,
`docs/testing.md`, `docs/references.md`, and `docs/validation.md` without claiming
provisioning, routine-body safety, future-state safety, or unexecuted live proof.

## Concrete Steps

Work from `/Users/aa/Documents/batter`.

1. Add request types and pure validation/evaluation tests. Run:

       cargo test -p batter-sqlx --features test-support --locked verification::

   Expect all focused tests to pass and no live tests to run.

2. Extend the private executor and catalog capture incrementally, running the
   package suite after each safeguard family:

       cargo test -p batter-sqlx --features test-support --locked
       cargo clippy -p batter-sqlx --all-targets --all-features --locked -- -D warnings

3. With all three explicit live URLs for a dedicated disposable PostgreSQL 18
   environment, run:

       bash scripts/test_sqlx_live.sh

   The runner must first discover exactly its checked-in inventory, then execute
   every case serially. If credentials are absent, record the prerequisite as
   unexecuted; do not use unrelated containers or weaken the runner.

4. Run each guard-removal mutation against its named test, restore the exact
   implementation, and repeat the unmodified test. Record failing and passing
   commands in `docs/validation.md`.

5. Run complete local validation on both toolchains:

       bash scripts/verify.sh
       RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh

   Rebuild `batter-axum`'s `http_service` with each toolchain and run the five
   documented `scripts/smoke_http.py` modes: default, SIGINT, deadline,
   warn-filter, and warn-filter plus deadline.

6. Run the Jig work profile for this plan, inspect evidence/gates, and preserve
   a fresh current-source `api:test` receipt:

       scripts/jig work check --plan-id plan_01M2EE1MCEQ3BKF29GKCGRQA44
       scripts/jig work evidence --plan-id plan_01M2EE1MCEQ3BKF29GKCGRQA44
       scripts/jig work gates --plan-id plan_01M2EE1MCEQ3BKF29GKCGRQA44

7. Run the requested comprehensive all-reviewer review/fix loop with low as the
   minimum severity. Repair medium findings and repeat; redesign from the root
   cause for high findings. At terminal low-only findings, create residual Beads
   before commit. Re-run affected validation after every repair.

8. Update this plan and `batter-g19`, close the Bead only when acceptance is
   honestly met, commit every tracked state/evidence change, and require an empty
   `git status --short`.

## Validation and Acceptance

The additive low-level compatibility contract is proven when existing
`verify`, `verify_migrations`, and `verify_authority` signatures and exhaustive
policy struct construction still compile. An already-cancelled wrapper performs
no acquisition. Cancellation while the one-slot pool is occupied and while each
new inspection is blocked returns the existing interruption type and leaves the
lease on the conservative retirement path. Rollback failure retains the primary
error and never reports success.

SQLx exact mode passes only for a present standard ledger containing exactly the
expected successful version/checksum set. Installed-subset mode passes for an
absent ledger or any non-prefix subset of expected successful rows; it rejects an
unknown, failed, mismatched, duplicate, inherited, RLS-enabled, malformed,
oversized, or unprotected late-appearing ledger. Shape comparison ignores column
order/default expressions but requires the exact six names, pg_catalog type
identities, NOT NULL flags, and primary key list.

Schema inspection passes only when every scoped definer routine has exactly one
stored `search_path=pg_catalog, pg_temp` entry. It includes trigger and ungranted
routines, accepts unrelated GUCs, rejects missing/duplicate/alternate strings,
and fails closed at count/byte limits. Ownership inspection passes only when no
capability-reachable role owns the current database or a current-database object;
direct, inherited, SET, ADMIN-derived, and mixed paths are covered. Non-owner
dependency kinds do not create findings; unfamiliar but structurally valid owner
addresses do.

The final public example must exercise a protected high-level wrapper in normal
and deliberate-violation modes on PostgreSQL 18 when credentials are available.
Documentation must state the snapshot, absence-race, role reachability, routine
configuration, capacity, and unsupported-body semantics precisely.

## Idempotence and Recovery

All production verification is read-only except PostgreSQL relation locks and
transaction-local settings, which rollback releases. Fixture creation and
cleanup remain owned by the existing live harness. If a live run fails, preserve
its bounded output, rerun cleanup through the harness, and retry only against the
same authorized disposable environment. Do not manipulate unrelated local
database containers. Guard mutations are local temporary edits: capture the
original diff, prove the named test fails, restore with `apply_patch`, and verify
the final diff rather than using destructive Git reset commands.

The public APIs are additive. Existing low-level callers need no migration. If a
new enum variant breaks an exhaustive internal match, update every checked-in
consumer and record that limited source-compatibility consequence; do not hide a
distinct safeguard failure under an unrelated old variant.

## Artifacts and Notes

The reviewed parent design is retained at
`.agent/plans/batter-7r3.6-compact-verification-adoption.md`, Milestone 3. Primary
external semantics belong in `docs/references.md`. Executed outputs and explicit
missing prerequisites belong in `docs/validation.md`. Tracker comments summarize
delivery evidence but do not replace either artifact.

Plan revision note (2026-09-14): expanded the Jig-created task stub into the
self-contained `batter-g19` implementation plan so safeguard work can proceed
independently of the later consumer-adoption Bead.

## Interfaces and Dependencies

`crates/batter-sqlx/src/verification/request.rs` should expose, through
`verification.rs`, private-field owned types with focused constructors:

    pub enum SqlxLedgerMode { Exact, InstalledSubset }

    pub struct SqlxMigrationManifest { /* ledger, mode, expected */ }

    pub struct SchemaInspectionPolicy { /* schemas, exact search_path */ }

    pub struct VerificationRequest<'a> { /* optional borrowed components */ }

`verification.rs` should expose:

    pub async fn verify_sqlx_migrations(
        pool: &PgPool,
        context: &OperationContext,
        manifest: &SqlxMigrationManifest,
    ) -> Result<VerificationReport, OperationError<VerificationError>>;

    pub async fn verify_exact_role(
        pool: &PgPool,
        context: &OperationContext,
        role: &CompiledExactRole,
    ) -> Result<VerificationReport, OperationError<VerificationError>>;

    pub async fn verify_request(
        pool: &PgPool,
        context: &OperationContext,
        request: VerificationRequest<'_>,
    ) -> Result<VerificationReport, OperationError<VerificationError>>;

Use SQLx 0.9.0 and PostgreSQL 18 already pinned by the workspace. Do not add an
ORM, migration executor, DI container, setup connection, second verifier, or
foundation dependency. Application code continues to own migration selection,
grant contents, role creation, DDL, provisioning credentials, routine bodies,
and deployment.
