# SQLx composition example guide

## Purpose

Demonstrate native SQLx resource ownership with the foundation. This is an
unpublished executable package, not a database adapter API.

Follow the root [Unix-only platform policy](../../AGENTS.md#platform-scope).
Windows support and non-Unix fallbacks are out of scope.

## Key entrypoints

- `src/main.rs` acquires the pool, registers its close, and observes startup.
- Its private `complete_startup` and `report_exit` functions retain failures and
  determine the executable outcome; `src/tests.rs` tests those same paths.
- `src/tests/live.rs` contains explicitly selected tests against an externally
  provisioned database. Missing configuration must fail that selected target.
- `src/support.rs` holds example-owned signal and shutdown-budget policy.
- `tests/diagnostics.rs` runs `tests/diagnostics.py`, an external watchdog for
  actual executable configuration failures. It reuses the repository's bounded
  Unix process owner; keep credential fixtures and assertions in this example.

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
cargo test -p batter-example-postgres-lifecycle --test diagnostics --locked
python3 scripts/test_smoke_postgres.py -v
cargo run -p batter-example-postgres-lifecycle --bin postgres_lifecycle
scripts/jig check test
```

Select the live checks with `cargo test -p batter-example-postgres-lifecycle
--bin postgres_lifecycle --locked tests::live:: -- --ignored`, with `DATABASE_URL`
already configured. `scripts/smoke_postgres.py` exercises the built executable's
readiness, signal exit and pool cleanup; it never provisions PostgreSQL.

Running requires `DATABASE_URL` for an existing local test database. Record a
live execution separately from compilation in the root validation document.
