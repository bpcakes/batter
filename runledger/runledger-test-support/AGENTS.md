# Native Runledger test support

## Purpose
Own PostgreSQL 18 testcontainers, per-test databases and process-bound container cleanup.

## Key entrypoints
`src/lib.rs` exports the public database fixtures and scoped environment helper.
`src/postgres_container.rs`, `src/container_lifecycle/`, `src/db_lifecycle.rs`,
`tests/container_lifecycle.rs`, and `build.rs`.

## Edit here for X
Native database provisioning and process cleanup fixtures belong here. Generic
Batter test utilities and Batter SQLx external fixture policies remain separate.

## Invariants
Preserve Unix process semantics, optional reaper failure behavior, awaited native
cleanup, migration-copy equality and PostgreSQL 18 prerequisites. Missing Docker
is a test failure. This package must not become a production dependency of the
Batter foundation or facade.
Owned disposable PostgreSQL data lives on a bounded 2 GiB tmpfs mount under
`/var/lib/postgresql`; normal WAL/fsync settings remain enabled. Preserve the
mount/settings assertions and process-exit controls. External servers are
unmodified. These fixtures provide no disk-durability or restart-persistence evidence.

## Common commands
From the root: `cargo test -p runledger-test-support --locked` and
`cargo clippy -p runledger-test-support --all-targets --locked -- -D warnings`.
