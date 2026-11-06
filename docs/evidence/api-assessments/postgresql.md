# API assessments: Managed objects, grants, fixtures and reconciliation

[ADR-010 policy](../../adr/010-agent-only-consumption.md) · [Assessment index](README.md)

These records apply ADR-010 to specific changes. They retain the evidence
and limitations recorded at delivery; current contracts remain in rustdoc
and [guarantees](../../guarantees.md).

## Native online-index repair assessment (`batter-fnj8.10`)

Catalog identity checks alone did not protect a later name-based DROP: a real
PostgreSQL 18.6 probe replaced the inspected INVALID index with a foreign UNIQUE
index, which repair deleted. Earlier matcher/progress fixes addressed observations,
not this execution gap. Native Runledger now owns a bounded metadata transaction
that locks the actual index, validates identity and definition under index/table
locks, and removes it before commit. Concurrent creation begins afterward. Busy
tables refuse immediate metadata locking instead of queuing a blocking DROP.
A later review (`batter-fnj8.11`) found that one refused attempt left a live
queue unrepairable, so native Runledger now retries the whole rolled-back
transaction within a fixed five-second budget, each attempt still `NOWAIT`
with its own identity checks; no committed rename or caller retry protocol was
added. A table never idle for an instant still needs an operator's
`DROP INDEX CONCURRENTLY`, documented as protected-path recovery.
Callers acquire no extra lock and coordinate no new completion phase.

Rollback failures retain both causes; commit uncertainty prevents subsequent
creation. These opaque observations confer no replay authority or proof of remote
termination. External tooling remains outside library execution: preparation
refuses occupied non-adoptable names, but cannot prove later arbitrary DDL or
approve staged rollout policy. New diagnostics are additive to non-exhaustive
enums; old public call shapes remain. `OnlineIndexRollback` and
`OnlineIndexRepairCommitUnconfirmed` are public `SchemaCompatibilityError`
variants that the library produces only as the cause inside
`OnlineIndexExecution`, never at top level. This representable-but-unproduced
top-level state is an accepted trade-off: the execution cause is itself a
`SchemaCompatibilityError`, so a separate cause enum would duplicate every
other variant it must also carry, and their rustdoc names the nesting.
Non-exhaustive matching already requires a wildcard arm, so callers handle no
extra state. A dedicated execution-cause type remains a possible follow-up. Public-API failure regressions cover the
swap and lifecycle boundaries; fresh-agent usability tasks remain unexecuted.

## Managed application SQL assessment (`batter-aouh`)

Publishing a managed caller and granting its new private dependency in separate
transactions is an invalid ordering when both must become available together.
`migrate_and_apply_with` makes that composition one consuming lease operation.
The callback cannot extract/replace the connection, commit through a typed
handle, or reuse the consumed lease. Native savepoint/XID checks reject raw
transaction escape; explicit COMMIT effects remain remote uncertainty, never
an invented rollback. Refusal and continuity failure remain independently
inspectable without exposing either in automatic formatting.

Ownership and exact grant decisions are application policy. Managed definitions,
ledger evidence and the selected PUBLIC policy are library invariants and are
checked again after callback and deferred work, before COMMIT. No optional
caller validation step or paired finalize call exists. Successful application
work still retires session state. The existing no-work method remains compatible
for consumers without atomic application grants. Live failure scenarios exercise
these boundaries; no fresh-agent usability evaluation is claimed.

## Native grant fragment assessment (`batter-qjxi`)

Before this change, an application that wanted least-authority roles for the
native Runledger and Runlimit stores had to hand-maintain their relation, column
and privilege lists beside its own declarations. Nothing in the API could reject a
stale or over-broad list, and two audited consumer shapes had already drifted
apart. `GrantFragment` moves that knowledge to the library that owns the SQL.

Construction answers the first two questions. `grant_fragment` takes a non-empty
operation selection and rejects an empty one before any declaration exists, so a
selection that could never authorize work is not representable as a compiled
value. A fragment's column group requires that fragment to already declare the
parent relation, so an application never attaches a parent declaration for a
native relation and discovery defaults never silently choose that relation's
ownership, row-type or PUBLIC behavior. Relation and column names are validated
inside `FragmentObjectPolicy` and always qualified by its schema, so a fragment
cannot reach an object outside the schema the application named.

Partial state is unrepresentable rather than documented. Every fragment builder
and `ExactRoleManifest::with_fragment` consume their value and return
`Result<Self, _>`, so a rejected declaration or a declaration-bound overflow
yields no partially extended fragment and no manifest to compile. There is no
`&mut` append that could leave an executable partial grant set, and no second
compiler or renderer: a fragment reaches the one existing
`ExactRoleManifest::compile` and `GrantPlan::render`, which already normalize
duplicate and reordered declarations and reject contradictory purposes, PUBLIC
choices, row-type options and object/privilege pairs.

Authority is named by behavior, not by role. `RunledgerOperation` and
`RunlimitOperation` are non-exhaustive enums of operation groups, with no
administrator preset and no application role name, so a caller cannot ask for
"all privileges" and cannot accidentally select the privileged full-schema
snapshot, durable promotion, catalog synchronization or scheduled dispatch while
asking for intent submission. Every declaration `FragmentObjectPolicy` emits is
required and provisioned, never permits a grant option, and derives its PUBLIC
delivery from the exact privileges the application named, so a native producer
cannot smuggle in an allowance the application did not ask for.

That narrower contract is the policy type's, not the fragment type's. The raw
`GrantFragment` builders accept any declaration the manifest accepts, including an
allowed-only ceiling or a permitted grant option, because a library may
legitimately publish one. Enforcing the stricter shape in `GrantFragment` would
remove a legitimate use without removing a caller-memory obligation, since the
declaration a library publishes is already visible in its own source and in the
frozen privilege oracles. The boundary is therefore documented rather than
type-enforced: a library that bypasses `FragmentObjectPolicy` keeps whatever
purpose and grant option it declares, and the native producers do not bypass it.

What remains with the application is explicit and deliberately not implied to be
stronger. A fragment cannot carry the current-database declaration, discovery
scope or defaults, role-attribute ceilings or the current-database ownership
guard, and `FragmentObjectPolicy` makes the application state its PUBLIC
delivery, row-type and ownership choices rather than inheriting a library
default. Which operations a deployment runs is application policy; selecting one
it never invokes provisions authority it does not need, and no local type can
detect that. The requirements describe this source version's statements: they are
not evidence that the schema is installed, that a remote effect occurred, or that
an application's authorization model is correct, and rendering grants cannot
remove privileges a provisioned role already holds. Required `UPDATE` and
`DELETE` privileges, including row-locking `UPDATE` columns, remain arbitrary SQL
authority over those rows, so no payload-secrecy, tenant-isolation, immutable
identity or history-protection guarantee follows from a narrower column list.
Installed trigger ownership, routine bodies and application-added trigger
dependencies stay explicit prerequisites. Fresh agent implementation and
modification evaluations remain proposed and unexecuted for this API.

## Maintained Runlimit storage assessment (`batter-z4m9`)

The same read-only two-consumer survey that motivated periodic components found
native GCRA storage cleanup omitted entirely in one consumer and hand-written as a
supervised future in the other, which had also deferred work because native
fixed-window cleanup read `ctid` and so needed relation-level `SELECT` for its
maintenance login. Expired counters keep their capacity slots until deleted, so
the omission eventually turns into storage-capacity denials. A PostgreSQL limiter
that a service serves from without registered cleanup was therefore reachable
from the canonical path, and only documentation said otherwise.

`maintenance::register_in` is now the only constructor of `MaintainedLimiter`,
the value a service serves through `Quota::new` or the HTTP assembly, and it
registers cleanup through the existing periodic authority before returning it.
Holding the canonical value is therefore evidence that cleanup was registered
for that same native limiter. Maintenance runs on that limiter's own pool, login
and configuration rather than a separately supplied pool, so a maintenance pool
naming different storage than the serving pool, which no local check could
detect, is not expressible; for either store, combining the cleanup selection
with admission adds only `DELETE` and `SELECT (capacity_shard)` on the counter
relation. A zero row bound, which the native call treats as "do nothing", is
unrepresentable through `NonZeroU32`. The per-run budget is derived from the
native acquisition and operation timeouts instead of being chosen, so the
ordinary running budget that preserves native failure classification is
library-owned rather than a caller instruction, and an executed negative
control shows what it prevents. An earlier initialization allowance or shutdown
boundary can still interrupt classification; those existing periodic outcomes
remain authoritative. Maintenance
failures are always recoverable, so a transient database failure cannot drain
the process, while `after_first_success` lets an application make a missing grant
or schema fail startup.

What remains an obligation is stated rather than designed away. The native
limiters stay reachable through `batter::runlimit::postgres`, because native
reachability and type identity are a recorded decision (`batter-0jsf`) and
migration, finite commands and caller-owned schedules legitimately use them; they
are documented as the unmaintained low-level form. `Quota::new` still accepts any
`Limiter`: restricting it would be a hard cut on the memory and application-owned
backends and cannot be expressed for one native type without negative reasoning,
so the protection is that the routing table, module documentation and examples
lead to `register_in`. Grants, installed schema and keeping pace with expiry are
remote effects: registration proves none of them, and the startup policy only
detects the first two. Fresh agent implementation and modification evaluations
remain proposed and unexecuted for this API.

## Managed database objects assessment (`batter-54d0`)

A downstream consumer restated whole routine bodies in append-only migrations;
parallel branches restating one object raced silently, and keeping objects
after migrations, pairing apply with verification and revoking `PUBLIC`
`EXECUTE` were caller-memory obligations. The canonical path is now
`PgLease::migrate_and_apply`, the only apply entry point, so applying objects
before or without the application's migrator on the same lease is not
expressible (compile-fail rustdocs), and the one `ManagedObjectManifest` value
drives both apply and `VerificationPlan::with_managed_objects`. The `PUBLIC`
policy is a required exhaustive argument, retirements are explicit, and every
executed statement must write exactly its declared identity. The per-item
review and the obligations at that time (unordered releases, migration-only
return-type changes, two-release trigger rebinding and ownership transfer) are
in [ADR-013](../../adr/013-managed-database-objects.md). Its 2026-11-05 release-ordering
amendment (`batter-ofom`) now enforces revisions and matching legacy adoption;
application release numbering and administrative SQL remain outside that proof.
Fresh-agent usability evaluation is proposed and unexecuted.

2026-10-16 (`batter-n2s1`): constraint triggers had stayed in migrations,
because PostgreSQL cannot replace them, which left the consumer a written
ordering rule (ship a new constraint trigger a release after its function)
that protected only databases deploying every release. That instruction was
the design-debt signal above. `ManagedObject::constraint_trigger` now makes
the kind managed: apply owns the drop, its order after routines and the
transaction that installs the trigger with the body it binds, so the rule is
no longer needed. The review is in ADR-013.

## Fixture login pool assessment (`batter-7r3.8`)

A consumer needed fixture-managed pools that authenticate as a real restricted
login; role switching on an owner connection cannot reproduce `session_user` or
`system_user`. A proposed connect-options closure was rejected before
implementation. It would let a declaration retarget the host, socket or database,
caught only after a lease existed, and a closure that set only the username
would still send the owner's URL, `PGPASSWORD` or passfile password, a paired
call SQLx cannot undo because its setters cannot clear a password or socket.
`PlannedPool::password_login` takes exactly a username and a `SecretString`
password and replaces both on the lease's own native options, so neither
retargeting, half-replaced credentials nor a swapped password (which PostgreSQL
would echo into authentication errors) can be written. Silent identity rebinding, a caller-supplied full
options template and producer-side construction were also weighed and rejected:
the first two discard caller intent or the lease's TLS settings, and the third
would move delivered-lease failures into the harness error channel.

Pure construction cannot see the lease URL, so one rejection stays at runtime:
the harness keeps the admin URL's query, and SQLx lets `dbname` override the
path. Each lease's URL must select `lease.database_name()`; otherwise
`LeaseDatabaseMismatch` is retained like any pool failure, before any pool
exists; the `fixture_lease_database_mismatch_retained` live probe proves this on
both the owned and low-level paths. It travels in `sqlx::Error::Configuration`, following the repository's
existing configuration-wrapping precedent, because a new `FixtureError` variant
or `DatabaseCleanup` field would break exhaustive matches and struct literals.
Server-owned outcomes (unknown role, expired or wrong password, connection
limits, a failing `after_connect`) stay native SQLx errors. Every declared pool
is now constructed and registered before the first check, so a cancelled or
failed check cannot strand an unregistered pool; each pool is pushed as it is
built, so a construction panic cannot either.

Role provisioning, grants, their ordering relative to templates and `DROP ROLE`
are cluster-scoped consumer or outer-server policy, kept outside fixture cleanup.
Positional addressing of mixed-login pools was a known-invalid composition: a
wrong index could make a positive restricted assertion pass through the owner
pool. Review replaced the positional mixed-login constructor, before release,
with `ConnectionPlan::declare`, whose `pool` returns an opaque `PoolKey` bound to
its plan; `FixtureDatabase::pool` refuses another plan's key instead of mapping
it to a different pool. `ConnectionPlan::new` stays positional because all its
pools share the lease login. `FixtureDatabase::pools()` and
`DatabaseFixture::pools()` still give positional access on a `declare` plan:
both are public since 0.0.1, so removing them would be a hard cut, and close
inspection over every pool needs them. Their rustdoc directs `declare` plans to
`pool(key)`; positional selection on a mixed-login plan is a disclosed,
unprotected escape, not the protected path. An empty username
is left to the server because construction-time rejection needs a new error
variant. The binding holds at construction only, since native
`PgPool::set_connect_options` and `connect_options` are upstream escape hatches.
`PgProfiledPool` and `RunledgerDatabase` cannot host fixture pools, and its
acquisition verification hook is crate-private, so the live evidence uses the
reference pool settings and declared profile through public reset hooks, not
full three-hook parity; that composition belongs with `batter-nhu5`. Live cases
cover principals, authority, rejection, session-policy failure, both
cancellation points and registration after failure; mutations reverting
two-phase registration or keeping the owner's password fail them. Fresh-agent
usability evaluation is proposed and unexecuted.

## Intent reconciliation assessment (batter-hw3k)

D03's caller-assembled intent/job/definition correlation now belongs to native
Runledger's atomic scope. One query eliminates inter-query promotion/retention
races; private evidence fields and a job-carrying promoted variant prevent
fabricated or incomplete successful correlations. Exact `JobScope` excludes an
accidental administrative wildcard. Both old single-intent pool readers are
removed without shims because their full-row, owner-free shape restores the gap.
Queue reads cannot regain intent recording. Existing Q02/Q03 owners retain
continuity checks, savepoint cleanup and commit uncertainty.

Absence cannot locally prove a remote rollback, and an observation cannot freeze
future remote state. These remain explicit limitations, not new retry authority.
Application scope authorization and arbitrary SQL/admin-list escape hatches remain
application policy. Column grants constrain this reader's access, not all
capabilities of a role granted additional operations. Executable evidence covers
restricted roles, actual lost COMMIT acknowledgement and concurrent remote
transitions; no fresh-agent usability evaluation is claimed.
