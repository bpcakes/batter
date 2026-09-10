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

Run `cargo test -p batter-sqlx --features test-support --locked` for offline contracts and
`bash scripts/test_sqlx_live.sh` with DATABASE_URL for the ignored live cases.
The explicit invocation must fail when prerequisites are missing. Also run the
root two-toolchain verification, HTTP smoke profiles and required Jig gates.

## Optional fixture support

The `test-support` feature exposes `test_support::FixtureSuite`, declared native
pool capacity, ordered template inputs and bounded lock observation. Use
`FixtureSuite::start` to retain native acquisition producers before cancelling
waiters is possible. The driver joins producers after body exit, then closes
pools, cleans leases and drains deferred cleanup. Reports preserve acquisition
failures even when the body handles them. Template initializers must be Send and
static; panic reaches upstream's awaited abort path. Their own pools/operations
still require explicit close/join.

Harness clones share native admission; other owners must release leases for
waiting acquisitions to progress. The caller owns server shutdown after all runs
finish. Owned reports and borrowed wait views are must-use; driver join alone
is not fixture success. `FixtureRun::wait` can be cancelled and resumed; dropping it loses report
observation, and runtime death has no completion guarantee. `PoolAcquire` means
cleanup is pending; low-level `Connect` reports attempted cleanup. The low-level
fixture owner is must-use and still requires explicit finish. Provisioning and
caching stay upstream; migrations and row builders stay consumer-owned.
