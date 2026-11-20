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
URL. When this fixture owns the container, its admin URL and every derived
database URL end in `?hostaddr=127.0.0.1&sslmode=disable`: the address pin
overrides an ambient
`PGHOSTADDR` in SQLx 0.9.1 and libpq, and `sslmode=disable` matches the
native test proxies' plaintext; derivation re-appends only that query, never
the admin query. Every fixture mode refuses any ambient client `PG*` variable (all but `PGDATA` and `PG_*`)
before provisioning: SQLx fills unset settings from them, but native test
proxies dial and forward to the URL host themselves, without TLS. Every
external connection is therefore pinned to `sslmode=disable`, and derived URLs
carry only that from the admin URL, so every query key is refused except a `hostaddr` that repeats its
own IP literal host, and the URL must name a DNS or IPv4 host the native
proxies can dial and exactly one database path segment (`src/external_admin.rs`); otherwise provisioning and
leases could reach different servers or leases could silently lose TLS
settings. Failed startup cleanup verifies the
attempt label before removing its immutable container ID and anonymous volumes;
honor explicit keep policy and retain both startup and cleanup diagnostics.
Keep Testcontainers and `astral-tokio-tar` absent from the dependency graph.
The private Docker handle releases a runtime-owned cleanup task on drop; explicit
test removal awaits its result. Runtime death cannot prove remote cleanup. The
existing optional CLI reaper retains process-exit ownership of shared containers.

## Common commands
From the root: `cargo test -p runledger-test-support --locked` and
`cargo clippy -p runledger-test-support --all-targets --locked -- -D warnings`.
