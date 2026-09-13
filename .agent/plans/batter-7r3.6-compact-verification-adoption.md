# Remove downstream PostgreSQL verification duplication through an exact-role manifest

This ExecPlan is a living document. The sections `Progress`, `Surprises &
Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept current
as implementation proceeds. Maintain this file according to `.agent/PLANS.md`.
The owning delivery record is Bead `batter-7r3.6` in the Batter repository.

## Purpose / Big Picture

An application adopting Batter's PostgreSQL verifier should describe each
database role once and then use that same description both to provision the role
and to verify it at startup. Today Batter exposes only the low-level
`AuthorityPolicy` graph. In the authorized private reference adoption, removing
local SQL checker logic required a larger Rust translator, a large role grant
list, and another copy of grants already present in provisioning scripts. The
generic catalog evaluator moved upstream, but downstream hand-authored code grew
and policy remained duplicated. Exact consumer paths, names, revisions, and
measurements live only in the consumer-side adoption record.

After this work, Batter will expose a compact, validated exact-role manifest. An
"exact-role manifest" is application-owned data describing the privileges a
serving role must have and the maximum privileges that the authenticated login,
its inherited roles, and its switchable roles may possess. Batter will compile
that data into the existing low-level `AuthorityPolicy` and into a deterministic
plan of PostgreSQL `GRANT` statements. It will not connect with owner credentials,
execute DDL, create roles, repair grants, or own application policy.

The same real downstream application will be the pinned reference consumer. Its
four distinct serving and queue profiles will each have one grouped manifest. A local
operator command will render the complete grant transaction to stdout; an
operator may explicitly pipe that output to `psql`. Startup will compile the same
manifest and invoke Batter's verifier. The four hand-maintained provisioning
scripts and the consumer's generic translation layer will be removed. Existing missing
and excess privilege behavior, migration checks, search-path checks, ownership
checks, error categories, and restricted-login tests must remain observable.

The practical proof is twofold. First, each reference profile must produce a
grant transaction that provisions a fresh restricted login which then passes
startup verification, while removing or adding one privilege still fails with
the existing category. Second, the consumer's inclusive hand-authored
verification responsibility must
be smaller than its pre-adoption baseline instead of merely moving lines between
files.

## Progress

- [x] (2026-09-13) Inspected Batter commit
  `23a50fa74fd043579260135ecac2af35840e02de`, its public verification API,
  package guidance, open and closed Beads, and the immutable reference-consumer
  adoption. Exact private revision identifiers are retained downstream.
- [x] (2026-09-13) Reproduced the private reference consumer's historical and
  current affected-source measurements. The detailed path inventory, totals, and
  net Rust change are retained downstream; the upstream acceptance uses only the
  generic conclusion that current hand-authored code exceeds the baseline.
- [x] (2026-09-13) Integrated planning review round 1. It found structural gaps
  in schema-versus-authority execution, PUBLIC-deny precedence, provisioning
  intent, database and routine rendering context, absent-ledger concurrency,
  SQLx feature isolation, safeguard bounds, prototype sequencing, inclusive LOC
  accounting, live-test registration, and render-pipeline failure semantics.
- [x] (2026-09-13) Completed four fresh planning reviews. Rounds 1 and 2 made
  structural corrections, round 3 moved one misplaced integration obligation,
  and round 4 found no further implementation-blocking issue at fingerprint
  `e0345411366518edceb8ececf90a92738984abc59250e4795010a98ed063e682`.
- [x] (2026-09-13) Converted the reviewed plan into implementation Beads:
  `batter-9jh` owns the independently ready pure manifest/compiler/renderer,
  `batter-g19` owns protected safeguards after `.5`, and `batter-r2a` owns the
  pinned adoption after both. The terminal `.6` depends on `batter-r2a`.
- [x] (2026-09-13) Updated `batter-7r3.6` through supported `br --no-db`
  operations with this final generic plan, executable acceptance criteria, the
  three-node delivery DAG, and four-round planning evidence while preserving
  prior comments and the independently owned verifier blocker.
- [ ] Complete the independently owned post-repair review of the committed
  `batter-7r3.5` verifier baseline and settle its `.12` blocker before extending
  verifier execution or completing adoption; pure manifest work may proceed.
- [x] (2026-09-13) Added the pure high-level exact-role manifest compiler and
  deterministic grant-plan API for `batter-9jh` without changing existing
  low-level verifier entrypoints. Grouping, conflicts, PUBLIC precedence,
  capacity, structural routine rendering and the generic example have focused
  offline coverage. Both supported toolchain matrices, all ten rebuilt HTTP
  smokes and the comprehensive all-reviewer loop passed; low residual coverage
  is recorded in `batter-h2r`, while live grant application remains with the
  later adoption milestone.
- [ ] Add generic SQLx-ledger shape, scoped SECURITY DEFINER search-path, and
  coarse current-database ownership safeguards needed to remove the remaining
  downstream catalog queries.
- [ ] Prove the manifest compiler, renderer, safeguards, and low-level policy
  equivalence with offline and real PostgreSQL 18 tests plus a generic runnable
  example.
- [ ] Obtain an immutable Batter commit under the repository's explicit commit
  and publication policy before claiming pinned downstream adoption.
- [ ] Update the authorized reference consumer to that exact pin, make one grouped manifest the
  source for provisioning and verification, and remove superseded local code and
  scripts.
- [ ] Run the reference consumer's complete PostgreSQL, Rust, SQLx, frontend, and Jig
  gates, plus a comprehensive low-severity review-fix loop over the complete
  consumer adoption.
- [ ] Re-measure the inclusive consumer responsibility, record semantic and physical LOC, update
  both repositories' adoption evidence, and close `batter-7r3.6` only if every
  acceptance item is satisfied.

## Surprises & Discoveries

- Observation: Batter's current highest-level execution functions are
  `verify`, `verify_migrations`, and `verify_authority`; none constructs a compact
  application policy.
  Evidence: `crates/batter-sqlx/src/verification.rs` and
  `crates/batter-sqlx/src/verification/policy.rs` at `23a50fa` expose the
  entrypoints and public policy structs but no builder, profile, manifest, or
  grant-plan abstraction.

- Observation: Batter's documentation intentionally says profile-specific grant
  manifests remain downstream, but it also tells consumers not to duplicate the
  generic ACL evaluator. The reference consumer had to implement a generic manifest-to-policy
  evaluator anyway.
  Evidence: `crates/batter-sqlx/AGENTS.md` and
  `crates/batter-sqlx/README.md`; exact downstream source evidence is retained in
  the private adoption record.

- Observation: the reference consumer has two manually maintained
  representations of the same role grants: Rust policy declarations and four
  operational provisioning scripts.
  Evidence: the private adoption record retains exact source paths, atom counts,
  routine counts, and the immutable consumer revision.

- Observation: application policy cannot be inferred safely from SQL queries,
  SQLx metadata, or the currently connected role. Query inference misses dynamic
  SQL, function-mediated writes, row-lock privileges, durable-operation internals, and
  excess authority. Treating observed authority as desired authority would make
  verification tautological.
  Evidence: the reference profiles include lock-only column updates, exact overloaded
  routines, queue lifecycle tables, and deliberate denial of unrelated authority.

- Observation: `batter-7r3.6` already states that successful adoption must
  actually reduce downstream code. The 2026-09-13 adoption comment accurately
  lists removed SQL checkers but does not record that hand-authored consumer code
  grew. This plan corrects the acceptance interpretation; it does not erase the
  historical comment.

- Observation: the Batter SQLite tracker cache and `.beads/issues.jsonl` have a
  known semantic import conflict. Normal `br show` failed on `batter-538`; exact
  JSONL reads through `br --no-db` remain available. Preserve the existing
  uncommitted `batter-7r3.6` comment in `.beads/issues.jsonl` and use a
  hash-checked JSONL-safe update path rather than flushing the stale database.

- Observation: schema safety and serving-role authority are separate public
  contracts in the reference adopter. Schema verification also runs through
  owner/setup credentials, and its search-path failures retain a schema-error
  category. A safeguard stored only in a serving-role policy cannot preserve
  that boundary.
  Evidence: the adopter's public schema-verification method, setup call site,
  and direct search-path regression exercise this behavior independently from
  role verification.

- Observation: the existing public `AuthorityPolicy` is exhaustively
  constructible. Adding mandatory safeguard fields would be a source-breaking
  change even though the new high-level API is intended to be additive.
  Evidence: public policy structs and complete struct literals in Batter's own
  tests and example.

- Observation: allowed authority and provisioning intent are not equivalent.
  The reference profiles tolerate database CONNECT/TEMPORARY through ambient
  PUBLIC defaults, while their scripts grant schema, relation, column, and
  routine authority directly. A renderer generated from the allowed ceiling
  would grant too much.

- Observation: the migration module is feature-gated by SQLx's `migrate`
  feature, while `batter-sqlx` currently enables only `runtime-tokio` and
  `postgres`. Workspace feature unification can conceal that dependency during
  development.

- Observation: Batter disables automatic test discovery and its live runner
  checks an exact ignored-test inventory. New live cases require explicit module
  or target registration and runner inventory updates before a green command is
  meaningful.

## Decision Log

- Decision: keep `batter-7r3.6` as the terminal owner of the upstream API and the
  pinned reference adoption instead of creating a local consumer workaround.
  Rationale: the downstream translator implements generic conversion rules over
  Batter types. Leaving it downstream would violate the adoption task's purpose
  and make every future consumer repeat the same work.
  Date/Author: 2026-09-13 / Codex.

- Decision: represent implementation as a three-node delivery DAG rather than
  treating planning or review rounds as work items.
  Rationale: `batter-9jh` can implement pure manifest compilation immediately;
  `batter-g19` depends on the independently reviewed `.5` verifier; and
  `batter-r2a` depends on both before immutable reference adoption. The terminal
  `.6` depends on `batter-r2a`. This exposes useful ready work without weakening
  the retained `.6 -> .5 -> .12` prerequisite or creating a cycle.
  Date/Author: 2026-09-13 / Codex, after planning review round 4.

- Decision: introduce a high-level manifest alongside, not instead of, the
  existing `AuthorityPolicy`.
  Rationale: the low-level graph is necessary for unusual PostgreSQL policies
  and is already exercised extensively. The new layer represents the common
  exact-serving-role case and compiles into that graph, preserving one evaluator
  and avoiding a second authority engine.
  Date/Author: 2026-09-13 / Codex.

- Decision: make the manifest application-owned Rust data with explicit grouped
  constructors rather than TOML, JSON, a proc macro, or parsed SQL.
  Rationale: Rust data uses existing types, adds no parser or serialization
  dependency, receives compiler assistance, and remains inspectable by an agent.
  An opaque DSL would conflict with Batter's agent-only consumption policy.
  Parsing general PostgreSQL and `psql` syntax would add a second difficult
  grammar and still obscure semantic distinctions such as required authority,
  PUBLIC delivery, grant options, and ownership.
  Date/Author: 2026-09-13 / Codex.

- Decision: compile one manifest into both `AuthorityPolicy` and a normalized
  `GrantPlan`.
  Rationale: startup needs policy while provisioning needs statements. Producing
  both from one validated object removes the duplicated grant matrix without
  making Batter execute provisioning. The plan is data; execution remains an
  explicit operator action owned by the consumer.
  Date/Author: 2026-09-13 / Codex.

- Decision: distinguish required-and-provisioned declarations from allowed-only
  ceilings, and make PUBLIC delivery an explicit per-target override.
  Rationale: a verifier describes both the minimum and maximum acceptable
  authority, while a grant plan may contain only authority deliberately granted
  to the target role. Discovery defaults, ownership tolerance, grant-option
  tolerance, and ambient PUBLIC authority are verification rules rather than
  provisioning instructions. Exact PUBLIC denial must override permissive
  discovery defaults rather than rely on omission.
  Date/Author: 2026-09-13 / Codex, after planning review round 1.

- Decision: preserve separate schema-inspection and serving-authority requests
  over the existing owned executor.
  Rationale: schema verification must work under owner/setup credentials and
  retain schema error classification. New wrapper/request types can carry
  safeguards without changing the layout of public low-level policy structs or
  introducing a second evaluator.
  Date/Author: 2026-09-13 / Codex, after planning review round 1.

- Decision: provide a deterministic pure PostgreSQL renderer in `batter-sqlx`,
  but no provisioning connection or command.
  Rationale: identifier quoting, privilege grouping, and stable rendering are
  generic consequences of the manifest. A pure renderer cannot mutate a
  database. The consumer remains responsible for selecting the role, adding its
  transaction wrapper and global PUBLIC-schema revoke, and deciding whether to
  pass the resulting text to `psql`.
  Date/Author: 2026-09-13 / Codex.

- Decision: replace the consumer's four static provisioning scripts with an explicit
  render-only operator command, not an automatic grant applier.
  Rationale: a render-only command provides one source of truth while preserving
  owner review and explicit execution. It builds the complete string before
  writing stdout, emits `BEGIN` and `COMMIT`, returns nonzero before emitting SQL
  on invalid input, and never reads database credentials or connects. If a pipe
  breaks before `COMMIT`, `psql` disconnects with the transaction uncommitted.
  Date/Author: 2026-09-13 / Codex.

- Decision: move generic SQLx ledger-shape checking, exact scoped SECURITY
  DEFINER search-path checking, and coarse database-local ownership denial into
  Batter as opt-in policy.
  Rationale: these checks contain no application object names or business rules.
  They are the remaining generic catalog mechanisms in the reference consumer.
  Batter must own their
  captured-snapshot, capacity, cancellation, and role-reachability behavior if
  adoption is to delete rather than rename the local checker. Existing low-level
  defaults remain unchanged, so callers opt in deliberately.
  Date/Author: 2026-09-13 / Codex.

- Decision: prototype every real role profile through a temporary local path
  before freezing the upstream public API.
  Rationale: the terminal adoption task is not satisfied by an elegant upstream
  type alone. A pre-freeze consumer proof is the earliest reliable check that
  constructors are usable, policy equivalence holds, rendering is complete, and
  the local translator actually disappears. The prototype is evidence, not a
  pinned or completed adoption.
  Date/Author: 2026-09-13 / Codex, after planning review round 1.

- Decision: preserve application error classification downstream.
  Rationale: `MissingRuntimePrivileges`, `UnsafeRuntimeRole`, and operator-facing
  remediation messages are application-domain contracts. Batter should return
  typed reports and native failures, not know the consumer's startup taxonomy.
  Date/Author: 2026-09-13 / Codex.

- Decision: use LOC as a regression signal, not as a code-golf target.
  Rationale: the failure being repaired is duplicated machinery, but line count
  alone can reward compressed or opaque code. Acceptance therefore combines an
  exact baseline measurement with deletion of duplicate sources, API-boundary
  assertions, readable grouped data, and behavior tests.
  Date/Author: 2026-09-13 / Codex.

## Outcomes & Retrospective

Planning is in progress. No Batter or consumer implementation has been changed by
this plan. The current immutable Batter baseline remains `23a50fa`; the exact
consumer baseline remains in private provenance. The tracker update and review-round
record will be added after the plan reaches steady state.

At implementation completion, replace this paragraph with the exact upstream
commit, downstream pin, removed files and mechanisms, before/after measurements,
test receipts, review-loop result, limitations, and any unfinished acceptance.
Do not describe a non-converged review loop or an uncommitted path dependency as
completed pinned adoption.

## Context and Orientation

There are two repositories involved in the authorized local implementation
session. The tracked upstream plan and examples remain application-neutral; the
consumer repository retains its private provenance, exact paths, role names, and
measurements in a companion adoption record.

`batter-sqlx` is an optional SQLx 0.9 PostgreSQL adapter. Its public verification entrypoint is
`crates/batter-sqlx/src/verification.rs`; its policy model is
`crates/batter-sqlx/src/verification/policy.rs`; catalog inspection and evaluation
are under `crates/batter-sqlx/src/verification/authority/`; migration inspection
is in `crates/batter-sqlx/src/verification/migration.rs`; reports are in
`crates/batter-sqlx/src/verification/report.rs`. The runnable generic example is
`crates/batter-sqlx/examples/verification.rs`. `crates/batter-sqlx/AGENTS.md`
governs changes in this package.

The authorized private reference consumer pins five Batter packages to one Git
revision. It has separate schema, serving-role, and queue verification methods;
a generic tuple-to-`AuthorityPolicy` translator; four serving and queue grant
matrices; four scripts that repeat those grants; an existing operator command
tree; and shared restricted-role support using disposable PostgreSQL 18
databases. Exact repository paths and business names belong in the companion
consumer adoption record, not this upstream plan.

The low-level `AuthorityPolicy` makes three distinctions that the high-level API
must not erase. A required privilege must be usable by the entry current role.
An allowed privilege is the maximum authority that the authenticated login can
reach through direct grants, inheritance, `SET ROLE`, or administrative paths.
A PUBLIC allowance states whether PostgreSQL's implicit PUBLIC role may supply a
privilege. Ownership and grant options are separate authority. The exact-role
manifest is allowed to reduce repetition only because it gives these distinctions
explicit profile-wide defaults plus per-group overrides.

The phrase "grant atom" means one object, one privilege, and, for a column grant,
one column. Grouped input may contain many atoms. Capacity and duplicate checks
must operate on the expanded atom count so grouping cannot bypass the existing
10,000-entry and 32-privilege-list bounds.

The phrase "grant plan" means validated, normalized data from which an operator
can render `GRANT` statements. It is not proof that statements ran. It carries no
database URL, password, pool, connection, future, or retry behavior.

## Required End-State Interfaces

Create `crates/batter-sqlx/src/verification/manifest.rs` and re-export its public
types from `crates/batter-sqlx/src/verification.rs`. Names may change only if an
implementation discovery proves a collision or materially clearer contract; log
that decision before editing consumers.

Define `ExactRoleManifest` as the validated high-level input. Construction must
be possible from ordinary Rust without macros and without a network operation.
It must accept the following explicit data:

- the discovery scope and the primary application schema;
- database and schema privileges;
- grouped relation grants over one or more relation names and one or more
  privileges;
- grouped column grants over one relation, one or more column names, and one or
  more privileges;
- exact routine signatures using structural scalar or array type identities;
- per-relation row-type PUBLIC-USAGE allowance, defaulting to false;
- per-routine SECURITY DEFINER allowance, defaulting to false;
- discovered-object defaults, including separate ordinary sequence, type,
  invoker-routine, and definer-routine behavior;
- whether a declaration is required-and-provisioned or allowed-only;
- whether a declared required privilege may be delivered through PUBLIC;
- grant-option and ownership allowances, defaulting to false;
- optional serving-authority safeguards such as blanket current-database
  ownership. SQLx-ledger and schema-configuration inspection remain separate
  request components rather than fields on the role manifest.

Use explicit constructor types rather than tuples at the public boundary. The
target Rust shape is conceptually:

    pub struct ExactRoleManifest { /* private owned fields */ }

    pub struct RelationGrantGroup { /* private fields */ }

    pub struct ColumnGrantGroup { /* private fields */ }

    pub struct RoutineGrantSpec { /* private fields */ }

    pub enum DeclarationPurpose {
        RequiredAndProvisioned,
        AllowedOnly,
    }

    pub enum PublicDelivery {
        Deny,
        AllowDeclared,
    }

    pub struct CompiledExactRole {
        /* private AuthorityPolicy, GrantPlan, and authority safeguards */
    }

    impl ExactRoleManifest {
        pub fn compile(self) -> Result<CompiledExactRole, PolicyError>;
    }

    impl CompiledExactRole {
        pub fn authority_policy(&self) -> &AuthorityPolicy;
        pub fn grant_plan(&self) -> &GrantPlan;
    }

    pub struct SchemaInspectionPolicy { /* private owned fields */ }

    pub struct VerificationRequest<'a> { /* borrowed optional inspections */ }

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

`ExactRoleManifest` may use an ordinary builder or focused mutation methods, but
the finished value must own its strings and vectors so callers do not need
`'static` data or self-referential storage. Constructors for grant groups must
accept iterators or arrays and preserve clear call sites. Do not add a trait,
dynamic dispatch, proc macro, global cache, or serialization format merely to
reduce punctuation.

Reuse the existing `RoutineType` and `RoutineSignature` directly. Do not add a
second scalar/array identity enum or another conversion layer. Preserve the
layout and behavior of `AuthorityPolicy` and the other existing exhaustively
constructible public low-level policy structs. Additional safeguards live in the
new protected request types, not new mandatory fields on old structs.

Every new async entrypoint accepts the caller's `&OperationContext` and returns
the existing `Result<VerificationReport, OperationError<VerificationError>>`.
Validation remains inside `context.run` so an already-cancelled context is not
silently converted to a policy error. The protected wrappers delegate to the
existing executor and must not create their own timeout, cancellation token,
lease, rollback, or connection-disposition path.

`compile` must be a pure split-phase transform. It validates identifiers,
privilege/object compatibility, and routine types; expands groups with checked
capacity accounting; sorts by structural object identity; merges exact duplicate
atoms idempotently; and rejects contradictory declarations. Apply a checked
pre-expansion work bound before retaining arbitrarily large iterator input, then
apply the existing low-level final capacity rules, which count nested records,
privileges, repeated routine arguments, requirements, and PUBLIC entries rather
than merely distinct grant atoms. It creates the existing `AuthorityPolicy`; it
does not implement or bypass catalog evaluation.

A required-and-provisioned privilege appears in `required_privileges`, the
corresponding role allowance, and `GrantPlan`. An allowed-only privilege appears
only in the verification allowance. Discovery defaults, role attributes,
ownership allowances, allowed grant options, and PUBLIC allowances are ceilings
and never produce SQL. `PublicDelivery::AllowDeclared` adds the permitted exact
PUBLIC privilege. `PublicDelivery::Deny` creates the necessary exact empty or
reduced PUBLIC override so a permissive discovery default cannot restore the
denied privilege. Normalize one PUBLIC allowance per target and never emit
conflicting public grants and public overrides for the same target. Preserve
relation/column precedence: an exact relation declaration suppresses PUBLIC
column defaults, explicit column exceptions remain possible, and a column deny
cannot subtract a whole-relation privilege. Existing low-level
`AuthorityPolicy::validate` remains the final validator, but high-level input is
also checked before rendering so allowed-only invalid combinations cannot emit
unusable SQL.

Every exact relation declaration maps its explicit
`allow_row_type_public_usage` option to `RelationPolicy` and defaults it to false.
Every exact routine declaration maps its explicit `allow_security_definer`
option to `RoutinePolicy` and defaults it to false. An allowed-only declaration
may still declare an exact PUBLIC ceiling, including database
CONNECT/TEMPORARY; PUBLIC delivery does not imply a current-role requirement or
a role-targeted grant. An explicitly permitted SECURITY DEFINER routine may pass
only when its exact policy allows it; an unlisted or ordinary-default definer
remains rejected.

The initial grant plan renders required privileges without `WITH GRANT OPTION`.
Permission to tolerate an observed grant option is not permission to provision
one. If implementation includes grant-option provisioning, represent that intent
with a separate explicit declaration and test it independently.

Define `GrantPlan` as private-field normalized data with read-only accessors and a
pure rendering method. Rendering takes a validated PostgreSQL role `Identifier`
and, only when the normalized plan contains a database grant, an explicit
validated database `Identifier`. Missing database rendering context is a typed
error before any SQL is emitted; do not substitute `current_database()`, a psql
variable, or an environment setting. Rendering
quotes every role, schema, relation, column, and routine component through one
shared identifier renderer, groups only semantically identical statements, and
produces deterministic UTF-8 ending in newlines. It emits only role-targeted
`GRANT` statements. It must never emit `CREATE ROLE`, `ALTER ROLE`, `REVOKE`,
`BEGIN`, `COMMIT`, passwords, connection strings, or psql variables. The consumer owns
those surrounding decisions.

The output must preserve exact overload identities and use `GRANT EXECUTE ON
ROUTINE`, because the existing routine identity does not distinguish functions
from procedures. A routine statement renders
the schema, routine name, and canonical structural argument types. Do not accept
arbitrary SQL type expressions or resolve aliases by string. Reuse the existing
`RoutineSignature` and `RoutineType` validation rather than creating parallel
identity semantics.

The compiled role is the protected authority-verification input. Its dedicated
entrypoint delegates to the existing private executor and carries its authority
safeguards. The `authority_policy` accessor deliberately exposes only the ACL
policy for low-level use; callers choosing that escape hatch do not receive the
additional safeguard contract. `VerificationRequest` provides schema-only,
migration-only, authority-only, and combined modes through that same executor.

Keep the existing public `verify`, `verify_migrations`, `verify_authority`,
`AuthorityPolicy`, and all low-level policy structs available. The new API is
additive. The verifier receives the compiled low-level policy and therefore keeps
one execution path.

Compatibility here means the existing three function signatures and public
low-level policy-struct layouts remain unchanged and existing construction code
continues to compile. New finding and supported-surface distinctions may require
new variants in currently exhaustive public enums. Inventory every exhaustive
match in Batter, its examples, and the authorized consumer; update them in the
coordinated cutover and add a compile fixture that demonstrates the unchanged
low-level construction path. Do not call the overall enum surface fully
source-compatible. If enum compatibility becomes mandatory during implementation,
return safeguards through a new wrapper report instead of collapsing distinct
failures into old variants.

For SQLx migration shape, add a named opt-in high-level policy and entrypoint
rather than silently changing `MigrationPolicy::new`. The end state must support
both reference-adopter cases:

- exact installed history: the ledger must exist, have the SQLx 0.9 six-column
  shape and primary key, and contain exactly the expected successful versions and
  checksums;
- setup-compatible subset: the ledger may be absent, or every present row must
  have the standard shape, be successful, and match one expected migration; not
  every expected row must yet be installed.

Call these modes `SqlxLedgerMode::Exact` and `SqlxLedgerMode::InstalledSubset`
unless code evidence requires a clearer name. Add `SqlxMigrationManifest` with a
constructor accepting a qualified ledger and an iterator of version/checksum
expectations. If the public constructor instead accepts native SQLx migration
references, explicitly enable SQLx's `migrate` feature in `batter-sqlx`; do not
rely on workspace feature unification. Copy only validated version and checksum
metadata, consistently include `Simple` and `ReversibleUp`, exclude
`ReversibleDown`, retain no SQL bodies, and never invoke `Migrator`. Add
`verify_sqlx_migrations`; it must use the same owned lease,
transaction reset, captured snapshot, ledger locking, cancellation, capacity,
rollback, and connection-retirement machinery as existing verification. Do not
perform a preflight query outside that transaction.

Its public shape is conceptually:

    pub async fn verify_sqlx_migrations(
        pool: &PgPool,
        context: &OperationContext,
        manifest: &SqlxMigrationManifest,
    ) -> Result<VerificationReport, OperationError<VerificationError>>;

The schema-only and combined request paths use the same caller context and result
type. Add wrapper-level interruption regressions rather than assuming the
low-level executor tests cover argument validation and dispatch order.

The standard SQLx 0.9 ledger shape is exactly six non-dropped user columns:
`version:int8`, `description:text`, `installed_on:timestamptz`, `success:bool`,
`checksum:bytea`, and `execution_time:int8`, all NOT NULL, plus a primary key whose
key-column list is exactly `version`. Compare captured `pg_catalog` type identity,
not `typname`, aliases, or a later cache lookup. Preserve the baseline's lack of
column-order and default-expression requirements. Existing generic three-column
`MigrationPolicy` fixtures and entrypoints remain unchanged.

`InstalledSubset` is set membership, not prefix membership: any installed
successful version must match one expected checksum, while expected versions may
remain absent. It permits an absent ledger only when the captured snapshot also
shows it absent. A failed pre-snapshot relation-lock attempt is not enough. If the
ledger appears in the snapshot without the required protective lock, return an
explicit incomplete/native outcome; do not pass or inspect the new relation via
an unprotected path. A whole-transaction retry is optional future work. Retain
`ACCESS SHARE` locking and `ONLY` reads when the ledger exists. "No migration
lock" means no SQLx migrator advisory lock, not no relation lock; the check does
not reserve an absent name or certify state after its snapshot.

For SECURITY DEFINER search paths, add `SchemaInspectionPolicy` independent of a
serving role. Its explicit schema scope selects every definer routine, including
trigger functions and routines not granted to the authenticated login, and
requires exactly one `search_path` entry matching the canonical setting provided
by the application. The reference contract is the exact stored string
`search_path=pg_catalog, pg_temp`; absence, duplicate `search_path` entries, an
extra path item, a different order or spelling, `$user`, an unqualified value, or
no explicit setting is a structured violation. Unrelated `proconfig` entries such
as timeouts are preserved and accepted, matching the current consumer contract.
The initial implementation need not parse general PostgreSQL setting syntax or
treat alternative whitespace and quoting as equivalent. Bound both selected
routine count and retained configuration bytes;
prefer server-side extraction or bounding over fetching unbounded arrays. An
expected `pg_temp` path item is configuration data and must not select a temporary
namespace for ACL discovery. Report a distinct supported-surface marker for this
configuration check while continuing to mark routine bodies and trigger execution
semantics unsupported.

Provide schema-only execution through the existing owned executor, optionally
combined with `SqlxMigrationManifest`. It must work under owner credentials
without applying serving-role restrictions. The schema scope is independent of
`ExactRoleManifest`'s ACL discovery scope.

For blanket ownership, add an explicit authority safeguard that rejects ownership of any
current-database object reachable by the authenticated login or by a role it can
inherit or switch to. Reuse the verifier's captured role graph. Include the
database owner from `pg_database.datdba` and database-local shared-dependency
ownership rows from `pg_shdepend` where `dbid` is the current database,
`refclassid` is `pg_authid`, and `deptype` is ownership. Root reachability at the
authenticated login and retain inheritance, SET, ADMIN-derived potential, and
mixed SET-then-INHERIT paths; do not root it at masked `current_user`. The result
may identify an unfamiliar object class coarsely by catalog class, object
identifier, and `objsubid`; it need not model that class's ACL or execution
semantics. Known ACL, initial-ACL, policy, and tablespace dependencies are not
ownership findings. An unfamiliar but interpretable ownership class yields a
coarse finding; an uninterpretable ownership record, unknown form, or capacity
exhaustion yields incomplete/error, never a pass. Bound and count every retained
row and add a distinct supported-surface marker without claiming broader semantic
coverage.

## Plan of Work

### Milestone 0: Settle the inherited-verifier prerequisite independently

The inherited verifier prerequisite was settled independently on 2026-09-14.
`batter-7r3.12` was reviewed and closed first. The complete `batter-7r3.5`
baseline then received fresh Claude, Codex and Cursor review, repair and closure
passes at recorded fingerprints `e8db1344ab4c75fc97e992430979327e9cb6bb339d4f61f5590dc37b7ed823f2`,
`f93e980234eda91d2aec126131cb1b755e324d5e63162e6da707d115477353c3`
and `7115f8b50f746849928d898713aed05f6c2c9f4a0e249cfa918dfe49a94f7826`.
After the file-budget compaction and residual bookkeeping, all three reviewers
also confirmed complete fingerprint
`5d7f0561fafbf3b0e43e6577a3e85d4631cb073689a5e4f10e49223ded91c840`
with no actionable finding.
No terminal medium- or high-severity defect remained. Terminal low diagnostic
and live-oracle gaps are explicitly retained as `batter-mzi` and `batter-lod`.

The review covered inherited-ledger, temporary-namespace, PUBLIC-precedence,
capacity, role-authority, finding-identity and connection-disposition behavior.
Current-source validation includes both supported Rust toolchain matrices, all
ten rebuilt HTTP smokes, and passing standalone Jig test/format/Clippy/contract/
file-budget receipts. Credentialed PostgreSQL execution remains earlier Linux
evidence; changed live fixture bytes were not executed on the macOS closure host.
This evidence is prerequisite settlement only and is not a review of `.6`'s
later delivery diff.

This milestone is accepted when `.5`'s own tracker and retained review artifact
show a converged review of the selected immutable verifier baseline, its blocking
defect is settled honestly, and `.6` can consume that baseline without inheriting
unreviewed work. Pure Milestone 2 code may proceed in parallel while this evidence
is pending; Milestones 3 through 7 may not finish first.

### Milestone 1: Freeze behavior and consumer-size baselines

Work in both repositories without changing production behavior. In Batter, add a
small fixture policy matching a generic service role and record the exact
low-level `AuthorityPolicy` it creates today. In the authorized private reference
consumer, add or identify characterization tests for all four profiles. Each test
must prove a successful freshly provisioned role, one missing
grant, one excess grant, a forbidden grant option, a forbidden sequence grant,
an unsafe SECURITY DEFINER search path, and reachable ownership. Existing tests
may satisfy cases if their assertions reach the public startup method and use a
real restricted login; record the mapping rather than duplicating them.

Before replacement, serialize or compare the normalized current low-level
authority policy for every profile as an independent oracle. The baselines must
cover requirements, role allowances, discovery defaults, exact PUBLIC
precedence, role limits, ownership, sequence behavior, grant options, relation
versus column behavior, and routine identities. They must be constructed from the
current implementation, not from the new manifest under test.

Reproduce the consumer's historical baseline and current adoption totals using
the exact revision and old-path inventory recorded in the private companion. That
list is historical only because it cannot contain replacement files that did not
yet exist. Before implementation, create a responsibility-based inventory
containing every old and replacement profile, translation, schema-checking,
provisioning, operator-parser, command-renderer, and directly supporting module.
Use the union of those old and new paths for a whole-file metric and recount that
identical union at every compared revision. Also report changed-line and
responsibility-level counts where a shared file has unrelated code. Never compare
the final implementation against a list that omits its new files. Count tests,
documentation, generated SQL, and upstream Batter changes separately. Do not set
a CI line-count gate; this measurement is an acceptance audit paired with
semantic tests, not a code-golf target.

This milestone is accepted when current behavior is mapped to executable tests,
all four independent low-level policy oracles are captured, the private historical
measurements are reproducible, the replacement-inclusive measurement inventory is
recorded in the private consumer handoff, and no behavior change has been made.

### Milestone 2: Add and prove the exact-role manifest compiler

Work in the Batter repository. Add the high-level module and types described
above. Implement construction, normalization, capacity accounting, conversion to
`AuthorityPolicy`, and deterministic grant-plan rendering as pure code. Reuse
existing identity, privilege, validation, and low-level policy types. Do not move
catalog SQL or change verifier execution in this milestone.

Add offline tests under `crates/batter-sqlx/src/verification/manifest/tests.rs` or
the nearest existing test organization. At minimum prove:

- two differently grouped manifests that contain the same atoms compile to the
  same normalized policy and grant plan;
- repeated identical atoms are idempotent, while duplicate object policies with
  incompatible ownership, grant-option, PUBLIC, or required settings fail;
- relation and column privileges remain distinct;
- required privileges, allowed reachable privileges, and PUBLIC delivery are not
  conflated;
- PUBLIC denial remains effective under permissive schema, column, type, and
  routine discovery defaults; mixed per-privilege PUBLIC settings normalize
  without overwriting one another;
- allowed-only declarations, discovery defaults, ownership allowances, and
  grant-option ceilings produce no SQL;
- invalid privilege/object combinations fail even when allowed-only;
- scalar and true-array routine arguments preserve structural identity and
  deterministic rendering;
- invalid and overlong identifiers fail before network work;
- capacity is charged after expansion and an oversized group fails without a
  large partial result;
- rendering quotes embedded double quotes correctly, cannot emit more than the
  normalized grant set, and never renders credentials or role creation;
- database grants require explicit database rendering context, while a plan with
  only allowed database authority requires none;
- `ON ROUTINE` output preserves zero arguments, scalar/array overloads, quoted
  identifiers, and qualified types as deterministic text; live function and
  procedure execution is deferred to Milestone 3;
- exact relation row-type PUBLIC-USAGE and routine SECURITY DEFINER options map
  to the existing low-level fields, default false, and distinguish an explicitly
  permitted definer from an unlisted one;
- output order is stable across input order and duplicate placement.

Update the generic runnable verification example so its policy construction uses
`ExactRoleManifest`, while retaining an explicit low-level policy example in
rustdoc. Do not switch its execution path until Milestone 3 provides the
protected entrypoint. The example must remain generic and contain no consumer
names.

This milestone is accepted when all pure manifest tests pass, example
construction uses the high-level manifest while execution remains on the
existing verifier, low-level construction still compiles, and a diff inspection
shows no second ACL evaluator or database I/O in the new module.

### Milestone 3: Move the remaining generic catalog safeguards upstream

Implement `SqlxMigrationManifest` and `verify_sqlx_migrations` through the existing
verification executor. Extend captured authority inspection with the opt-in
search-path and blanket-ownership safeguards. Keep all queries inside the same
captured transaction and shared capacity budget. Reuse the current role graph and
report types. Add new finding or unsupported variants only when existing variants
cannot identify the violated invariant without ambiguity.

Add real PostgreSQL 18 cases to the existing live verifier suite. Migration tests
must cover exact success, absent exact ledger, optional absent subset ledger,
valid non-prefix partial subset, empty expected and installed sets, duplicate
expected versions, migration-kind filtering, unknown row, failed row, checksum
mismatch, an extra or missing column, wrong type or nullability, a same-spelling
type outside `pg_catalog`, wrong primary key, 10,000- and 10,001-row boundaries,
inherited ledger, RLS, and cancellation while blocked. Add deterministic
concurrency controls for a ledger appearing after an initial missing-lock result
but before snapshot classification, and for present-ledger replacement or shape
change around locking. Coordinate through observed database locks or a narrowly
scoped test seam, never sleep-only timing. Preserve a low-level three-column
ledger acceptance case.

Search-path tests must cover correct exact stored configuration, missing
configuration, duplicate `search_path` settings, an accepted unrelated routine
setting, alternate whitespace/quoting, extra or
reordered entries, `$user`, a trigger function, a routine not otherwise granted
to the login, a single oversized configuration value, count/byte capacity, and a
concurrent catalog change serialized or reported according to the
captured-snapshot contract. Ownership tests must cover direct, inherited,
SET-reachable, and ADMIN-reachable owners; the current database itself; a
relation already covered by specific inspection; a large object or another
coarse `pg_shdepend` class; clean controls; non-ownership dependency types;
unfamiliar owned classes and uninterpretable records; capacity; and current-role
masking.

Protected-wrapper integration tests must prove an already-cancelled context
before polling, cancellation while pool capacity is occupied, and interruption
during migration, schema, and authority-safeguard inspection without changing
lease disposition. Execute renderer output against both a function and a
procedure, covering zero arguments, scalar/array overloads, quoted identifiers,
and qualified types. Switch the generic runnable example to
`verify_exact_role`, then run its normal and deliberate-violation modes against
PostgreSQL 18.

Register every new live module from `tests/verification_live.rs`. Because
`batter-sqlx` has `autotests = false`, add an explicit Cargo target if a separate
test binary is used. Update `scripts/sqlx_live.py`'s exact ignored-test inventory
and its runner-control tests for every added or renamed case; compare the
discovered inventory before treating a live run as evidence. Keep exhaustive Jig
input scopes synchronized in `.jig.toml` and `.agent/jig-contract.json` if any new
source, script, or fixture path falls outside current patterns.

Run guard-removal mutations for each new safeguard test family. Temporarily
disable the ledger-shape predicate, exact search-path comparison, and coarse
ownership row evaluation one at a time. Each mutation must make a named test fail.
Restore exact source bytes before continuing and record the commands and failures
in `docs/validation.md`.

Update `crates/batter-sqlx/README.md`, package `AGENTS.md`, `docs/guarantees.md`,
`docs/integrations.md`, `docs/status.md`, `docs/testing.md`, and
`docs/validation.md`. Change the old statement that exact SQLx ledger shape and
search-path inspection always remain downstream. Continue to state that migration
selection, grant contents, routine body semantics, actual provisioning, owner
credentials, and application error classification remain downstream.

Document that installed-subset shape, RLS, inheritance, and capacity rejection
is a deliberate tightening if the reference adopter's former unbounded query
accepted those states. Do not describe it as unchanged legacy behavior.

This milestone is accepted when the new live cases and mutations prove the
behavior, existing verifier cases remain green, and documentation distinguishes
supported safeguards from still-unsupported semantics.

### Milestone 4: Prototype the protected path in a real consumer before API freeze

Before freezing the public API or asking for an upstream commit, use an
uncommitted local path dependency in the authorized private reference consumer.
Exercise all four real role profiles, schema and queue startup, exact and
installed-subset ledgers, schema search-path inspection, ownership denial, and
the render-only operator path. The prototype must call high-level Batter
constructors directly and must not preserve or recreate a local generic
translator. Record constructor friction and revise the public boundary in Batter
while both working trees remain uncommitted.

After the Milestone 1 oracles and old-script outputs are captured, delete the
translator and duplicate scripts in this uncommitted prototype so the real final
shape and inclusive LOC result are measurable. The original bytes remain
recoverable from the immutable consumer baseline or an isolated comparison
checkout. Do not commit the local path dependency or describe these recoverable
prototype deletions as pinned adoption.

Compare every compiled role against the independent low-level policy oracle from
Milestone 1. Compare old-script and new-renderer effects semantically on separate
fresh disposable roles before deleting any scripts: provision each path, inspect
effective/reachable authority through the verifier, and exercise real operations
for lock-only column updates, quota changes, authentication transitions, queue
publication, promotion, claim, and completion. Do not use the new manifest as
both implementation and sole expected-output source.

Run the replacement-inclusive LOC inventory while the prototype exists. Reject
the API design if equivalent translation or a second grant matrix remains in the
consumer, if the four profiles are not readable grouped policy data, or if the
inclusive hand-authored responsibility count does not improve. Keep exact
consumer names, paths, role identifiers, and measurements in the companion
consumer record; upstream evidence states only the generic result and references
the authorized handoff.

This milestone is accepted when all real profiles compile and execute through
the protected path, independent equivalence oracles pass, the consumer has no
generic adapter, and the inclusive size result demonstrates actual downstream
reduction. A local path is explicitly prototype evidence, not pinned adoption.

### Milestone 5: Validate and make an immutable Batter revision available

Run the complete Batter validation against the finished upstream source. Use the
repository-pinned Rust 1.98.1 and minimum Rust 1.94.0 toolchains. Run the exact
PostgreSQL 18 live suite, both outcomes of the runnable verifier, all required HTTP
smokes because the root verification script covers the workspace, strict Clippy,
rustfmt, rustdoc/doctests, Python controls, and Jig work gates. Record exact test
counts, platform, PostgreSQL version, lock hash, commands, failures, corrections,
and missing hosted evidence.

Run a comprehensive review-fix loop over the complete Batter diff with
`--fix-mode comprehensive --min-severity low`. Use the repository's configured
bounded rounds. A converged loop is required before requesting commit. If it does
not converge, stop at the last green state, preserve the findings, and do not
present the API as ready for consumer pinning.

Committing and pushing Batter require explicit user authorization. The reference
consumer may use the Milestone 4 temporary local path only for development proof
and must not record pinned adoption. Once authorized and published, record the
exact commit and verify that the remote resolves it before changing the consumer
pin.

This milestone is accepted only with a converged review loop, all required local
checks green, and an immutable Batter revision available to the reference
consumer. A working tree hash or local path is not sufficient.

### Milestone 6: Pin and complete the private reference-consumer adoption

Work in the authorized private consumer only after Milestone 5 provides an
immutable Batter revision. Update all five Batter package pins together and
review every upstream commit in the range as required by its adoption
contract.

Replace the generic translator and separate queue-authority module identified in
the private companion with one focused application-owned profile module. Use the
high-level Batter types directly; do not recreate `Privilege`,
`RelationGrant`, `RoutineGrant`, argument-type conversion, BTree grouping,
required/public expansion, or default-policy construction. Group relations by
identical privilege sets and group columns by relation plus privilege set. Keep
profile names, exact grant data, and remediation messages downstream.

Use `SqlxMigrationManifest` for the consumer's exact application and durable
operation histories and for its setup-time installed-subset rule. Remove its
generic ledger-shape method and raw catalog query from both startup verification
and serialized migration setup.
Use the independent Batter schema-inspection policy for the public schema with
the exact stored `search_path=pg_catalog, pg_temp` contract. The consumer's public
schema-verification method must continue to perform this check under both
owner/setup and restricted credentials, and failures must retain the established
schema-error category. Its serving-role, administrator-role, and queue-role
methods continue to classify only authority inspection. Select blanket ownership
denial in those compiled role profiles. Remove the local `pg_proc` search-path
query and `pg_shdepend` ownership query. Keep the application-specific durable
operation schema-compatibility call in the queue-schema method; do not move that
protocol upstream. Preserve public method boundaries, error categories, and the
current order of startup and setup stages without exposing native details.

Add the render-only operator subcommand specified in the private companion.
It accepts one of the four fixed profile names and a target `ROLE`; `ROLE` is
validated as one PostgreSQL identifier through Batter's identifier type. The
command must be parsed and executed before ordinary application configuration is
loaded, while preserving the existing earliest SSH transport dispatch. It must
not bypass, consume, or reinterpret that transport protocol. It performs no
filesystem reads beyond normal executable loading, no database connection, no
environment secret lookup, no tracing initialization, and no provider work.

The command compiles the selected consumer manifest, renders all role-targeted
grants into memory, wraps them in `BEGIN;`, the existing
`REVOKE CREATE ON SCHEMA public FROM PUBLIC;`, and `COMMIT;`, then locks stdout,
uses `write_all` for the complete buffer, and explicitly flushes it. Schema USAGE
comes from the manifest and is rendered exactly once. The consumer's database
CONNECT/TEMPORARY rules are allowed-only PUBLIC ceilings, so this command needs no
database argument and emits no database `GRANT`. Diagnostics go to stderr and
contain no SQL fragment derived from invalid input. An invalid profile or role
returns nonzero and emits no stdout bytes. The renderer does not revoke existing
role grants; startup continues to reject extras. An output error returns nonzero
but makes no claim about how much a downstream pipe received.

Update README/operator guidance to replace the four static scripts with a
pipeline of this form, substituting the consumer's concrete executable,
subcommand, profile, role, and setup URL from the private companion:

    set -o pipefail
    cargo run --locked -p APPLICATION -- RENDER_SUBCOMMAND PROFILE ROLE \
      | psql -X "$OWNER_DATABASE_URL" -v ON_ERROR_STOP=1

Successful rendering proves only output generation. Successful application still
requires the restricted role to pass startup verification. Keep the existing
operator warning that revoking PUBLIC schema CREATE affects other users and that
the generated transaction does not remove excess grants already held by the role.

Remove the four provisioning scripts named in the private companion only after
the command output is proven equivalent on disposable roles. Update active guides
and docs that name them. Do not add automatic role creation, passwords, owner
credential loading, or bootstrap-time grants.

Adapt the restricted-role test support to provision from the same rendered
manifest or `GrantPlan`. Do not keep a separate test-only grant list. For each
profile, provision a fresh role, prove successful public startup verification,
then mutate one grant and prove the same missing/excess/unsafe categories as the
baseline. Preserve all queue and identity tests, including lock-only column
updates, quota column restrictions, routine overloads, sequences, PUBLIC,
inheritance, SET reachability, ADMIN reachability, and coarse ownership.

Exercise the built executable through the real command parser with absent or
invalid serving configuration, secret-file settings pointing to nonexistent
files, invalid profiles, invalid Unicode/identifier input, extra arguments, and a
closed stdout sink. A valid render invocation must still succeed without loading
application settings. Assert stable exit status and stdout/stderr. Run a truncated
transaction and a transaction containing a deliberately failing `GRANT` through
`psql -X -v ON_ERROR_STOP=1`; neither may commit a partial grant set.

Update every live consumer of the deleted provisioning scripts, including shared
test-role construction, private DB queue tests, API source support, process
diagnostics, worker tests, and administrator lifecycle tests. Read each owning
guide before editing its package. Remove obsolete SQLx metadata for deleted raw
queries through a fresh prepare cycle. Update active README, environment, and
package guidance, but preserve historical audit and adoption records as
historical evidence rather than rewriting them to invented current paths.

This milestone is accepted when there is one application grant source, no local
generic policy compiler or generic catalog safeguard remains, render-only
provisioning is explicit, and all characterized behavior still passes.

### Milestone 7: Prove actual downstream reduction and finish adoption

Run the consumer's formatting, strict Clippy, combined Rust tests, complete disposable
PostgreSQL 18/Git suite, lifecycle/fixture shell scenarios, fresh SQLx prepare
checking, both frontend coverage suites, and every path-aware Jig gate required by
the final diff. Use the consumer repository's pinned tool launcher as required by
its guide.

Run a comprehensive low-severity review-fix loop over the complete consumer working
changes with `--fix-mode comprehensive --min-severity low`. The review scope must
include generated/operator SQL rendering, all four profiles, migration modes,
report-to-error mapping, startup call sites, tests, documentation, dependency
pins, and deleted scripts. If the loop does not converge, stop and report the
remaining findings; do not commit the consumer adoption.

Re-run the exact baseline line-count procedure. Acceptance requires all of the
following, not merely a favorable total:

- the replacement-inclusive affected-code inventory is smaller than its
  pre-adoption baseline when every revision is measured with the same final path
  union and responsibility rules;
- the private historical figures and replacement-inclusive totals are reported
  separately downstream, with no profile, renderer, operator-parser, or
  supporting translation code omitted;
- the translator and queue-authority files named in the private companion are
  deleted rather than replaced by equivalent local adapters;
- the four manually maintained provisioning scripts are deleted and their
  render-only replacement is generated from the startup manifest;
- all four profiles remain readable grouped application data;
- any generated SQL size is reported separately from hand-authored Rust and is
  not counted as removed semantic policy;
- total consumer code LOC before and after the complete Batter adoption is reported
  with the same tool and exclusions used for both revisions;
- Batter LOC is reported separately so complexity moved upstream is visible.

Update the consumer's private Batter adoption and feedback records with the old
and new pins, the incorrect first adoption, the final removed mechanisms, the one
remaining application policy source, every failed approach, exact validation,
review-loop result, and LOC measurements. Append only generic final downstream
evidence to `batter-7r3.6`; do not rewrite or delete prior comments. Close the Bead
only if its other existing acceptance requirements, including actual pinned
consumer and honest second-consumer/release limits, are satisfied. Otherwise
leave it open and state exactly what remains.

## Concrete Steps

From the Batter repository root, begin by confirming the immutable source and
tracker record:

    git status --short --branch
    git rev-parse HEAD
    bv --robot-triage
    br --no-db show batter-7r3.6 --json

The recorded implementation DAG, written as `dependent -> prerequisite`, is:

    batter-g19 -> batter-7r3.5 -> batter-7r3.12
    batter-r2a -> batter-9jh
    batter-r2a -> batter-g19
    batter-7r3.6 -> batter-r2a
    batter-7r3.6 -> batter-7r3.5

`batter-9jh` is the independently ready pure manifest/compiler/renderer task;
`.13` remains related evidence. Distinguish committed
implementation availability from tracker completion. Do not close or remove
prerequisites from consumer-only evidence or create a dependency cycle.

Expect HEAD to be at or descend from `23a50fa`. Preserve unrelated changes. The
known JSONL comment for the reference adoption may already make
`.beads/issues.jsonl` dirty; do not reset it.

Create a Jig work record for `batter-7r3.6` using the current baseline and this
plan, following the exact current `scripts/jig work start --help` contract. Do not
copy a stale command from another plan. During implementation, run focused checks
after each milestone:

    cargo fmt --all -- --check
    cargo test -p batter-sqlx --features test-support --locked
    cargo clippy -p batter-sqlx --all-targets --features test-support --locked

Configure the disposable PostgreSQL 18 prerequisites exactly as documented in
`docs/testing.md`, then run:

    bash scripts/test_sqlx_live.sh

Run the complete Batter checks after the upstream source is final:

    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh

Build and execute every HTTP smoke profile listed in `docs/testing.md` for both
toolchains. Run the normal and deliberate-violation modes of the verification
example. Run the applicable Jig work check, evidence, gates, and finish commands
using the plan ID created for this work. Expected outcome is zero failed required
targets and fresh receipts matching the final source fingerprint.

After an authorized immutable Batter commit exists, follow the private companion
to confirm the exact consumer baseline and update all Batter pins together. Run
focused database checks while migrating one profile at a time, then every
documented consumer gate. Use the exact current Jig work contract. Do not treat a
skipped PostgreSQL prerequisite as a pass; SQLx metadata checks use a fresh
migrated PostgreSQL 18 database.

Exercise render-only provisioning for all four profiles. For each profile, first
capture output without a pipe and inspect that it begins with `BEGIN;`, contains
only the selected role, includes the expected global schema revoke and grant, and
ends with `COMMIT;`. Then provision a fresh disposable role through `psql` and run
the corresponding public consumer verifier. Invalid profile and invalid identifier
controls must return failure with zero stdout bytes.

Run the repository's configured review-fix-loop command exactly as documented by
the installed Jig review skill, with comprehensive fixes and minimum severity
low. Store review artifacts outside product code or in the repository's designated
review evidence path. Re-run any gate invalidated by a repair.

Reproduce the private historical LOC measurement for continuity, but measure the
final outcome with the replacement-inclusive path union from Milestone 1 at the
pre-adoption, current-adoption, prototype, and final revisions. Treat missing
paths as zero. Also run one consistent repository code-LOC command across all
revisions. If the earlier command cannot be recovered, recompute every revision
with one documented tool and label older figures historical rather than directly
comparable. Keep exact revisions, paths, and totals in the consumer record; put
only the generic reduction result upstream.

## Validation and Acceptance

The high-level API is accepted only when a generic example can express
an exact role without constructing `AuthorityPolicy` fields manually, the compiled
policy is structurally equivalent to the low-level oracle, and live PostgreSQL
tests produce the same pass/missing/excess/incomplete outcomes.

The grant plan is accepted only when it is deterministic, identifier-safe,
structurally tied to the compiled policy, and unable to perform I/O. A role
provisioned from it must pass the same manifest's verifier. Removing one generated
grant must produce a missing finding. Adding one forbidden grant or grant option
must produce an excess finding. These are end-to-end semantic tests, not string
snapshots alone.

The migration helper is accepted only when exact and installed-subset modes
preserve the consumer's set-membership, checksum, success-state, and absent-ledger
semantics over real SQLx 0.9 ledgers. Standard-shape enforcement and conservative
RLS, inheritance, capacity, and late-attachment handling are intentional
tightenings and must be characterized, documented, and accepted through the
consumer's existing error boundary. The helper performs no DDL or migrator
advisory locking; it retains the necessary relation lock. Cancellation must retain
existing lease retirement and rollback evidence.

The search-path safeguard is accepted only when it inspects every selected
SECURITY DEFINER routine, including ungranted trigger functions, requires exact
ordered configuration, and does not claim to inspect bodies. The ownership
safeguard is accepted only when direct and reachable owners of both modeled and
coarse database-local objects are rejected, the database owner is included, and
unknown or over-capacity catalog state cannot pass.

Reference adoption is accepted only when all four profiles are provisioned and
verified from one manifest representation, all old behavior regressions pass, and
the render-only command has no configuration or external side effects. Existing
operator and serving command parsing must be unchanged outside the new exact
subcommand.

The reduction claim is accepted only when the replacement-inclusive affected-code
inventory is smaller than its recomputed pre-adoption value and source inspection
confirms no equivalent adapter or duplicate grant matrix moved elsewhere in the
consumer. A compact macro, generated Rust blob, or one-line include of duplicated
data does not qualify. Report Batter growth and generated output separately.

The overall Bead may be closed only after the upstream review loop converges, an
immutable upstream revision exists, the downstream review loop converges, all
required gates are green on final fingerprints, adoption evidence is appended,
and no existing `batter-7r3.6` acceptance item is falsely claimed. A second
independent adopter, hosted CI, release, or deployment remains incomplete unless
actually executed and evidenced.

## Idempotence and Recovery

Manifest compilation and grant rendering are pure and repeatable. Renderer tests
must compare complete strings so a second invocation is byte-identical. The
rendered transaction is safe to rerun for already-present grants because
PostgreSQL `GRANT` is idempotent for the same authority. It intentionally does not
remove unrelated existing grants; startup verification detects those and tells
the operator to correct or recreate the role.

All live tests use disposable databases and roles with unique names. Register
cleanup before provisioning. If a test fails after role or object creation, use
the harness's retained cleanup report and explicit outer cleanup; never weaken a
test to leave cluster-global roles behind. Do not run provisioning against a
shared or production database.

Keep the existing low-level API working while the high-level layer is added. If a
manifest design step fails, revert only that focused working-tree edit and keep
the last green low-level verifier. In the consumer prototype, do not delete the
old translator or scripts until their independent policy/original-SQL oracles are
captured and behavioral equivalence passes. After that evidence, recoverable
uncommitted deletion is required before upstream API freeze. Final committed
deletion still waits for the immutable Batter pin.

During temporary local-path consumer development, clearly mark the path as
uncommitted evidence. Before final consumer pinning, restore Git dependencies and
verify the exact remote commit. Do not commit a local path dependency or claim it
as immutable adoption.

The tracker database conflict is a separate prerequisite for tracker mutations.
Use `br --no-db` for inspection. Before mutation, identify and validate the exact
supported `br --no-db` command against a recoverable copy, compare the target
issue and dependency records before and after, and preserve the current
`batter-7r3.6` comment. Do not invent a raw JSONL-edit workflow, run
`br sync --flush-only` against a known-stale database, or overwrite unrelated
records. Implementation and read-only evidence gathering may continue if tracker
write recovery is unavailable, but the Bead must not be called updated or ready.

## Interfaces and Dependencies

The implementation stays within the existing Batter-to-SQLx dependency direction.
`batter-sqlx` may use `sqlx` and `batter`; it must not make the foundation crate
depend on SQLx. No serde, TOML, SQL parser, proc-macro crate, ORM, database
provisioner, role manager, or application repository is added.

The exact-role manifest depends on existing `Identifier`, `QualifiedName`,
`RoutineSignature`, `RoutineType`, `ObjectPrivilege`, `AllowedPrivilege`,
`DiscoveryScope`, `DiscoveryDefaults`, `AuthorityPolicy`, and `PolicyError`.
Where the existing error enum cannot describe contradictory high-level input,
add a precise variant and cover its Display projection without embedding input
data.

The grant renderer depends only on the compiled normalized plan and `fmt::Write`
or `String`. It performs no async work and accepts no database handle. Expose
read-only normalized records if the consumer needs to add its wrapper; do not expose
mutable internal policy vectors that can make the renderer and verifier diverge.

The migration helper depends on SQLx 0.9 migration metadata already present in
the adapter. It must not run `Migrator`, take SQLx's migrator advisory lock, or
change migration tables. It shares the verifier executor and report/failure
contracts.

The consumer continues to own profile names, exact application relations,
columns and routines, role selection, PUBLIC-schema CREATE revocation, operator
messaging, error classification, durable-operation compatibility, and explicit
execution through `psql`. Its test support may consume the compiled grant plan
but must not define a parallel list.

No public HTTP contract, durable application data, queued payload, migration
file, or deployment resource changes. No migration is needed for this refactor.
If implementation uncovers a separate schema requirement, follow the reference
consumer's explicit greenfield hard-cutover policy rather than inferring a
migration or database recreation from this plan.

## Plan Revision Note

Initial plan written 2026-09-13 from Batter `23a50fa` and an authorized immutable
reference-consumer revision recorded only in the private companion. It
incorporates the post-adoption finding that generic evaluator reuse increased,
rather than reduced, the consumer's hand-authored verification responsibility.

Planning review round 1 was integrated on 2026-09-13. It changed the design from
authority-owned search-path fields to separate schema and role requests, split
allowed ceilings from provisioning intent, made exact PUBLIC overrides explicit,
defined database/routine rendering context, specified the absent-ledger race and
SQLx feature graph, bounded safeguard payloads, inserted a pre-freeze real
consumer prototype, replaced the incomplete LOC comparison, registered live-test
surfaces, and hardened render-pipeline semantics. This was a structural round,
not steady state.

Planning review round 2 was integrated on 2026-09-13. It restored the caller's
`OperationContext` and actual error type in every protected entrypoint, added an
independent `.5` prerequisite-settlement milestone, mapped row-type and definer
options, preserved unrelated routine configuration, made prototype deletion
recoverable, removed the obsolete absolute LOC gate, completed upstream/private
provenance separation, and narrowed the compatibility claim to the contracts that
can actually remain source-compatible. This was another structural round. Record
rounds 3 and 4 below before updating the Bead as implementation-ready.

Planning review round 3 was integrated on 2026-09-13. It found one remaining
sequencing defect: the pure compiler milestone required protected executor and
live PostgreSQL behavior that does not exist until the safeguard milestone. The
protected cancellation, live routine rendering, and runnable execution checks now
live in Milestone 3; Milestone 2 is independently implementable while `.5`
settlement is pending. No other structural revision survived that review.

Planning review round 4 completed on 2026-09-13 against fingerprint
`e0345411366518edceb8ececf90a92738984abc59250e4795010a98ed063e682` and found no
further implementation-blocking issue. The plan reached steady state. The only
retained sequencing condition is explicit: pure manifest work can start now,
while safeguard execution and terminal adoption wait for `.5 -> .12` settlement.
