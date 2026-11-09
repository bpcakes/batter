# batter-flags-postgres

Experimental PostgreSQL 18 flags store. `PooledStore` owns each acquisition,
transaction, timeout and connection disposition through `batter-sqlx`. It
stores audited writes, catalogs, process presence and the fleet view; it does not
publish snapshots or register process tasks, which `batter-flags` owns.

```rust
use batter_flags_core::{FlagKey, SwitchState, Version};
use batter_flags_postgres::{Actor, Change, ChangeReason, ExplicitValue,
    PooledStore, Precondition, StoreTiming, Write, WriteOutcome};
use std::time::Duration;
use tokio::time::Instant;

# async fn example(pool: sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
let timing = StoreTiming::new(Duration::from_secs(2), Duration::from_secs(3),
    Duration::from_secs(1), Duration::from_secs(3))?;
let store = PooledStore::new(pool, timing);
store.migrate(Instant::now() + Duration::from_secs(30)).await?;
let change = Change::new(Actor::new("operator")?,
    ChangeReason::new("contain outbound failures")?,
    Precondition::UnchangedSince(Version::new(0)));
let request = Write::explicit(FlagKey::new("outbound_halt")?,
    ExplicitValue::Switch(SwitchState::Engaged), change);
match store.write(request, Instant::now() + Duration::from_secs(3)).await? {
    WriteOutcome::Written { .. } | WriteOutcome::WrittenUndeclared { .. } => {},
    WriteOutcome::Unchanged { .. } => {},
    WriteOutcome::RevisionMismatch { .. } => {},
    WriteOutcome::Refused(_) => {},
    WriteOutcome::Uncertain { .. } => { /* reconcile before another write */ },
}
# Ok(()) }
```

Use `Write::set` and `Write::kill` for retained declarations. `Write::explicit`
requires a compatible kind/value/state arm and may create a never-declared key;
it still applies environment, archive, encoding, CAS and fleet checks. This is
an application capability, not operator-CLI authority. Actors and reasons are
validated text, not authentication; the audit separately records `session_user`.
Archive/restore take `Archive::new(key, change)`; archives load as absent.

`load` returns an opaque `Loaded` read at one REPEATABLE READ READ ONLY primary
snapshot. `loaded.candidate(&validation_scope)` passes all rows to the existing
core validator. Higher/extra-field encodings remain present so core can hold them.
A standby returns `ErrorClass::Standby` without a candidate. `history` returns
a `HistoryPage` of at most `HISTORY_PAGE` (1000) rows strictly after a version,
in counter order, with `next: Continuation` saying whether more follow; `keys`,
`key`, `history` and the fleet reads refuse a standby and an unusable schema in
their own snapshot. A `ProcessPresence` heartbeats and stops only through the
store that issued it. Only explicit fields expose actor, reason and row images. Default diagnostics do not print them.

## Presence, prune and the fleet view (T-13)

A `ProcessPresence` holds what does not change between heartbeats: a process id
minted for it, its application, environment and build labels, its catalog as a
`CatalogPublication`, the revisions it resolved and its `Cadence` (liveness and
retention, whole seconds from 1 s to 24 h). It is made from the store that
heartbeats it (`ProcessPresence::new(&store, ...)`), and
`presence.heartbeat(&report, deadline)` runs one READ COMMITTED transaction: it takes a key-share lock on the stored
catalog, inserting the catalog and its entries only when none is stored, refreshes
the catalog's reference at most once a day, and upserts the process row with a
server-clock expiry and the row's own cadence. A heartbeat never clears a stop;
`presence.stop(deadline)` consumes the presence, records the stop and leaves the
expiry alone. Neither is audited or retried, so
an unknown commit is `ErrorClass::CommitUnknown` and the next heartbeat restates
presence.

`prune(PruneLimit, deadline)` deletes, with `FOR UPDATE SKIP LOCKED`, process rows
whose expiry is older than the server clock minus their own retention, then
catalogs unreferenced for 30 days that no process row references. A heartbeat
waiting on a prune's lock publishes its catalog again once the delete commits.

`fleet` and `fleet_declaring(&key)` read every process row (or every declarer of
a key) with the head in one read-only snapshot, each as `ProcessState::{Live,
Silent, Stopped}`. `Fleet::rollout(version)` answers whether every Live process
applied a version: only a control-plane publication at or above it counts, never
a persisted file or the declared base.

## Ordering and failure contract (T-12)

One counter row serializes first writes and updates. The store locks and bumps it,
reads the row and catalogs, checks (in order) stored encoding, requested kind,
archive status, unchanged state, CAS, environment restrictions, declaration,
live/silent decodability and fleet regression, then changes one row. Refusals and
no-ops roll back the bump. `UnchangedSince(v)` uses zero for an absent row;
above-head requests are refused. Equality is tested before CAS, so an already
satisfied request is unchanged even with an old CAS revision.

Live means not stopped and not expired by the server clock. Silent means expired
but not stopped/pruned. Both block incompatible writes and archive; only Live
counts for `Written`. Stopped declarers yield `NoLiveDeclarer`, unreferenced
retained catalogs yield `Orphaned`, and explicit writes without any retained
catalog yield `NeverDeclared`. A live control-plane snapshot above the database
head blocks writes; a persisted-file or not-loaded heartbeat does not.

The counter and audit versions have no allocation gaps and follow commit order.
Triggers reject unaudited writes, stale/future row versions, counter decrement,
unpaired counter advances at COMMIT, state deletion, audit mutation and table
truncation. Trigger functions are SECURITY DEFINER with a fixed safe search path,
qualified table resolution and no PUBLIC execute grant. Their errors are static.
State changes emit a version-only NOTIFY in the same transaction. A full NOTIFY
queue can reject **every** write, including a kill switch, at COMMIT. The
application's declared settings floor remains the outage containment mechanism.

`Uncertain` retains native `batter-sqlx::CommitUnconfirmed` or a commit-phase
deadline, with the provisional version. SQLSTATE ErrorResponses rejecting COMMIT
(such as 54000/P0001) are certain errors; connection-loss/unknown-completion and
shutdown codes remain uncertain. There is no replay. Before COMMIT is invoked,
deadline/drop retires the connection and cannot commit this library's private
body; local retirement is not acknowledgement of server-session termination.

The current Batter atomic/snapshot owners retire even after acknowledged success
or rollback. This store retains that policy instead of adding a second pooling
protocol. The native COMMIT owner does not expose its send point; a timeout in
its validate-and-commit future is conservatively uncertain. The snapshot owner
also has no separately bounded acquisition seam, so load/history and migration
preflight are wholly capped by the acquire limit as well as the caller deadline.
Writes cap acquisition/setup separately and bound all subsequent work by the
caller deadline. Server statement, lock and idle-in-transaction settings are
transaction-local and capped by the remaining budget for store transactions.
Migration uses `PgLease::migrate_until`: native SQLx locks outside a transaction
and starts its own migration transactions, so the owner installs session limits
that inherit into those transactions and retires even on success. Migration
scripts are trusted and can override settings; migration interruption does not
prove that no migration committed. No caller coordinates pooling or cleanup.

`operation_until` and `inspect_until` keep an observed application rejection
outside the bounded native recovery future. If cleanup times out, the store
retains that rejection plus `ErrorClass::Deadline` through `RollbackFailure`;
it does not replace an observed SQLSTATE with a cause-free timeout. External
cancellation of the whole store future still discards its result. Request kinds,
row/archive and process presence states, and prepared commit/rollback decisions
are explicit internal enums. Only consuming native completion proofs turn a
prepared decision into an acknowledged public outcome.

## Schema and authority

The initial migration and its SQLx ledger live in **`flags`**, independent of the
host's `_sqlx_migrations`. No existing migration is edited. `MIGRATOR` is the
strict, low-level bundle; `PooledStore::migrate` tolerates newer flags migrations
while validating known checksums. `CREATE_FLAGS_SQL` supports vendoring and
`REQUIRED_SCHEMA_REVISION` is 1. Migration checksums use SQLx's SHA-384.
Queries always qualify the flags schema; changing `search_path` cannot redirect
writes. Rows are unscoped; environment is a heartbeat label. The salt column is
reserved for deferred rollout work, not an implemented rollout feature.

The reverse direction is the application's: PostgreSQL's default `search_path`
is `"$user", public`, so for a role named `flags` the `flags` schema comes first
once it exists. The host's unqualified names then resolve into it, SQLx's
default `_sqlx_migrations` ledger included, and unqualified `CREATE TABLE`
places tables there. A strict host migrator then fails with `VersionMissing`.
One with `set_ignore_missing(true)` records its versions in the flags ledger
and creates its tables in `flags` without error, and `PooledStore::migrate`,
which also ignores versions it does not know, never refuses them; only the
strict `MIGRATOR` does. `PooledStore::migrate` therefore refuses with
`ErrorClass::SearchPath` when a session of its pool would resolve into
`flags` (`batter-jkw6.34`), before anything is migrated (`batter-jkw6.41`).
PostgreSQL resolves that session's default path, which `DISCARD ALL` restores,
and its `current_schemas` lists only existing schemas, so the probe creates the
schema if it is absent inside a transaction it always rolls back: a refused
first migration creates and applies nothing. The probe needs only the database
`CREATE` privilege that SQLx's own `CREATE SCHEMA IF NOT EXISTS` needs on every
run. A server or search-path refusal applies nothing; a failure while
migrating can follow committed migrations, and a rerun applies nothing twice.
The probe and the migration use separate connections: those of one
pool share their defaults unless the login's role or database settings or the
pool's connect options (`PgPool::set_connect_options`) changed while it held
connections. Sessions of other pools or logins, including a separate host
migration login, stay the application's to configure: do not name such a role
`flags`, or give it an explicit `search_path` without `flags`. The strict
`MIGRATOR` does not check.

The migrator owns tables and functions. Serving writer logins must be non-owner,
non-superuser and unable to set `session_replication_role`. They need schema
USAGE, SELECT on control/state/catalog/entry/process tables, UPDATE(version) on
control and INSERT/UPDATE on state; history readers additionally need SELECT on
audit. Every operator read first reads control's `schema_revision` (with
`singleton`) for its guard; `batter_flags::grants` lists each operation's exact
privileges. They receive **no** INSERT/UPDATE/DELETE on audit, no schema CREATE, and
no ownership membership. Database owners/superusers/replica-mode sessions are
outside the guard claims. Schema creation does not grant application roles;
fixture setup does, and the later adapter grants inventory owns production
selection. No new process-registration API is introduced here.

## Verification

```sh
cargo test -p batter-flags-postgres --locked
# Explicitly provide a disposable PostgreSQL 18 administrative endpoint:
cargo test -p batter-flags-postgres --locked -- --ignored --test-threads=1
```

The second command requires `BATTER_FLAGS_POSTGRES_TEST_DATABASE_URL`. It uses
the repository-pinned external `postgres-test-harness` to create and clean up
isolated databases. Docker provisioning stays outside this library. The wire
proxy test suppresses an actual successful COMMIT acknowledgement; the queue
failure test injects the server's 54000 ErrorResponse at COMMIT without exhausting
the cluster's queue. Standby and older-server checks use injected server values;
no actual replica or PostgreSQL 17 server is claimed. The standby refusal of
`load`, `preflight` and the operator reads (`keys`, `key`, `history` and the
fleet reads) goes through one store check, which a test-only hook makes read
as a standby: the ignored live unit test
`every_read_and_the_load_refuse_a_standby_before_reading_anything` asserts
`Standby` from each of them on an unmigrated store, before any schema or row
is read. The read-only snapshot pins `search_path` to `pg_catalog, pg_temp`, so
no session can stand in for a replica, and no suite runs one: a real replica's
`pg_is_in_recovery()` stays unverified. Shared core encoding
vectors, schema column lists and the initial migration checksum are pinned.
