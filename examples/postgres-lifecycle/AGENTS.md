# SQLx composition example guide

## Purpose

Demonstrate native SQLx resource ownership with the foundation. This is an
unpublished executable package, not a database adapter API.

Follow the root [Unix-only platform policy](../../AGENTS.md#platform-scope).
Windows support and non-Unix fallbacks are out of scope.

## Key entrypoints

- `src/main.rs` acquires the pool, registers its close, and observes startup.
- `src/support.rs` holds example-owned signal and shutdown-budget policy.

## Edit here for X

Keep pool configuration, schema checks, and application teardown in this
composition root. Future transactions must use native SQLx transaction values.
Create reusable adapters only after actual consumers establish shared mechanics.
Keep application fixture helpers beside their integration tests.

## Invariants

Register cleanup after acquisition and explicitly drive it after startup failure.
Preserve original causes and the cleanup report without printing raw secrets.
No hidden migrations, automatic transaction replay, database provisioning, or
generic pool/transaction abstraction. A timeout does not prove rollback.
Do not import or relocate the external PostgreSQL harness as part of this package.

## Common commands

Run from the workspace root:

```sh
cargo check -p batter-example-postgres-lifecycle --all-targets --locked
cargo run -p batter-example-postgres-lifecycle --bin postgres_lifecycle
scripts/jig check test
```

Running requires `DATABASE_URL` for an existing local test database. Record a
live execution separately from compilation in the root validation document.
