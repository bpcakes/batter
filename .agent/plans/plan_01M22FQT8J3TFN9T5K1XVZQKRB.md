# Preserve HTTP completion fields when spans are filtered

Owning Bead: batter-faj.3. A WARN completion currently loses all HTTP facts when INFO spans are disabled. Keep those facts on the event itself without changing the middleware API or its lifetime.

## Progress

- [x] Inspect original implementation, reviewer reproduction, current tests and repository guidance.
- [x] Add regression tests that read event fields independently of spans; demonstrate failure before the fix.
- [x] Retain sanitized method/route in the observer and emit complete fields at every severity.
- [x] Update contracts/status and remove duplicated aggregate inventories from the testing guide.
- [x] Run both supported toolchain verification scripts and all HTTP smoke modes; record evidence.
- [x] Run required Jig gates and final backend tests.

## Surprises & Discoveries

- Existing severity tests enable TRACE and thereby keep the INFO span, masking the field dependency. All three new direct-event tests failed against the old implementation with only a message present, then passed after the fix. The initial test build required an explicit StatusCode return on the unreachable handler; that compile failure is not counted as regression evidence.
- Axum 0.8.9 MatchedPath is cloneable and retains only the route template; tracing-core 0.1.36 Option values omit absent fields.

## Decision Log

- Keep the INFO span for existing nested context, and retain completion facts independently. Do not make observation depend on span enablement.
- Use native event fields and the existing static level callsites. No new public API, dependency or callback during destruction.
- Keep historical execution counts in docs/validation.md; the testing guide describes behaviors and commands.

## Outcomes & Retrospective

Private observer changes and three regression tests are implemented. Each supported toolchain passed 383 test/doctest executions plus format/check/Clippy/rustdoc; all three standard HTTP smoke modes and the live WARN-filter reproduction passed. The original network attempt was denied by the sandbox before server launch and was rerun with approval. All five required Jig targets passed with fresh evidence, and the final scripts/jig check test passed. Evidence is in docs/validation.md and .agent/tmp/batter-faj.3/. No public API or dependency change was required. macOS and hosted execution of this change remain unverified.

## Context and orientation

crates/batter-axum/src/lib.rs owns HttpObservation and shared standalone/combined observation. Tests under tests/observation use scoped subscribers and Router::oneshot. tests/support/capture.rs provides the existing formatted capture; new structured assertions must inspect event.record directly so span formatting cannot satisfy them accidentally.

## Plan of work

Add a narrow event-field capture for filtering regressions. Check completed WARN/ERROR events with INFO spans disabled and other levels with spans enabled, admission rejection, unmatched routes and destruction under a different ambient subscriber. Preserve the existing complete-router and lifetime tests. Retain normalized method and cloned MatchedPath in HttpObservation; emit optional numeric status, outcome and latency alongside method and route. Preserve span recording and dispatch protection.

## Concrete steps

Run cargo test -p batter-axum --test observation --locked before and after the implementation, retaining the expected failing regression log. Update docs/guarantees.md, the adapter rustdoc/README, docs/status.md, docs/testing.md and primary source evidence. Run bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh; build the HTTP example and execute scripts/smoke_http.py in default, SIGINT and deadline modes. Run scripts/jig work check with this plan ID, inspect evidence/gates and finish backend validation with scripts/jig check test.

## Validation and acceptance

Enabled completion events carry normalized method, route template or unmatched marker, actual numeric status when a response exists, bounded outcome and finite nonnegative latency even when the INFO span is disabled. A dropped future has no fabricated status and retains its first-poll dispatcher. One completion per installed observer and existing redaction/response behavior remain intact. Record command results, toolchains, lock hash and platform limits in docs/validation.md.

## Idempotence and recovery

Preserve the pre-existing uncommitted implementation. Logs are ignored under .agent/tmp/batter-faj.3. Fix failures without weakening semantic assertions. Do not commit or publish. If required sandboxed verification fails because of permissions, rerun with the required approval rather than changing its behavior.

## Interfaces and dependencies

All changes are private to the adapter and its tests/documentation. Existing public middleware and HttpObservationLevel behavior are preserved. Cargo.lock remains Cargo-owned.
