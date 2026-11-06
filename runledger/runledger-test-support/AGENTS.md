# Native Runledger test support

## Purpose
Own PostgreSQL 18 Docker fixtures through Bollard, per-test databases and process-bound container cleanup.

## Key entrypoints
`src/lib.rs` exports the public database fixtures and scoped environment helper.
`src/postgres_container.rs`, `src/docker.rs`, `src/container_start.rs`, `src/container_lifecycle.rs`, `src/db_lifecycle.rs`,
`tests/container_lifecycle.rs`, and `build.rs`.

## Edit here for X
Native database provisioning and process cleanup fixtures belong here. Generic
Batter test utilities and Batter SQLx external fixture policies remain separate.

## Invariants
Preserve Unix process semantics, optional reaper failure behavior, awaited native
cleanup, migration-copy equality and PostgreSQL 18 prerequisites. Missing Docker
is a test failure. This package must not become a production dependency of the
Batter foundation or facade.
Docker owns dynamic host-port allocation on IPv4 loopback, matching the fixture
URL. Failed startup cleanup verifies the
attempt label before removing its immutable container ID and anonymous volumes;
honor explicit keep policy and retain both startup and cleanup diagnostics.
Keep Testcontainers and `astral-tokio-tar` absent from the dependency graph.
The private Docker handle releases a runtime-owned cleanup task on drop; explicit
test removal awaits its result. Runtime death cannot prove remote cleanup. The
existing optional CLI reaper retains process-exit ownership of shared containers.

## Common commands
From the root: `cargo test -p runledger-test-support --locked` and
`cargo clippy -p runledger-test-support --all-targets --locked -- -D warnings`.
