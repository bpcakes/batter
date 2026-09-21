# batter-sqlx

An independently selected PostgreSQL adapter for SQLx 0.9 and Batter. Rust 1.94
minimum, Unix-only, unpublished. The foundation does not depend on this package.

## Owned transactions and snapshots

With an existing operation budget, prefer
`run_atomic_in(&pool, &context, "operation.name", async |scope| ...)`.
It returns `Result<T, OperationError<PgAtomicError<T, E>>>` and retains the entire
native outcome before cancellation/deadline resolution. There is no await
between acknowledged disposition and retention. A confirmed commit cannot be
replaced by a later local timeout; an unobserved commit remains uncertain and
must not be replayed automatically. The callback has exactly the same constrained
SQL capability as `run_atomic`. Dropping the outer future still loses its result.

Use `run_atomic(&pool, async |scope| ...)` for application and library writes
that share one READ COMMITTED transaction. The callback uses
`scope.application(async |sql| ...)` and ordinary SQLx queries through
`sql.executor()`. Outputs are provisional inside the callback; the runner
returns `Ok(T)` only after acknowledged commit. It returns `Rejected(E)` only
after acknowledged rollback. Unconfirmed commit/rollback retain the body output
or rejection with the original cause, under `PgAtomicError::Uncertain` and the
narrow `PgAtomicUncertainty` enum. A lost scope retains the callback result and
the first terminal database cause even if the callback catches it. Dropping a
polled operation records `PgScopeLoss::OperationAbandoned` instead of inventing
a database failure. Later operations return the retained loss without running SQL.
`PgScopeError::Application` is a recovered operation rejection;
`PgScopeError::Terminal(PgScopeFailure)` cannot contain a plain business rejection.

Each application operation owns a private savepoint and validates the original
XID, isolation and access mode. Recoverable errors roll back their savepoint;
terminal failure or cancellation consumes the usable owner even when the body
catches the error or abandons an inner future. There is no await after completion
acknowledgement. Arbitrary SQL and captured external side effects are not a sandbox:
explicit COMMIT can have irreversible effects, but cannot yield canonical success.

Every atomic and snapshot completion retires its physical connection, including
acknowledged commit/rollback. Acquisition executes ROLLBACK, clears SQLx's statement
cache, and executes DISCARD ALL before BEGIN. This removes inherited settings,
temporary objects, roles and session advisory locks; pool `after_connect` session
customizations are intentionally reset. Express transaction-local settings inside
the scope. Failed or cancelled reset retires the connection. Retirement releases
local pool capacity, not synchronous proof of backend termination.

`PgReadOnlySnapshot::inspect(&pool, async |sql| ...)` owns a REPEATABLE READ READ
ONLY transaction and passes the distinct `PgReadOnlySql` capability. A helper
requiring `PgScopedSql` or `PgExecutor` cannot accept it. Arbitrary SQL text is
still checked by PostgreSQL. Inspection errors produce a clean `Inspection(E)`
only after rollback to the original private guard, guard release, characteristic
validation and acknowledged outer rollback. Boundary loss retains both causes.

The snapshot establishes no SELECT before the inspector, allowing it to lock
authoritative objects before its first snapshot query. Qualify authoritative names;
the local search path is `pg_catalog, pg_temp`. Catalog cache functions may not
obey MVCC; use direct catalog reads and appropriate DDL locks.

`low_level::PgAtomicTransaction` is the exceptional consuming-owner API. It leaves
provisional-output/completion pairing with the caller and is not the canonical
consumer path. Both APIs target PostgreSQL 18. Bound the entire workflow with an
OperationContext when required. Cancellation returns no result and does not
prove rollback; process death or deliberately forgotten values remain outside
local destruction guarantees.

## Low-level session operations (not transaction workflows)

`PgLease::acquire(&pool, &context)` bounds acquisition using the existing operation
budget. Move the lease into the bounded operation future and run its work through
`lease.with_connection(async |session| ...)`: after application work returns
`Ok`, Batter runs an idle-state handshake ending in `ROLLBACK` on that exact
physical connection and returns it to the pool only when the final rollback
succeeds. The handshake opens and rolls back an empty transaction when already
idle, avoiding PostgreSQL's outside-transaction warning. `Err`,
failed or interrupted cleanup, unwinding and application interruption retire it.
There is no direct pool-return call, so an interrupted query can never be followed
by pool return. Work that must retire for every outcome uses the consuming
`lease.with_retiring_connection(async |session| ...)` path. Both closures receive
an opaque `PgSession`: `session.executor()` runs native SQL, but there is no
`session.begin()` or public `PgTransaction`. Use `run_atomic` for application
transactions; only `low_level::PgAtomicTransaction` offers exceptional manual
ownership. The session does not implement `DerefMut` or `AsMut`, so application
code cannot replace the physical connection before disposition. Low-level
session results are not transaction evidence. Native execution permits raw transaction
control such as `BEGIN`; the same-connection `ROLLBACK` closes both open and
failed raw transactions before the private pool-return proof can exist. It does
not reset arbitrary session settings, session advisory locks or prepared
transactions. See the crate rustdoc for compiling and compile-fail examples of
both paths.

Retirement releases local pool accounting. It does **not** acknowledge remote
cancellation, rollback, or server-session disappearance. Interrupted SQL can
retain server locks after local drop, and detached sessions can exceed the
pool's `max_connections` and outlive `Pool::close`. Applications own remote
outcome reconciliation and server-side resource policy. Ordinary successful
return is SQLx's asynchronous health-check path, not an immediate reuse promise.

`probe(&pool, &context)` performs SELECT 1 with acquisition and query sharing the
total budget. Reserve cleanup first, then use
`pool_in(slot, PgPoolOptions, PgConnectOptions)` to construct a native lazy pool
whose awaited close hook is registered before the pool is returned. This return
does not establish connectivity or readiness, and native minimum-connection
maintenance may begin during construction. Release checkouts and join dependent
work before cleanup expects `Pool::close` to complete.

`register_pool_close(&mut supervisor, name, &pool)` remains the lower-level path
for externally owned pools and registers native closure in existing explicit
LIFO cleanup. It does not close on registration failure; the caller retains the
pool and must await teardown. The facade-owned
[`owned_pool`](../batter/examples/owned_pool.rs) example composes `pool_in` with a finite
`Command`; the runnable [PostgreSQL lifecycle
example](../../examples/postgres-lifecycle/README.md) and the reference root use
`pool_in` inside protected service startup, and the reference retirement command
uses it on a finite `CommandScope` slot.

`SqlxFailure` retains the original SQLx cause with fixed Debug/Display.
`FailureClass` classifies native variants, never error strings, and grants no
replay permission. Trusted source-chain inspection can still disclose native
contents. SQLx logging and Rust panic-hook output remain application-owned.
Transaction commit uncertainty stays separate from operation interruption;
neither pool retirement nor an I/O error proves a write did not commit.

No database provisioning, repository abstraction or automatic replay is supplied.
Select TLS through native SQLx features in the consumer.

## Read-only schema and authority verification

The optional `verification` module provides a generic PostgreSQL 18 catalog
check through an owned lease from the actual serving pool:

```rust,no_run
use batter_sqlx::verification::{
    AllowedPrivilege, AuthorityPolicyBuilder, DatabasePolicy, DiscoveryDefaults, DiscoveryScope,
    Identifier, MigrationExpectation, RequiredPrivilege,
    MigrationPolicy, ObjectPrivilege, PublicGrant, PublicObject, QualifiedName,
    RelationPolicy, SchemaPolicy, VerificationPlan, verify,
};

# async fn check(pool: &sqlx::PgPool, context: &batter_core::operation::OperationContext) -> Result<(), Box<dyn std::error::Error>> {
let ledger = QualifiedName::new("app", "_sqlx_migrations")?;
let migration = MigrationPolicy::new(
    ledger.clone(),
    vec![MigrationExpectation::new(1, vec![0; 48])],
)?;
let authority = AuthorityPolicyBuilder {
    discovery: DiscoveryScope::Schemas(vec![Identifier::new("app")?]),
    defaults: DiscoveryDefaults {
        allow_row_type_public_usage: true,
        ..DiscoveryDefaults::default()
    },
    required_privileges: vec![
        RequiredPrivilege {
            object: PublicObject::Relation(ledger.clone()),
            privilege: ObjectPrivilege::Select,
        },
        RequiredPrivilege {
            object: PublicObject::Schema(Identifier::new("app")?),
            privilege: ObjectPrivilege::Usage,
        },
    ],
    relations: vec![RelationPolicy {
        relation: ledger,
        privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)],
        columns: Vec::new(),
        allow_owner: false,
        allow_row_type_public_usage: true,
    }],
    schemas: vec![SchemaPolicy {
        schema: batter_sqlx::verification::Identifier::new("app")?,
        privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)],
        allow_owner: false,
    }],
    database: DatabasePolicy {
        privileges: vec![
            AllowedPrivilege::new(ObjectPrivilege::Connect, false),
            AllowedPrivilege::new(ObjectPrivilege::Temporary, false),
        ],
        allow_owner: false,
    },
    public_grants: vec![
        PublicGrant {
            object: PublicObject::Database,
            privilege: AllowedPrivilege::new(ObjectPrivilege::Connect, false),
        },
        PublicGrant {
            object: PublicObject::Database,
            privilege: AllowedPrivilege::new(ObjectPrivilege::Temporary, false),
        },
    ],
    ..AuthorityPolicyBuilder::default()
}.build()?;
let plan = VerificationPlan::migrations(&migration).with_authority(&authority)?;
let report = verify(pool, context, plan).await?;
if !report.is_within_declared_policy() {
    // Inspect structured findings and choose the application's readiness path.
}
# Ok(()) }
```

`verify(pool, context, plan)` owns acquisition, transaction reset, inspection,
rollback and lease disposition under the supplied `OperationContext`. It clears
residual raw transaction state only on its newly acquired checkout. Separate
application transactions are never borrowed. It preserves serving identity and
session options, uses a trusted transaction-local search path for catalog SQL,
and recovers the authenticated login transaction-locally for the
excess-authority audit, and uses the entry current role for required privileges
and query visibility. Only acknowledged rollback and completed operation permit
pool return; failure or interruption retires the client lease. This does not
acknowledge server-session termination.

Before its first snapshot-taking query, the verifier locks the ledger and its
existing descendants in ACCESS SHARE mode. A ledger participating in inheritance
in the captured snapshot returns Incomplete/InheritedMigrationLedgers through
combined and migration-only verification. Supported standalone ledgers are read
with ONLY: a late attachment cannot inject rows through a newer planner view of
inheritance. Ordinary row writes remain possible; DDL conflicting with the held
locks waits until rollback. RLS-enabled ledgers are findings, and `row_security = off` requests an
error if a read would filter rows. This is not an RLS bypass. The verifier never
runs DDL, takes a migrator advisory lock, repairs grants or terminates a backend.
A snapshot describes its observed state, not future connections or grants.

Use `verify_migrations(pool, context, migration_policy)` for a separate schema
stage, including setup credentials, and `verify_authority(pool, context,
authority_policy)` for a serving-role stage. These share the same owned executor
and report only their inspected surfaces. Authority-only inspection can report a
missing required ledger SELECT privilege without trying to read that ledger.

`AuthorityPolicyBuilder::required_privileges` states what the current role must already
have through direct, PUBLIC or INHERIT grants. SET/ADMIN potential counts only
for the excess-authority audit. Schema USAGE and each needed relation, column or
routine privilege are independent requirements. These are catalog capabilities,
not proof of RLS-visible rows, function bodies or accepted parameter values.

`DiscoveryScope::Schemas` and `UserSchemas` inspect unlisted supported objects
within the chosen scope. Per-kind `DiscoveryDefaults` allow ordinary privileges
without allowing ownership or grant options implicitly. Exact object policies
replace role defaults; exact `PublicAllowance` entries replace PUBLIC defaults,
including an empty explicit deny. An exact PUBLIC relation declaration (either
`PublicGrant` or `PublicAllowance`) also suppresses PUBLIC column defaults for
that relation. Explicit column declarations remain exceptions. A permitted
whole-table privilege still supplies the corresponding column privilege; a
column deny cannot subtract a permitted whole-table grant. Required-policy
validation and discovery use this same precedence. Invoker and definer routine defaults are
separate. Keep application grant manifests as data and add exact exceptions
there; do not duplicate the generic ACL evaluator downstream.

Builder collections have two deliberate duplicate contracts. Set-like schema,
role, and required-surface collections, identical privilege atoms, and identical
legacy `PublicGrant` atoms normalize to one canonical entry. Keyed object
policies, column policies, exact `PublicAllowance` entries, and required
object/privilege pairs reject duplicates; repeated privilege atoms that disagree
about grant option also fail. Tables, views, materialized views, foreign tables,
and column parents are relation identities, while sequences are sequence
identities. One schema-qualified name cannot declare both kinds. If the database
later exposes the opposite kind for a valid declaration, verification retains
the ordinary missing-object or missing-privilege finding and evaluates the observed object with its own
discovery defaults. Catalog drift is not reclassified as invalid policy input.
The private catalog-evaluation policy is assembled during the same cooperative
catalog traversal: each selected schema, relation, column, type and routine
identity is checked once before its already-normalized defaults are appended.
It is not sent back through the external draft compiler for a global sort and
second structural-index pass.

For the common exact-role case, `ExactRoleManifest` owns that application data
in grouped Rust declarations and compiles it into the same low-level
`AuthorityPolicy`. `RequiredAndProvisioned` privileges also enter an inert,
normalized `GrantPlan`; `AllowedOnly` privileges, discovery defaults, ownership
allowances and grant-option ceilings never produce SQL. PUBLIC delivery is an
exact verification allowance, not a provisioning instruction. The pure renderer
requires validated role/database identifiers, quotes every identifier component,
uses `ON ROUTINE` for structural overload identities, and emits only role-targeted
`GRANT` statements. It rejects PostgreSQL's special PUBLIC/NONE spellings and
reserved `pg_` role namespace case-insensitively. PUBLIC delivery defaults to
deny, so applications retaining PostgreSQL's built-in PUBLIC defaults must opt
in for each exact declaration. A column group requires a relation group for the
same parent so relation ownership, row-type and PUBLIC policy are explicit. It
also rejects a column PUBLIC deny paired with the same parent-relation PUBLIC
allowance because a table privilege already reaches that column. It never
connects, creates roles, revokes authority, wraps a transaction or executes the
output. Applications still own exact object choices, role membership and
selection, transaction wrapping, any global PUBLIC revocation and explicit
operator execution.

```rust
use batter_sqlx::verification::{
    DeclarationPurpose, DiscoveryScope, ExactRoleManifest, Identifier,
    ObjectPrivilege, QualifiedName, RelationGrantGroup,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut manifest = ExactRoleManifest::new(
        Identifier::new("service")?,
        DiscoveryScope::Declared,
    )?;
    manifest.add_relations(
        RelationGrantGroup::new(
            [QualifiedName::new("service", "records")?],
            [ObjectPrivilege::Select],
            DeclarationPurpose::RequiredAndProvisioned,
        )?
        .allow_row_type_public_usage(true),
    )?;
    let compiled = manifest.compile()?;
    let sql = compiled
        .grant_plan()
        .render(&Identifier::new("service_reader")?, None)?;
    assert_eq!(
        sql,
        "GRANT SELECT ON TABLE \"service\".\"records\" TO \"service_reader\";\n"
    );
    Ok(())
}
```

PostgreSQL ordinary table row types grant PUBLIC USAGE by default. The example
opts into that observed default explicitly; omit the builder call only when the
application expects the verifier to require its revocation.

The supported authorization model is PostgreSQL 18 catalog ACLs, ownership,
listed role attributes/predefined capabilities, and bounded role reachability
for the selected object kinds. Ordinary persistent and unlogged objects are
included. Temporary namespaces and their objects are explicitly unsupported:
selecting `pg_temp`, `pg_temp_*` or `pg_toast_temp_*`, including through PUBLIC,
required privileges, discovery, a migration ledger or routine argument type,
returns `Incomplete` with `UnsupportedSurface::TemporaryNamespaces`. All three
entrypoints reject that inspection before ledger locking or ACL comparison;
no partial supported-surface claim or incorrect missing-privilege finding is
returned. `UserSchemas` retains its documented exclusion of system schemas.

Required and excess authority are evaluated from the same captured catalog
snapshot. PostgreSQL's native `has_*_privilege` inquiry functions can observe a
later concurrent grant through catalog caches even within REPEATABLE READ;
they are differential test references on stable fixtures, not replacements for
snapshot evaluation. Unsupported execution protocols (routine bodies, RLS-visible
rows, extensions and other listed surfaces) remain outside the verdict. New
coverage needs an explicit semantic rule and native differential evidence;
matching an object's catalog shape alone does not establish support.

Discovery retains dependent array and multirange identities, including when a
multirange's source range lives outside the selected schemas. Type loading
resolves and shares the source ACL separately; an exact policy on one alias
still applies to that alias. Finding object names quote schema, object and
column components individually, and declared routines use
`RoutineSignature::as_str()`. Security-definer configuration findings instead
append `#<oid>` to the quoted schema and routine name; that identity distinguishes
overloads within the captured snapshot and is not stable across object recreation.
Do not construct expected finding names with unquoted dot concatenation.

A caller can explicitly permit successful later migrations (`AllowSuccessful`
requires a nonempty required set and strictly later versions). The reader caps
ledger rows at 10,000 and materializes at most 1,024 checksum bytes per row plus
bounded mismatch bookkeeping. A larger ledger is a `MigrationLedgerLimit`
violation; an oversized checksum cannot match a validated expectation. Each
catalog query, including expanded ACL rows, accepts at most 10,000 rows; a
10,001st sentinel or parameter name over 1,024 bytes returns `CatalogCapacity`
instead of a partial report. Role, membership and explicit parameter ACL limits
are cluster-wide: unrelated catalog growth can reject verification for a small
policy too. These bound retained catalog inputs, not PostgreSQL
execution memory or non-yielding client work. Server timeouts remain in force.

Authority input is limited to 10,000 entries including nested columns,
arguments and allowances, and at most 32 entries per privilege list
(`PolicyError::AuthorityCapacity`). Captured evaluation uses one shared budget
and yields every 64 object/role visits. More than one million visits, 100,000
findings, or 16 MiB of finding payload (struct sizes and object/subject string
bytes) returns `EvaluationCapacity`; it never returns a partial policy pass.
Catalog-default expansion participates in those checkpoints and has no
post-traversal generated-policy canonicalization phase.
Ordinary object ACL evaluation considers actual reachable grantees, the owner
and active superusers instead of visiting every reachable role per object.
Parameter context defaults still require active-role evaluation. These are
cooperative work limits, not hard wall-clock, allocator or destruction bounds.
Manifest retention, raw grouped expansion and the generated low-level policy are
bounded separately. Generated exact PUBLIC overrides and required-privilege rows
count toward the policy's 10,000-entry limit, so fitting the retained-declaration
limit alone does not guarantee that compilation fits the generated-policy limit.

The owned checkout always receives an acknowledged `ROLLBACK` before its new
transaction. With an idle checkout PostgreSQL emits the expected warning
"there is no transaction in progress" on SQLx's `sqlx::postgres::notice` target.
SQLx 0.9's public transaction check sees managed depth, so it cannot safely
replace this reset for abandoned raw transactions. The verifier preserves the
native notice; it does not suppress other warnings or change the subscriber.

`VerificationReport::supported()` names the ACL and role surfaces evaluated for
the declared or discovered objects; `unsupported()` names bounded classes that are disclosed
but not inferred. Add `RequiredSurface` values to
`AuthorityPolicyBuilder::required_surfaces` when an application needs an unsupported
surface to make the report `Incomplete`; a compiled policy exposes the selection
through `AuthorityPolicy::required_surfaces()`. Merely allowing a routine ACL does not
request a proof of its SECURITY DEFINER body or stored settings. Extension-owned
objects still have their declared ACLs inspected; extension membership and
upgrade semantics remain unsupported.

The runnable `verification` example requires an explicit migration boundary:
set `BATTER_VERIFY_REQUIRED_VERSION` and
`BATTER_VERIFY_REQUIRED_CHECKSUM_HEX` (even-length hexadecimal bytes). It builds
a protected compiled-role request and exact SQLx 0.9 ledger manifest.
The retired `BATTER_VERIFY_ALLOW_LATER` flag is rejected rather than silently
weakening or misrepresenting that one-entry exact manifest; complete
multi-migration example input remains tracked separately.
Set `BATTER_VERIFY_DELIBERATE_VIOLATION=1` to remove the ledger SELECT allowance
and exercise the non-success path against the same database. Owner and
superuser allowances are opt-in environment flags, not hidden defaults.
Connection establishment and the complete verification transaction share one
30-second `OperationContext`; interruption retires the verifier-owned lease.

### Protected SQLx, schema, and ownership requests

`SqlxMigrationManifest` opts into the SQLx 0.9 ledger contract: exactly six
non-dropped, non-null `pg_catalog` columns and one primary key containing only
`version`. `Exact` requires the complete successful version/checksum set.
`InstalledSubset` permits an absent ledger or any set-membership subset of the
expected successful rows; it is intentionally not prefix matching. Both modes
retain the 10,000-row and 1,024-byte checksum bounds, ACCESS SHARE locking,
inheritance rejection, `ONLY` reads, and RLS detection. A relation visible in
the captured snapshot after an absent-name lock attempt, or one that replaces a
locked relation after its schema is renamed, is incomplete rather than inspected
without a lock. A later commit remains outside that historical snapshot. Batter proves the
relation visible by the requested name carries its backend's pre-snapshot lock;
it does not choose migrations or run a migrator.

`SchemaInspectionPolicy` independently selects explicit schemas and requires
exactly one stored `search_path=...` entry on every SECURITY DEFINER routine in
them, including ungranted and trigger routines. `canonical` selects the exact
`search_path=pg_catalog, pg_temp` representation. Other stored settings are
accepted subject to the same fail-closed row, entry-size, and aggregate retained-
byte bounds. Every selected schema must exist; an absent name is a structured
missing-object violation, not an empty successful scope. Equivalent whitespace,
quoting, order, or extra path entries are not normalized. This verifies catalog
configuration, not routine bodies, trigger behavior, or future DDL.

Call `ExactRoleManifest::deny_current_database_ownership(true)` before compiling
to reject current-database ownership held by the authenticated login or a role
reachable through the verifier's INHERIT/SET/ADMIN capability graph. The check
uses the current database owner and database-local `pg_shdepend` owner records,
retains class/object/subobject addresses, excludes structurally valid non-owner
role dependencies and unreachable well-formed owners before the bound, and
returns incomplete for unknown, malformed, impossible, or missing-role record
forms or a bounded relevant catalog overflow. Database ownership, membership
endpoints, owner records, and the required `pg_database_owner` role must all
resolve in the same bounded role snapshot. Because
PostgreSQL omits ordinary dependency records for pinned roles,
a capability-reachable superuser or predefined role other than the implicit
`pg_database_owner` also makes this ownership-specific conclusion incomplete.
PostgreSQL 18 forbids stored memberships into or out of `pg_database_owner`;
if such a row is nevertheless observed, the ownership catalog is incomplete
rather than being interpreted as the implicit current-owner edge.
It does not inspect other databases or grant application-specific ownership
exceptions.

`verify_sqlx_migrations`, `verify_exact_role`, and non-empty `VerificationPlan`
composition reuse the same executor, lease, snapshot, rollback, and retirement
contract as the focused entrypoints. A plan starts from an authority, migration,
SQLx manifest, exact role, or schema-inspection constructor and rejects a second
component of the same semantic kind. `AuthorityPolicyBuilder` is mutable
configuration input; only a successfully built immutable `AuthorityPolicy` is
executable. Migration policies and the other plan components likewise validate
at construction, before an operation future or database lease exists. The
plan also rejects an authority/migration combination when authority declares the
ledger's schema-qualified name as a sequence: PostgreSQL requires the ledger to
be a relation, and those object kinds share one schema namespace. The
compiled role's internal authority policy is deliberately not exposed, so its
ownership safeguard cannot be discarded accidentally.

This is additive inspection machinery, not a replacement for application
policy. Profile-specific objects, migrations, privileges, provisioning, routine
semantics, and durable-history protocols remain application-owned.
Declaring a relation can explicitly allow its owning composite row type's
default PUBLIC `USAGE` with `RelationPolicy::allow_row_type_public_usage`; the
field is deliberately visible so an unexpected row-type ACL is not hidden.
Direct role grants on that row type and standalone user-defined types require
an explicit `TypePolicy`.
Migration ledgers with row-level security enabled are rejected because a
filtered `SELECT` cannot prove complete migration history. Declared custom
dot-qualified parameters that are absent from `pg_settings` are conservatively
treated as SETtable by reachable execution identities; explicit cluster-wide
parameter ACLs remain independently inspected. `ParameterName` preserves the
complete custom name instead of applying the 63-byte catalog-identifier limit,
normalizes ASCII comparison keys across both parameter catalogs. Requested hidden
parameter metadata produces `ParameterUnobservable` and `Incomplete`, preserving
known findings. Under captured full current visibility, a bounded boolean-only
existence probe distinguishes missing built-ins from existing settings omitted by
pg_settings; it never transfers values or invents their missing context.
An unknown custom placeholder supplies conservative potential authority only.
Requiring its privilege produces Incomplete until its definition is observable;
syntax alone cannot distinguish an assignable placeholder, a reserved prefix or
a hidden extension parameter. The verifier never executes SET to test a value.
Reachable ADMIN authority includes both ordinary grant reachability and the
PostgreSQL 18 path where an active CREATEROLE identity can manage a non-superuser
target via ADMIN across membership edges that grant neither SET nor INHERIT.
Each usable target requires its own `RolePolicy::allowed_admin_roles` entry.
Even an allowed ordinary target is audited as authority the login can grant back
to itself with SET and INHERIT. PostgreSQL reserves grants and revocations of
membership in superuser roles to an already active superuser, so an ADMIN-only
edge to such a target does not create usable superuser authority.
Routine policies use `RoutineSignature::new` with canonical `RoutineType`
components from `pg_type`; aliases do not resolve to their underlying catalog
identity, and named argument declarations or arbitrary SQL type expressions are
not accepted. PostgreSQL stores every source-level array dimensionality under
one true-array type OID, so `RoutineType::array` intentionally renders one
`[]`. Array identity requires both the catalog array subscript handler and the
element type's reverse `typarray` link, keeping subscriptable vector types
distinct.

Run `cargo test -p batter-sqlx --features test-support --locked` for offline
contracts. Live cases are ignored in ordinary all-feature checks. Configure
`DATABASE_URL` for an external disposable database and
`BATTER_SQLX_AUTH_ACCEPT_URL` for a known-good password-authenticated endpoint,
and `BATTER_SQLX_ADMIN_URL` for a PostgreSQL superuser connection on a dedicated
disposable cluster, then run `bash scripts/test_sqlx_live.sh`; missing
prerequisites fail. Roles and parameter ACLs are cluster-wide. Custom parameter
names are unique per fixture, while process death can still leave test-owned
cluster residue for external cleanup. The
authentication case first completes a query with those parsed connection options,
then changes only their password and requires exact PostgreSQL SQLSTATE `28P01`.
A trust endpoint, missing role or connection refusal is not equivalent.
The runner verifies
an exact 61-case inventory across the eleven PostgreSQL lease/read-only
verification cases, fourteen pool ownership cases and thirty-six authority-
and-protected-verification cases,
executes each target serially, and bounds every child process.
Cases can use up to six simultaneous server sessions, including retired sessions
and session advisory locks. Provisioning stays external.

## Optional fixture support

The `test-support` feature exposes `test_support::FixtureSuite`, declared native
pool capacity, ordered template inputs and bounded lock observation. Use
`FixtureSuite::start` to retain native acquisition producers before cancelling
waiters is possible. The driver joins producers after body exit, then closes
pools, cleans leases and drains deferred cleanup. Reports preserve acquisition
failures even when the body handles them. Template initializers must be Send and
static; panic reaches upstream's awaited abort path. Their own pools/operations
still require explicit close/join.

Harness clones share native admission; other owners must release leases for
waiting acquisitions to progress. The caller owns server shutdown after all runs
finish. Owned reports and borrowed wait views are must-use; driver join alone
is not fixture success. `FixtureRun::wait` can be cancelled and resumed; dropping it loses report
observation, and runtime death has no completion guarantee. `PoolAcquire` means
cleanup is pending; low-level `Connect` reports attempted cleanup. The low-level
fixture owner is must-use and still requires explicit finish. Provisioning and
caching stay upstream; migrations and row builders stay consumer-owned.

Owned runs can opt into `with_session_observer(SessionObserver)` before acquisition.
After tracked pool close, an independently budgeted admin pool must observe no
sessions for that database. Timeout/error retains the lease and parks its cleanup
until `SessionObserver::retry_with`; independent databases continue. Every failed
attempt survives recovery in `DatabaseCleanup::observation_failures`. Stop new
connection producers before observation; absence does not prevent future sessions.
`wait_for` returns pending without cancelling work; `cleanup_progress` reports
historical phases and errors. Pending is not clean reuse. Dropping the retry
control can strand a parked driver; runtime loss can trigger destructive native
lease Drop. The low-level manual APIs do not use this optional observer.

Pool-acquisition errors share their concrete cause through `Arc<sqlx::Error>` in
`FixtureError::PoolAcquire` and `DatabaseCleanup::pool_failures`; handling an error
in the body cannot make the report successful. Inspect all report branches after
finish, including separate consuming-cleanup and deferred-drain causes.

Session retries broadcast to all users of that control. Unread requests coalesce;
a request during an active attempt is consumed after failure without cancelling
that attempt. The full attempt budget includes pool acquisition; timeout means
no absence witness, not proof of surviving sessions.

Parked leases retain the upstream harness's admission permits. Exhausting a
shared harness with unresolved observations can block other runs' acquisitions
indefinitely. Recover the parked leases before awaiting more capacity; keeping
databases retained does not guarantee progress for another run.

Retry replacement never closes the previous pool. An observer mistakenly inside a
disposable database must be explicitly closed before a corrected retry. Same-server
identity and normal native catalog visibility remain caller preconditions; database
name presence is not a cluster identity check. Redacted report summaries separate
failed consuming database cleanup from retained pool and observation failures.


### Native migration execution

`PgLease::migrate(&Migrator)` consumes the lease while retaining its physical
connection internally. The application selects and configures its native SQLx
bundle; SQLx owns migration locking, history and checksum validation. A failed or
interrupted migration retires the connection, including a native validation error
that leaves its advisory lock held. Success uses the ordinary idle-state proof
before reuse. Await the method inside the enclosing operation budget.

The redacted `SqlxFailure` retains the original `MigrateError` inside
`sqlx::Error::Migrate`; trusted diagnostics may inspect it. There is no raw
connection callback, second migrator implementation or automatic migration on
pool construction. No local result proves server-session termination.

### Foundation dependency direction

No SQLx feature depends on Runledger. `run_atomic`, `PgScopedSql` and
`PgReadOnlySnapshot` are generic building blocks for persistence libraries.
Domain adapters depend on this package; it never imports their domain types.
