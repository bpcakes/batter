# Native PostgreSQL lifecycle example

`batter-example-postgres-lifecycle` demonstrates native SQLx pool acquisition,
a readiness query, explicit pool-close registration, and error-preserving
partial-startup cleanup with `batter`. Its executable is `postgres_lifecycle`.
The executable keeps concrete startup and shutdown reports behind a fixed,
redacted process diagnostic so a trusted sink can inspect the source chain.

This executable follows the workspace's [Unix-only platform policy](../../README.md#platform-support).
Windows is unsupported and not planned. Shutdown uses native SIGINT/SIGTERM listeners.

Run from the workspace root with a local test database:

```sh
DATABASE_URL='postgres://user:password@localhost/database' \
  cargo run -p batter-example-postgres-lifecycle --bin postgres_lifecycle
```

The example runs `SELECT 1`; it does not provision a server/database, apply
migrations, submit durable work, or retry a transaction. SQLx remains a native
dependency of this executable and is not part of the foundation's library API.
Live checks exercise real queries and pool closure; they do not establish
transaction commit/cancellation behavior. Executed versions and platforms are
recorded in [validation](../../docs/validation.md).

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
completion and exit functions used by main. No database is needed for those
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
check. The three live tests are labelled ignored in ordinary database-independent
runs. They use `SELECT 1` and division-by-zero errors to verify retained causes
and awaited pool closure after success, startup failure and task failure. They
do not create a database or modify its schema. The smoke commands require actual
readiness, successful pool cleanup and exit code 0 after the chosen signal.

Version 0.1.0; Rust 1.94 minimum, required by SQLx 0.9.0; publishing disabled.
MIT licensed.
