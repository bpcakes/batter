# Changelog

## Unreleased

- Enable workspace Clippy complexity/length limits of 20/250. Extract private
  cleanup/shutdown helpers while preserving behavior, including shutdown-future
  ownership through cleanup after component failure.

- Distinguish completed-but-unobserved tasks from abort targets, and arm emergency
  shutdown before a caller-owned driver is first polled.
- Preserve scoped tracing during future destruction as well as polling, including
  nested HTTP spans and task abort; no downstream wrapper is required.
- Report and log every skipped cleanup hook through one LIFO path, including
  the first hook skipped after budget exhaustion.
- Fix critical early-exit classification racing with drain; require explicit
  startup acknowledgements before publishing readiness.
- Add bounded finite process-owned tasks and a separately driven shutdown
  coordinator with repeatable, cancellation-safe completion observation.
- Add application-controlled HTTP infrastructure rendering and default-visible
  operation/HTTP/cleanup observations with sanitized fields and scoped tracing.
- Add explicit finalization reserves and caller-sampled deterministic-testable
  jitter, preserving total deadlines and provider lower bounds.
- Add process-owned and operation-budget examples and startup/drop/race,
  rendering/redaction, reserve/jitter regressions; extend local HTTP smoke to
  SIGINT, error envelopes, deadlines, and ordinary INFO telemetry.
- Upgrade all nine direct Rust dependencies to their latest stable releases and
  refresh Cargo.lock; see [exact versions](docs/references.md#dependency-refresh-2026-09-07).
- Pin the default Rust toolchain to 1.98.1. Raise the workspace minimum to 1.94
  for SQLx 0.9.0 and update the CI matrix to 1.94.0, 1.98.1, and stable.
- Normalize rustfmt output, remove a redundant cleanup-test closure, and escape
  a generic type in HTTP rustdoc. Failure-path assertions and runtime contracts
  remain unchanged.
- Record local verification and HTTP SIGTERM smoke results in
  [validation](docs/validation.md); live PostgreSQL remains untested.

## 0.1.0 — MVP source snapshot — 2026-09-07

Added critical-task lifecycle supervision, explicit phased shutdown/readiness,
LIFO asynchronous cleanup reports, deadline/cancellation contexts, opted-in
bounded retry policy, process-local concurrency admission, minimal tracing,
optional Axum middleware/probes, and a small test-support crate.

Added worker, HTTP, and native SQLx lifecycle examples; 67 authored failure-contract
tests; verification/packaging scripts; CI definition; architecture decisions;
Effect v4 design rationale; integration boundaries and a prioritized agent backlog.

Validation limitation: no Rust toolchain or dependency resolution was available
in the authoring environment. Compilation, tests, formatting, lints, documentation
builds, live examples, and MSRV compatibility remain unverified. See
[validation](docs/validation.md). This is not a published or production-validated release.
