# Separate HTTP observation from admission

Owning Bead: `batter-faj.1`. Baseline: `19aa9ff8e72995add6d75fd65d4c7790ebafe872` (clean checkout at start). Beads retains scope and acceptance.

## Progress

- [x] Confirm the Bead, clean baseline and existing middleware/test contracts.
- [x] Extract additive middleware APIs and migrate the runnable composition.
- [x] Exercise complete-router observations, interruption, rendering, redaction and destruction.
- [x] Update contracts and execute both toolchain matrices, HTTP smoke and Jig gates.
- [x] Record final evidence for task and plan closure.

## Surprises & Discoveries

The investigation already established that Router::layer only covers previously assembled routes, and a service wrapper outside routing lacks MatchedPath at entry. Existing legacy tests remain intact; their scoped-subscriber capture fixture is now shared with the new suite. The first new-test compile exposed mark_ready returning bool while request returns unit; the fixture now discards that bool explicitly. No library semantics changed for that fix.

Focused validation passed 28 adapter tests and the new composition doctest on Rust 1.98.1. Clippy passed with warnings denied. Two temporary mutations were rejected by exact event counts: removing standalone observation and making admission call the combined wrapper. Source was restored byte-for-byte. Logs: `.agent/tmp/batter-faj.1/mutation-missing-observation.log` and `mutation-duplicate-observation.log`.

## Decision Log

Keep RequestPolicy's readiness/deadline policy intact. Add `observe_http` and `request_admission`; keep `request_scope` as the combined compatibility entry point. Share a private observation helper and private admission implementation. Each installed observer owns one event; nesting the legacy wrapper under observe_http deliberately yields two and is documented rather than hidden behind request-extension deduplication. No new dependencies or public observation guard are needed. The process smoke now checks one correlated HTTP event per tested request, including probes and unmatched fallback, so the runnable composition cannot silently lose their observation while unit fixtures pass.

## Outcomes & Retrospective

Implementation and focused tests are complete. Both Rust 1.98.1 and 1.94.0 matrices passed (374 test/doctest executions each, zero failed/ignored), including formatting, Clippy and rustdoc. HTTP was rebuilt with 1.98.1 and all three enhanced smoke modes passed. Logs and execution limits are recorded in docs/validation.md. All five targets in `scripts/jig work check --plan-id plan_01M21E97EVPQTVPJ6DDC79SR40` passed. Both work evidence and work gates reported fresh/passed; verify gate receipt `receipt_01M21F9R0A9A26Z26VB3FF8SJG`. The final `scripts/jig check test` exited 0 (api:test). Additional logs: `.agent/tmp/batter-faj.1/jig-work-check.log` and `jig-final-test.log`. Recording final documentation makes all five Jig target input digests stale, including the backend test receipt. Refresh the configured work check and final backend command after this last documentation edit; their logs retain the final results. Then close this plan and the owning Bead through their CLIs without another source/documentation edit. Linux execution is established, macOS/hosted execution of the change is unverified, and Cargo.lock is unchanged.

## Context and orientation

`crates/batter-axum/src/lib.rs` owns middleware and response rendering. At the baseline, private HttpObservation surrounded the admission body in request_scope_inner; the implementation now shares observe_response and request_admission_inner. `batter::telemetry::with_current_dispatch` protects polling and full future destruction. Operations retain their own separate events. Existing tests are `tests/http.rs`, `tests/telemetry.rs` and `tests/scoped_dispatch.rs`; the runnable root is `examples/http_service.rs`. Streaming bodies remain outside the observed and deadline lifetimes.

## Plan of work

Move the admission body into a private async function. Both public admission and observation entry points capture dispatch inside their async bodies, preserving first-poll semantics. The combined wrapper invokes the same observation and admission helpers. The private observation helper creates its guard inside the protected future and instruments its inner future before polling.

Add public rustdoc with a compiling composition example. Assemble guarded routes with request_admission, merge probes and fallback, then apply observe_http and application-owned server identity outermost. Update the runnable example's identity wrapper to retain destruction context too.

Keep all legacy failure tests; add composition tests checking event counts and fields for healthy/unhealthy probes, fallback, method rejection, application failures and short-circuit responses during startup/ready/drain. Exercise timeout/forced cancellation with custom rendering and retained original metadata. Extend destruction evidence across observation-only, split and combined modes, including inert never-polled futures and dropping a response body after completion. Assert redaction and layer placement directly.

## Concrete steps and validation

Run focused `cargo test -p batter-axum --locked`, formatting and Clippy as changes settle. Recheck resolved Axum 0.8.9 source and primary documentation; record semantics in docs/references.md. Update README, package guide, docs/guarantees.md, integrations.md, status.md, testing.md, usage.md and validation.md as needed.

Run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`. Build `cargo build -p batter-axum --example http_service --locked`, then `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` in default, `--signal SIGINT` and `--deadline` modes. Run `scripts/jig work check --plan-id plan_01M21E97EVPQTVPJ6DDC79SR40`, inspect evidence/gates, and finish backend verification with `scripts/jig check test`. Capture logs under ignored `.agent/tmp/batter-faj.1/`. Success requires all relevant checks, exact event assertions and unchanged Cargo.lock. Record actual platform evidence; do not infer macOS/hosted execution.

## Idempotence and recovery

Edits are additive and local; no database, publication, commit or downstream dependency update is required. Preserve any concurrent work. Fix failed checks without relaxing semantic assertions. Re-run affected checks after fixes and update this plan's evidence. Tracker and Jig state remain append-only through their CLIs.

## Interfaces and dependencies

New `pub async fn observe_http(Request, Next) -> Response` is stateless. New `pub async fn request_admission(State<RequestPolicy>, Request, Next) -> Response` enforces the existing policy without HTTP observations. Existing request_scope preserves its signature and behavior. Existing Rust 1.94 minimum and dependency graph remain unchanged.
