# ADR-013: Managed database objects after migrations

## Status

Accepted, 2026-10-15. Owning Bead: `batter-54d0`. The repository owner approved
the design direction before implementation; this record fixes the API shape
under [ADR-010](010-agent-only-consumption.md).

## Context

The main downstream consumer keeps every PostgreSQL routine body inside
forward-only SQLx migrations. A survey of its real schema, with all 263
migrations applied to a disposable PostgreSQL 18 database, recorded 67,482
migration lines, 941 routine definitions for 576 distinct routines and 25,112
lines of superseded copies; one routine has 19 versions, and 160 of 231
routine-bearing migrations restate at least one existing routine. Reviewers see
only a whole new body. Two branches that restate one object race silently: the
later migration wins, which on 2026-10-05 removed a value from a CHECK
vocabulary and cost two fix commits and a permanent pinned test. The cause is
structural: a routine's editable unit is a copy inside an append-only log.

The same survey established the PostgreSQL facts this design depends on. The
553 routines in `public` (487 PL/pgSQL, 66 SQL) dumped with
`pg_get_functiondef` and re-applied in one transaction left oid, owner, ACL and
body identical. None uses `BEGIN ATOMIC`, so application order among routines
does not matter for `CREATE OR REPLACE`. 77 routines are trigger functions and
131 of 182 historical trigger bindings bind a function created in the same
migration, so triggers must be managed with their routines and bound after
them. 49 historical rewrites changed a signature or return type. The 66
SQL-language bodies are validated against the catalog at creation, so the phase
must follow the migrations. The consumer revokes `EXECUTE` from `PUBLIC` on its
routines (224 explicit revokes) and transfers ownership with its own owner
policies after migrations. These survey figures are the consumer's evidence,
not a measurement in this repository.

Batter already executes a consumer-selected migrator under its lease
(`PgLease::migrate`), verifies the SQLx ledger against an expected set
(`SqlxMigrationManifest`) and inspects `pg_proc` read-only. Ordering objects
after migrations, pairing apply with verification, atomicity and a
non-omittable `PUBLIC` policy are exactly the obligations ADR-010 says the
library must own rather than leave as repeated caller instructions.

## Decision

Batter applies and verifies a consumer-declared set of managed routines and
triggers. The source of each object, ownership transfer, grants beyond the
declared `PUBLIC` `EXECUTE` policy, migration selection and provisioning stay
downstream. Migrations keep tables, columns, constraints, policies and data
changes. The capability lives in `batter-sqlx` beside the lease and verifier,
is not feature-gated (like `verification` and `PgLease`), and is orthogonal to
the SQLx ledger: Batter never reads or writes migration history for it.

### Shape

- `managed::ManagedObjectManifest::new(ledger, public_execute, objects,
  retirements)` is pure and bounded: at most 10,000 entries plus retirements,
  1 MiB per source and 64 MiB in total. Entries are
  `ManagedObject::{function, procedure, trigger}`; retirements are
  `Retirement::{function, procedure, trigger}`. Routine identities reuse the
  structural `RoutineSignature`, so an identity names exact catalog types and
  cannot be redirected by aliases or a search path. A trigger identity is its
  table and name, rendered `"name" ON "schema"."table"`. A deterministic
  SHA-256 `ManagedObjectDigest` covers the policy, every entry's kind, identity
  and source checksum and every retirement, in sorted identity order, but not
  the ledger name, so a build can embed it in release evidence.
- `PgLease::migrate_and_apply(migrator, &manifest)` is the only apply path.
- `VerificationPlan::managed_objects(&manifest)`, `with_managed_objects` and
  `verify_managed_objects` add a fourth, read-only verification axis.

### Ordering: one combined entry point

The brief allowed either a witness returned by `migrate` and required by
apply, or one method doing both. The combined method was chosen. A witness
would need a second consuming lease state: the existing `migrate` returns `()`
and changing it would be a hard cut, so a parallel `migrate_for_objects`
returning a witness lease would add a public type whose only legal operation
is the second half of one call. The combined method has the smallest surface,
keeps the connection private across both phases, and lets one session advisory
lock, keyed by the ledger, span migrations and objects so a concurrent applier
of the same ledger cannot land its objects between this connection's
migrations and objects. A compile-fail rustdoc rejects an apply-only call and a
second one rejects reusing a lease consumed by `migrate`.

### Apply

One transaction creates the ledger when absent, locks it `ACCESS EXCLUSIVE`
and requires its exact shape (`identity text PRIMARY KEY`, `kind text`,
`source_checksum bytea`, `rendered_checksum bytea`, `applied_at timestamptz`,
all `NOT NULL`, no row security, no inheritance). Any other relation of that
name, such as a SQLx ledger, is refused before anything is written. Then:
retired triggers that still exist are dropped, then retired routines that still
exist (never `CASCADE`); entries are executed, routines before triggers; each
executed statement must have written exactly its declared identity and kind in
this transaction (its catalog row's `xmin` is the current transaction id); the
`PUBLIC` policy is applied; and the ledger is rewritten: undeclared rows are
deleted, executed entries record their source checksum, the SHA-256 of their
rendered definition and `applied_at`, and a `manifest` row records the set
digest and a digest of all rendered hashes. `check_function_bodies` is forced
on for the transaction.

Deviation from the brief, recorded in the Bead: the brief listed
`CREATE OR REPLACE` for every entry and a drop for every retirement on every
apply. Probed on PostgreSQL 18.6, `CREATE OR REPLACE TRIGGER` takes
`SHARE ROW EXCLUSIVE` on its table even when the definition is unchanged, and
holds it until commit, so every startup of every replica would block writes to
every managed trigger's table for the whole transaction and queue later writers
behind a waiting apply. Apply therefore executes an entry only when its ledger
row is absent, its kind or source checksum changed, or its live definition no
longer renders to the recorded hash. It drops a retirement only when the object
exists, which keeps a completed retirement silent (`DROP ... IF EXISTS` on an
absent object holds no table lock but emits a notice on every apply) and makes
the reported count exact; dropping an existing trigger takes
`ACCESS EXCLUSIVE` on its table. Drift is still repaired, because a drifted object
no longer matches its recorded hash; an unchanged, undrifted set executes no
DDL. The cost is that an unchanged SQL-language body is not re-validated
against a schema a later migration changed, which matches the behaviour of
routines kept in migrations today.

Definitions are rendered with `pg_get_functiondef` and `pg_get_triggerdef`
under fixed `search_path`, `DateStyle`, `IntervalStyle`, `TimeZone`,
`extra_float_digits`, `bytea_output`, `standard_conforming_strings` and
`quote_all_identifiers` values in both apply and verification. Executed live,
the rendered text qualifies type and function names relative to the session
search path, so without the fixed settings a role default would turn an
unchanged definition into drift. Source statements themselves run under the
applying session's own settings.

### Verification

The verifier takes `ACCESS SHARE` on the ledger before its repeatable-read
snapshot, so it waits for an in-flight apply and blocks a new one until it rolls
back; catalog rendering reads PostgreSQL's catalog caches rather than the
snapshot, and the lock keeps a library apply from committing between the
snapshot's ledger rows and that rendering. It then reports: a missing ledger
(`MissingManagedObjectLedger`), a misshapen one (`ManagedObjectLedgerShape`,
`LedgerRowSecurity`) or one that appeared or was replaced concurrently or uses
inheritance (`UnprotectedManagedObjectLedger`, Incomplete); the set row
(`ManagedObjectSetDigest`); missing, unexpected and kind- or
checksum-mismatched rows; live definitions that are absent, of another kind or
render to another hash (`ManagedObjectDrift`); retired objects that still exist
(`RetiredManagedObjectPresent`); and, under `PublicExecute::Revoke`, managed
routines `PUBLIC` can execute (`ManagedObjectPublicExecute`). Catalog identities
are resolved by joins, not `regprocedure` input, so the serving role needs only
schema `USAGE` and ledger `SELECT`. Composition rejects a second manifest, a
managed ledger equal to the migration ledger and an authority policy that
declares the managed ledger as a sequence, in either order.

## ADR-010 invalid-state review

Executed for every new public item before delivery.

- `ManagedObjectManifest`. (1) Construction rejects every input pure
  information can reject: an empty set, the SQLx-reserved ledger name,
  temporary namespaces for the ledger, entries, retirements and argument
  types, duplicate entries (functions and procedures share identities),
  duplicate retirements, a retirement of a declared identity, and exceeded
  bounds; iteration stops at the capacity boundary. (2) Callers need not
  remember ordering or pairing: entries are sorted, apply orders phases, and
  the same value drives apply and verification. (3) The `PUBLIC` policy is a
  required exhaustive argument with no default, so leaving PostgreSQL's
  default `PUBLIC` `EXECUTE` on new routines must be written as
  `PublicExecute::Unmanaged` (a compile-fail rustdoc rejects omitting it).
  (5) Removing an entry without a retirement only stops managing the object:
  it stays in the database, because inference of drops is unacceptable; a
  consumer handing an object back to migrations is legitimate.
- `ManagedObject`. (1) Each constructor validates its source lexically: one
  statement (quotes, nested comments, dollar quotes and `psql`'s `BEGIN ATOMIC`
  rule), `CREATE OR REPLACE` of the declared kind, and a schema-qualified header
  naming the declared identity; `CREATE CONSTRAINT TRIGGER`, which PostgreSQL
  cannot replace, is rejected. Argument lists are not parsed: type spellings
  such as `int` and `integer` cannot be compared purely, so apply checks the
  written catalog row instead and a mismatch never commits. (3) The value is
  opaque; its checksum is derived, not supplied.
- `ManagedObjectKind`, `PublicExecute`. Exhaustive enums without defaults.
- `Retirement`. Constructors are infallible because their identities are
  already validated structural types; temporary namespaces and overlaps are
  rejected when a manifest is built, the only place a retirement can be used.
  The kind is explicit, so a function retirement cannot silently drop a
  procedure.
- `ManagedObjectDigest`. Read-only; constructible only from a manifest.
- `ManagedObjectError`. Exhaustive; overlap and duplicate variants carry the
  canonical identity, which names an object, not a secret.
- `PgLease::migrate_and_apply`. (2) Ordering, same-connection pairing,
  atomicity, ledger locking, the `PUBLIC` policy and the identity check are
  library-owned; there is no apply-only method or witness to misuse. (4) It is
  the only ordinary path. The method boxes its operation once, as other
  canonical lease entry points do.
- `AppliedManagedObjects`. `#[must_use]` evidence with read-only accessors.
- `ManagedApplyError`. Redacted `Display`/`Debug`; native causes through
  `source` and `native`. Only `CommitUnconfirmed` represents an uncertain
  outcome; every other variant was raised before `COMMIT` was sent.
- `VerificationPlan::managed_objects`, `with_managed_objects`,
  `verify_managed_objects`. (1) Duplicate selection, ledger conflicts and
  sequence/relation conflicts are composition errors before execution.
- New `FindingKind`, `SupportedSurface`, `UnsupportedSurface` and `PlanError`
  variants. These enums are intentionally exhaustive so consumers review new
  outcomes. Adding variants breaks exhaustive matches; there is no shim that
  can make an old match cover a new variant, and mapping managed-object
  outcomes onto migration variants would misreport them. The changelog carries
  a `Breaking:` entry with before/after matches. This is the one place the
  delivery is not purely additive.

Remaining obligations are explicit and not claimed as local guarantees. (5)
Which release's manifest is installed is last-writer-wins: an older release
that applies after a newer one reverts objects, unless the newer release added
a migration and the older migrator does not ignore missing versions, in which
case it fails with `VersionMissing` before apply; verification on the newer
replicas then reports the digest mismatch. A routine whose return type, `OUT` parameters or input parameter
names change, or whose defaults are removed, cannot be replaced in place: a
versioned migration drops it, and the next apply creates it. Because
retirements precede creation, a trigger cannot be rebound away from a routine
retired in the same manifest; rebinding and retiring take two releases, or a
migration that drops the trigger. Rendered hashes depend on PostgreSQL's deparser: a server upgrade that
changes rendering reports drift until the next apply re-executes and re-records
those entries. Ownership of newly created routines is the applying login's;
replacing an object requires ownership or membership in its owner role, and
transfer stays with the application. Lexical validation does not prove a body
correct; PostgreSQL validates SQL-language bodies and only parses PL/pgSQL. A
retired session still waiting on a lock remains on the server until the lock is
granted, as with `migrate`.

## Evidence

Offline unit tests cover every manifest rejection, the lexer, the digest, plan
composition and the ledger and catalog comparisons. Eight PostgreSQL 18.6 live
cases in `crates/batter-sqlx/tests/managed_objects_live.rs` cover first apply
after migrations, an idempotent re-apply with an identical catalog including
`xmin`, retirement with a dependent trigger, a blocked retirement that changes
nothing, both `PUBLIC` policies, out-of-band drift and its repair, a missing
ledger finding, undeclared overloads (absent and pre-existing), an invalid SQL
body and a foreign ledger. Mutation controls that re-executed every entry, or
dropped the `xmin` check, failed the suite. The PostgreSQL lifecycle example is
the consumer contract and has its own live case. Fresh-agent implementation and
modification evaluations are proposed and unexecuted.
