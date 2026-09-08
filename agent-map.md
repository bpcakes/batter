# Agent Map

Fast jump index for agent-facing guidance in this repository.

## Root guide

- [Repository AGENTS.md](./AGENTS.md)

## Nested guides

- [Foundation](./crates/batter/AGENTS.md): lifecycle, operations, retry, cleanup,
  admission, and telemetry.
- [Axum adapter](./crates/batter-axum/AGENTS.md): HTTP policy, probes, rendering,
  and HTTP observations.
- [Test support](./crates/batter-test-support/AGENTS.md): generic test scripts
  and error-preserving result combination.
- [SQLx lifecycle example](./examples/postgres-lifecycle/AGENTS.md): native pool
  acquisition and explicit teardown.

## Suggested usage pattern

1. Start with the root [AGENTS.md](./AGENTS.md).
2. Open the nearest guide for the area you will change.
3. Follow that guide's entrypoint map before editing.
