# PostgreSQL operation adapter guide

## Purpose

Optional SQLx 0.9 PostgreSQL connection disposition; native operations and
transactions remain application-owned. Follow the root Unix-only policy.

## Key entrypoints

- `src/lib.rs`: default-retiring lease, bounded probe, pool cleanup registration.
- `src/failure.rs`: redacted native causes and conservative classifications.
- `tests/postgres_live.rs`: explicitly selected external-database contracts.

## Edit here for X

Keep connection ownership mechanics here and migration, replay, SQL contents,
transaction completion and provisioning policy in applications. The runnable
consumer is `examples/postgres-lifecycle`.

## Invariants

Dropping an unsuccessful lease detaches and drops the client connection.
Only explicit acknowledged completion permits ordinary pool return. Local
capacity release and Pool::close do not acknowledge server-session termination,
rollback or remote cancellation; detached sessions can exceed max_connections.
Never log native error contents automatically. Core and generic support remain
independent of SQLx. Never provision PostgreSQL in this package.

## Common commands

Run `cargo test -p batter-sqlx --locked` for offline contracts and
`bash scripts/test_sqlx_live.sh` with DATABASE_URL for the ignored live cases.
The explicit invocation must fail when prerequisites are missing. Also run the
root two-toolchain verification, HTTP smoke profiles and required Jig gates.
