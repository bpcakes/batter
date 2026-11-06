# Flags PostgreSQL store

## Purpose

Own the dedicated `flags` schema, immutable migration ledger, coherent loads,
audited CAS writes, history, archive and restore. Follow the root Unix policy.
Native SQLx ownership stays in `batter-sqlx`; evaluation stays in flags core.
Catalog publication, presence and pruning have a separate delivery owner.

## Key entrypoints

- `src/store.rs`: validated timing and pool-backed protected owner.
- `src/write.rs`, `src/validate.rs`: transaction phases and ordered refusals.
- `src/migrate.rs`, `migrations/`: flags-only SQLx ledger and immutable SQL.
- `src/load.rs`, `src/history.rs`: bounded repeatable-read inspection.
- `src/request.rs`, `src/outcome.rs`, `src/errors.rs`: validated inputs, exhaustive
  outcomes, redacted retained causes.

## Edit here for X

Add forward migrations after the initial migration has shipped. Keep checksum
pins and older-column insert compatibility tests. SQL identifiers are fixed and
queries schema-qualified. New integration seams must preserve native connection
ownership; do not expose `PgConnection` or add a caller rollback protocol.

## Invariants

One locked counter, one row change, one audit row, one transactional NOTIFY.
No-op/refusal rolls back the counter. Silent declarers still block incompatible
writes/archive. Uncertain commit never becomes ordinary failure or automatic
replay. The private mutation callback cannot accept arbitrary caller SQL.
Default formatting does not expose values or native error contents.

## Common commands

`cargo test -p batter-flags-postgres --locked` and the ignored counterpart with
`BATTER_FLAGS_POSTGRES_TEST_DATABASE_URL` pointing only to disposable PostgreSQL
18. Use `--test-threads=1` for live cases. The harness owns database cleanup;
container setup stays external. Run the repository's complete `scripts/verify.sh`
after focused checks pass. See README for native owner limitations and evidence.
