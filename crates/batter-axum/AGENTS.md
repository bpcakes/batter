# Axum adapter guide

## Purpose

Translate the foundation's operational contracts into Axum request and probe
behavior. Keep application security, business logic, and wire-envelope policy
at the composition root.

Follow the root [Unix-only platform policy](../../AGENTS.md#platform-scope).
Windows support and non-Unix fallbacks are out of scope.

## Key entrypoints

- `src/lib.rs` contains `RequestPolicy`, `request_scope`, probes, and failures.
- `examples/http_service.rs` demonstrates router/lifecycle composition.
- `tests/http.rs`, `tests/telemetry.rs`, and `tests/scoped_dispatch.rs` cover HTTP
  failure behavior, sanitized observations, and future destruction.

## Edit here for X

Change HTTP policy and rendering here. Change operation/lifecycle semantics in
`batter`. Keep `RequestPolicy`'s combined readiness and deadline contract unless
a separately approved API change calls for decoupling. Update the root HTTP
contract, source map, implemented status, owning Bead, and validation with changes.

## Invariants

The adapter depends on the foundation, never the reverse. Use
`batter::telemetry::with_current_dispatch` inside the async request entrypoint to
retain first-poll capture and protect full future destruction. Keep observation
guards and nested spans inside the wrapped future. Do not duplicate its private
pin/drop implementation. Bound response construction without claiming body
streaming or detached connection-task shutdown. Keep probe routes separate from
guarded business routes. Do not log cause contents or untrusted request fields.

## Common commands

Run from the workspace root:

```sh
cargo test -p batter-axum --locked
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
scripts/jig check test
```
