# Agent Map

Fast jump index for agent-facing guidance in this repository.

## Root guide

- [Repository AGENTS.md](./AGENTS.md)

## Nested guides

- [Foundation](./crates/batter/AGENTS.md): lifecycle, operations, retry, cleanup,
  admission, dependency health, and telemetry.
- [Axum adapter](./crates/batter-axum/AGENTS.md): HTTP policy, probes, rendering,
  and HTTP observations.
- [SQLx adapter](./crates/batter-sqlx/AGENTS.md): native PostgreSQL leases,
  client retirement, bounded probes, explicit pool cleanup and opt-in isolated
  database fixture ownership.
- [Test support](./crates/batter-test-support/AGENTS.md): generic test scripts
  and error-preserving result combination.
- [SQLx lifecycle example](./examples/postgres-lifecycle/AGENTS.md): native pool
  acquisition and explicit teardown.
- [Reference compatibility](./examples/reference-service/AGENTS.md): native type
  identity, live migration/job/lease probes and pinned upstream contracts.

## Private shared test sources

- [Repository process machinery](./test-support/README.md): std-only Unix launch,
  capture and watchdog sources included by foundation and Axum tests. This is
  not the `batter-test-support` Cargo crate; preserve the documented relative layout.

## Suggested usage pattern

1. Start with the root [AGENTS.md](./AGENTS.md).
2. Open the nearest guide for the area you will change.
3. Follow that guide's entrypoint map before editing.

Owned initialization: `crates/batter/src/startup.rs` and `startup/` retain the
supervisor through failure cleanup or running handoff.
