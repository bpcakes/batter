# Native PostgreSQL lifecycle example

`batter-example-postgres-lifecycle` demonstrates native SQLx pool acquisition,
a readiness query, explicit pool-close registration, and error-preserving
partial-startup cleanup with `batter`. Its executable is `postgres_lifecycle`.

Run from the workspace root with a local test database:

```sh
DATABASE_URL='postgres://user:password@localhost/database' \
  cargo run -p batter-example-postgres-lifecycle --bin postgres_lifecycle
```

The example runs `SELECT 1`; it does not provision a server/database, apply
migrations, submit durable work, or retry a transaction. SQLx remains a native
dependency of this executable and is not part of the foundation's library API.
Live PostgreSQL execution is unverified; compilation alone does not establish
database or commit/cancellation behavior.

Version 0.1.0; Rust 1.94 minimum, required by SQLx 0.9.0; publishing disabled.
MIT licensed.
