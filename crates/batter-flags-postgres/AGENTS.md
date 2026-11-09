# Flags PostgreSQL store

## Purpose

Own the dedicated `flags` schema, immutable migration ledger, coherent loads,
audited CAS writes, history, archive and restore. Follow the root Unix policy.
Native SQLx ownership stays in `batter-sqlx`; evaluation and the invariant
judgement stay in flags core. This crate also owns catalog publication,
presence, pruning, the fleet view and the operator listings.

## Key entrypoints

- `src/store.rs`: validated timing and pool-backed protected owner.
- `src/write.rs`, `src/validate.rs`: transaction phases and ordered refusals.
- `src/migrate.rs`, `migrations/`: flags-only SQLx ledger and immutable SQL.
- `src/load.rs`, `src/history.rs`: bounded repeatable-read inspection.
- `src/request.rs`, `src/outcome.rs`, `src/errors.rs`: validated inputs, exhaustive
  outcomes, redacted retained causes.
- `src/relations.rs`: the write-side invariant judgement, replaying
  `Manifest::invariant_holds` per running declarer catalog and process kind.
- `src/catalogs.rs`, `src/presence.rs`, `presence/stop.rs`, `src/prune.rs`,
  `src/fleet.rs`, `src/listing.rs`: catalog publication, heartbeats and the
  stop marker, bounded pruning, the fleet view and the operator listings.
- `src/atomic.rs`: the native transaction owners presence and pruning run in.

## Edit here for X

Add forward migrations after the initial migration has shipped. Keep checksum
pins and older-column insert compatibility tests. SQL identifiers are fixed and
queries schema-qualified. New integration seams must preserve native connection
ownership; do not expose `PgConnection` or add a caller rollback protocol.

## Invariants

One locked counter, one row change, one audit row, one transactional NOTIFY.
No-op/refusal rolls back the counter. Refusals are judged inside the write's
transaction, after the counter lock; a kill switch is never refused for the
flags engaging it holds. Silent declarers still block incompatible
writes/archive. Uncertain commit never becomes ordinary failure or automatic
replay. The private mutation callback cannot accept arbitrary caller SQL.
Default formatting does not expose values or native error contents.
`PooledStore::migrate` refuses (`ErrorClass::SearchPath`) when its own
session's default path resolves into `flags`, decided before anything is
migrated by a probe that creates the schema only in a transaction it always
rolls back. Never move the check after the migrator: a refusal must mean
nothing was applied.
`load`, `preflight` and every operator read call `PooledStore::refuse_standby`
first in their snapshot, before any schema or row; keep that one check, which
the test-only `standby` hook exercises.
The write interrupt retains its terminal state across preparation and commit
validation. A closed sender disables interruption; it is never polled again.

## Common commands

`cargo test -p batter-flags-postgres --locked` and the ignored counterpart with
`BATTER_FLAGS_POSTGRES_TEST_DATABASE_URL` pointing only to disposable PostgreSQL
18. Use `--test-threads=1` for live cases. The harness owns database cleanup;
container setup stays external. Run the repository's complete `scripts/verify.sh`
after focused checks pass. See README for native owner limitations and evidence.
