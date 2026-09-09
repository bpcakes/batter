# batter-sqlx

An independently selected PostgreSQL adapter for SQLx 0.9 and Batter. Rust 1.94
minimum, Unix-only, unpublished. The foundation does not depend on this package.

`PgLease::acquire(&pool, &context)` bounds acquisition using the existing operation
budget. Move the lease into the bounded operation future and use
`lease.connection()` for native queries and `Connection::begin`. After fully
awaited successful query, commit or rollback, call `lease.return_to_pool()`.
Every other drop detaches and drops the client connection, including error,
unwinding and interruption when the lease is owned by the interrupted future.
See the crate rustdoc for a compiling native transaction example.

Retirement releases local pool accounting. It does **not** acknowledge remote
cancellation, rollback, or server-session disappearance. Interrupted SQL can
retain server locks after local drop, and detached sessions can exceed the
pool's `max_connections` and outlive `Pool::close`. Applications own remote
outcome reconciliation and server-side resource policy. Ordinary successful
return is SQLx's asynchronous health-check path, not an immediate reuse promise.

`probe(&pool, &context)` performs SELECT 1 with acquisition and query sharing the
total budget. `register_pool_close(&mut supervisor, name, &pool)` registers native
closure in existing explicit LIFO cleanup. It does not close on registration
failure; the caller retains the pool and must await teardown. The runnable
[PostgreSQL lifecycle example](../../examples/postgres-lifecycle/README.md)
demonstrates both helpers against an existing database.

`SqlxFailure` retains the original SQLx cause with fixed Debug/Display.
`FailureClass` classifies native variants, never error strings, and grants no
replay permission. Trusted source-chain inspection can still disclose native
contents. SQLx logging and Rust panic-hook output remain application-owned.
Transaction commit uncertainty stays separate from operation interruption;
neither pool retirement nor an I/O error proves a write did not commit.

No database creation, migration, repository, transaction manager, or replay is
supplied. Select TLS through native SQLx features in the consumer.

Run `cargo test -p batter-sqlx --locked` for offline contracts. Live cases are
ignored in ordinary all-feature checks. Configure `DATABASE_URL` for an external
disposable database and run `bash scripts/test_sqlx_live.sh`; missing prerequisites
fail. The runner uses up to six simultaneous server sessions, including retired sessions and session advisory locks,
verifies case inventory, and bounds the test process. Provisioning stays external.
