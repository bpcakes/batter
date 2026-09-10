# Axum adapter guide

## Purpose

Translate the foundation's operational contracts into Axum request and probe
behavior. Keep application security, business logic, and wire-envelope policy
at the composition root.

Follow the root [Unix-only platform policy](../../AGENTS.md#platform-scope).
Windows support and non-Unix fallbacks are out of scope.

## Key entrypoints

- `src/lib.rs` contains `RequestPolicy`, `observe_http`, `request_admission`,
  `HttpObservationLevel`, the combined `request_scope` compatibility entry point,
  probes, and failures.
- `src/observation.rs` privately owns response observation and tracing lifetime;
  its single internal composition entry has no admission policy.
- `src/correlation.rs` owns opt-in `operational_http`, generated `CorrelationId`
  and the standard infrastructure renderer; it composes the existing observer once.
- `src/readiness.rs` owns read-only dependency/lifecycle reasons and severity policy.
- `src/serving.rs` registers a bound native listener/router with the supervisor.
- `examples/http_service.rs` demonstrates adoption of these public helpers.
- `tests/http.rs`, `tests/telemetry.rs`, `tests/observation.rs` and
  `tests/scoped_dispatch.rs` cover failures, complete-router observations,
  middleware placement, and future destruction. `tests/operational/` covers forged/concurrent IDs,
  all readiness reasons, native startup/drain and a body surviving wrapper abort.
- `tests/http_lifetime.rs` and `tests/http_lifetime/` own real HTTP/1.1 socket,
  handler/body, direct-server and cleanup comparisons. They reuse only private
  workspace `test-support/process/` mechanics, never another package's fixtures
  or self-tests; keep report inspection and later body release separate.

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
guarded business routes. Apply `observe_http` after assembling routes/fallback;
use `request_admission` inside it. Nesting observation around `request_scope`
intentionally emits twice; no request-extension deduplication is provided.
Do not log cause contents or untrusted request fields. `operational_http` must
replace inbound header, Tower and adapter identities before observation and must
replace inner response IDs. Keep typed server correlation on completion events
when INFO spans are disabled. Retain test subscriber dispatches across cases.
Readiness defaults: Starting/Draining INFO; dependency failures while Ready and
Stopped WARN. Existing status-only probes and Problem JSON remain compatible.
Observation severity overrides are explicit response extensions, independent of
admission. Preserve actual status/outcome and the default WARN for dropped
futures. No application callback belongs in the observation guard's destructor.
Keep sanitized HTTP completion fields on the event independently of span filtering.
The all-targets test gate includes the example's live readiness tests and requires
loopback socket permission. Preserve both enabled-event and filtered-event assertions.
Select execution/event context once at first poll, retaining the HTTP span or its
available application parent. Never discover a fallback parent at Drop or record
HTTP fields into the application span. Handler panics propagate; an unwind before
a response is observed as dropped, with no invented status or logged panic payload.

## Common commands

Run from the workspace root:

```sh
cargo test -p batter-axum --locked
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
scripts/jig check test
```
