# batter-flags-postgres

Experimental PostgreSQL 18 flags store. `PooledStore` owns each acquisition,
transaction, timeout and connection disposition through `batter-sqlx`. It does
not publish snapshots, register process tasks, publish catalogs, heartbeat or
prune. Those integrations have separate owners.

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
at most 1000 rows strictly after a version, in counter order; only explicit fields
expose actor, reason and row images. Default diagnostics do not print them.

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
transaction-local and capped by the remaining budget. Migration success uses
`PgLease::migrate`'s acknowledged idle-state pool-return policy; failure/drop
retires. No caller has to coordinate rollback, pooling or cleanup.

## Schema and authority

The initial migration and its SQLx ledger live in **`flags`**, independent of the
host's `_sqlx_migrations`. No existing migration is edited. `MIGRATOR` is the
strict, low-level bundle; `PooledStore::migrate` tolerates newer flags migrations
while validating known checksums. `CREATE_FLAGS_SQL` supports vendoring and
`REQUIRED_SCHEMA_REVISION` is 1. Migration checksums use SQLx's SHA-384.
Queries always qualify the flags schema; changing `search_path` cannot redirect
writes. Rows are unscoped; environment is a heartbeat label. The salt column is
reserved for deferred rollout work, not an implemented rollout feature.

The migrator owns tables and functions. Serving writer logins must be non-owner,
non-superuser and unable to set `session_replication_role`. They need schema
USAGE, SELECT on control/state/catalog/entry/process tables, UPDATE(version) on
control and INSERT/UPDATE on state; history readers additionally need SELECT on
audit. They receive **no** INSERT/UPDATE/DELETE on audit, no schema CREATE, and
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
no actual replica or PostgreSQL 17 server is claimed. Shared core encoding
vectors, schema column lists and the initial migration checksum are pinned.
