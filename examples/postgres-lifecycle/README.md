# Native PostgreSQL lifecycle example

`batter-example-postgres-lifecycle` demonstrates a protected SQLx service root:
`Startup::scoped(...).with_unix_signals("signals")`, slot-owned native pool
creation, a bounded readiness query and error-preserving owned startup cleanup.
The initializer parses the application-owned `DATABASE_URL` into native
`PgConnectOptions`, reserves `postgres.pool` and calls `batter::sqlx::pool_in`,
which publishes awaited close before returning the lazy pool. Construction does
not contact PostgreSQL: the following `postgres.probe` stage runs the adapter's
bounded probe under a five-second child of the root's 15-second startup context.
The `postgres.schema` stage then runs the example's one versioned migration and
applies its managed routines, trigger and deferred constraint trigger through
`PgLease::migrate_and_apply`, and the `postgres.verify` stage gates readiness on
`VerificationPlan::sqlx_migrations(..).with_managed_objects(..)`; see
[managed objects](#managed-objects).
A long-running application component keeps this a service, not a finite command.
Its executable is `postgres_lifecycle`.
The executable keeps concrete startup and shutdown reports behind a fixed,
redacted process diagnostic so a trusted sink can inspect the source chain.

This executable follows the workspace's [Unix-only platform policy](../../README.md#platform-support).
Windows is unsupported and not planned. Shutdown uses native SIGINT/SIGTERM listeners.

Run from the workspace root with a local test database:

```sh
DATABASE_URL='postgres://user:password@localhost/database' \
  cargo run -p batter-example-postgres-lifecycle --bin postgres_lifecycle
```

The example runs `SELECT 1`, applies its own migration and managed objects to
the database it is given, and verifies them; it does not provision a
server/database, submit durable work, or retry a transaction. SQLx remains a native
dependency of this executable and is not part of the foundation's library API.
Live checks exercise real queries and pool closure; they do not establish
transaction commit/cancellation behavior.

Every returned failure reaches an explicit exit handler that prints only
`Error: process failed` and exits with code 1. Original startup
causes, cleanup reports and shutdown failures remain available there for a
trusted diagnostic sink; neither Debug nor source chains are printed by default.
Do not propagate these rich errors from a `main` returning `Result`: Rust's
termination implementation prints Debug. The explicit `ExitCode` boundary owns
that output policy, including when writing the fixed diagnostic fails.
Subprocess tests exercise missing, non-Unicode and malformed database URLs
without contacting PostgreSQL. Rust's default panic hook remains unchanged.

The normal test run also covers successful completion, startup plus cleanup
failure, failed tasks, and failed shutdown cleanup through the same private
owned startup and exit functions used by main. No database is needed for those
control-flow tests. The example emits `PostgreSQL lifecycle ready` only after
the application and signal components have acknowledged initialization.

For a separately provisioned test database, select the live tests explicitly:

```sh
cargo test -p batter-example-postgres-lifecycle --bin postgres_lifecycle --locked tests::live:: -- --ignored
cargo build -p batter-example-postgres-lifecycle --bin postgres_lifecycle --locked
python3 scripts/smoke_postgres.py --binary target/debug/postgres_lifecycle
python3 scripts/smoke_postgres.py --binary target/debug/postgres_lifecycle --signal SIGINT
```

These commands require `DATABASE_URL`; missing configuration fails the selected
check. The four live tests are labelled ignored in ordinary database-independent
runs. Each creates its pool inside protected startup through the same `pool_in`
helper. Three use `SELECT 1` and division-by-zero errors to verify retained causes
and awaited pool closure after success, startup failure and task failure, and do
not modify the schema. The fourth runs the schema stage: it applies the migration
and managed objects, checks that the managed trigger maintains `updated_at` and
that the managed constraint trigger rejects a blank note at commit but accepts
one corrected earlier in the same transaction, that a repeated apply executes
nothing, that an out-of-band `DROP TRIGGER` fails the
readiness gate with a drift finding, and that the next apply restores it. None
of them creates a database. The smoke commands require actual
readiness, successful pool cleanup and exit code 0 after the chosen signal.

## Managed objects

`migrations/` holds forward-only history: the `lifecycle` schema and its
`records` table. `objects/` holds the managed routine
`lifecycle.touch_updated_at()` and the trigger `records_touch_updated_at` that
binds it, each as one `CREATE OR REPLACE` statement edited in place, and the
routine `lifecycle.require_note()` with the `DEFERRABLE INITIALLY DEFERRED`
constraint trigger `records_note_present` that binds it. PostgreSQL cannot
replace a constraint trigger, so its file is one `CREATE CONSTRAINT TRIGGER`
statement; whenever it or its routine changes, apply drops and re-creates the
trigger in the transaction that installs the routine, holding
`ACCESS EXCLUSIVE` on `records` until commit. Keep constraint triggers in
`objects/`, not in migrations: a migration commits before apply and could bind
only the previously installed routine body. A deferred check receives the row
as of its event, so `require_note` reads the row's current state.
`src/schema.rs` declares them in one `ManagedObjectManifest` with
`PublicExecute::Revoke` and the ledger `lifecycle.managed_objects`, and derives
the expected SQLx history from the same embedded migrator. Startup passes that
one manifest value to both setup and the readiness gate, so the applied set and
the verified set cannot diverge. A repeated startup with unchanged files
executes no DDL; a release that edits a file replaces only that object. A
verification failure keeps the complete report in `schema::SchemaMismatch` for
a trusted sink and prints only the fixed process diagnostic. The SQLx ledger is
expected in `public`, the first schema of a default search path. The login in
`DATABASE_URL` therefore needs to create the schema, own its objects and read
both ledgers; production roots usually split the migration login from the
serving login, and Batter does not transfer ownership or grant anything.

Version 0.1.0; Rust 1.94 minimum, required by SQLx 0.9.0; publishing disabled.
MIT licensed.
