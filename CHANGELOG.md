# Changelog

## Backlog migration — 2026-09-08

Audited Markdown delivery requirements into dependency-linked Beads with completed
and deferred outcomes preserved. Removed duplicate roadmap/task lists; docs retain
contracts, capability facts and validation history.

## Unreleased

- Soften public status language now that the library is in internal use: drop
  MVP / not-production-validated framing while keeping unpublished-package,
  evidence-scope, and API-limit facts.

- Add `batter_axum::browser` with validated HTTPS/loopback origins,
  duplicate-aware opaque named-cookie reads, fixed host-only/root-path cookie
  set and removal, exact Origin/custom-marker/Fetch Metadata/JSON mutation
  signals, sanitized application-mappable rejections, and private-response
  headers. Every custom-marker policy automatically requires exact same-origin
  Fetch Metadata; an accepted marker name alone is not treated as browser or
  preflight provenance. Authentication, authorization, CORS, CSRF-token design,
  session persistence/revocation, and application error envelopes remain
  outside the adapter.
- Replace Axum-owned `ReadinessReason::Dependency(HealthStatus)`, which could
  represent a healthy dependency failure, with foundation-owned exhaustive
  classifications. `HealthStatus` now projects explicitly to
  `DependencyReadiness`; `ReadinessEvaluator` combines that result with lifecycle
  observation into `ReadinessDecision::Ready` or
  `Unready(ReadinessUnreadyReason)`.
  Axum carries the valid decision in response extensions and maps only transport
  status/severity. Migrate `ReadinessPolicy::reason()` to `decision()` and update
  `with_level` callbacks to accept `ReadinessDecision`. Replace
  `ReadinessReason::Ready` with `ReadinessDecision::Ready`, `.status()` with
  `readiness_status(decision)`, and `.level()` with
  `default_readiness_level(decision)`. Response-extension lookups must request
  `ReadinessDecision` and call `.unready_reason()` for its optional payload. The
  old `ReadinessReason` name is absent from both `batter_axum` and
  `batter::readiness`, so a stale typed extension lookup cannot be revived by
  changing only its import and then silently return `None`. Match
  `batter::readiness::ReadinessUnreadyReason` only inside an unready decision.
  No invalid or ambiguously named compatibility variant is retained.
- Replace clone-wide `ShutdownHandle::mark_ready` with one-shot readiness
  ownership. `Supervisor::start` and `run_until` consume approval automatically;
  only explicitly exceptional `start_unapproved` and `run_until_unapproved`
  paths withhold it. They return non-cloneable `UnapprovedSupervisor` and
  `UnapprovedDriver` owners; consume `approve_readiness()` to choose the normal
  transition. The caller-owned form remains movable after polling without
  separating the approval capability. Canonical
  `Startup` performs the same transition after successful initialization, while
  `without_readiness_approval()` changes its successful handoff type so deferred
  policy cannot be mistaken for an approved driver. Standalone lifecycles can
  explicitly construct a paired `ReadinessApproval`. Repeated approval, approval
  through shutdown/status/admission projections, and approval after transition
  no longer compile.
  Migration: callers that intentionally withheld approval must replace ordinary
  `start`/`run_until` with the corresponding explicitly unapproved path.
  `ShutdownHandle::new` and `Default` are removed: use `new_unapproved` only for
  a deliberately permanent Starting lifecycle, or construct the handle together
  with its one-shot approval for standalone admission.
- Split root lifecycle authority from consumer projections. `ShutdownHandle`
  now requests shutdown and constructs purpose-qualified views:
  `LifecycleStatus` for readiness/status, `OperationAdmission` for creating a
  readiness-gated downward-cancelled `OperationContext`, and `ShutdownSignal`
  for drain/cancellation observation. `RequestPolicy` now requires
  `OperationAdmission`; `ReadinessPolicy` and the status-only Axum readiness
  handler require `LifecycleStatus`. Migrate `handle.readiness()`,
  `handle.wait_ready()`, `handle.is_draining()`, and `handle.draining()` to
  `handle.status().readiness()`, `handle.status().wait_ready()`,
  `handle.status().is_draining()`, and `handle.signal().draining()` respectively;
  pass `handle.operation_admission()` to request policy. Raw `operation_token()`
  has no direct replacement: transient work uses `OperationAdmission::admit`
  while Ready, component drain work observes `ShutdownSignal::cancelled()`, and
  cleanup uses an independent bounded context. Managed supervision receives
  private coordinator authority directly and can no longer recover it through a
  read-only signal.
- Replace provenance-dependent `ShutdownSignal::mark_started` with a linear
  component-start boundary. `Supervisor::register` and constrained
  `Registration::register` factories now receive a non-cloneable
  `ComponentStartup`; consume `acknowledge_started()` after actual initialization
  and retain its returned observation-only `ShutdownSignal` while running.
  `HealthMonitor::run` no longer acknowledges a registered component: prefer
  `HealthMonitor::register_in`, or explicitly acknowledge the factory's
  `ComponentStartup` and pass the returned signal to `run`. Passing a clone from
  `startup.shutdown()` compiles but deliberately leaves readiness pending.
- Add an opt-in `RetryOptions` / `execute_with_options` boundary for per-attempt
  deadline caps and optional injected equal jitter. Attempt deadlines are
  recomputed after backoff, cannot exceed the input total/work context, and have
  a distinct non-exhaustive `RetryExecutionError` outcome. Legacy `execute`,
  `execute_with_jitter`, `RetryError`, `StopReason`, and `Interruption` remain
  source-compatible and keep their previous meanings. Options are consumed once
  and intentionally are not clonable, preventing accidental duplication of a
  stateful jitter stream across concurrent executions. The default sampler
  specialization implements `Default`, so `RetryOptions::default()` needs no
  sampler type annotation. Retry completion now reconciles cancellation of the
  public attempt scope before accepting a value or classifying an error, even
  when the factory cancels that scope and returns in the same poll. This applies
  consistently to the legacy and options entrypoints without changing their
  error-enum shapes or the meanings of existing variants. Composite attempt
  results are mapped at the operation-owned observation boundary so application
  errors remain WARN `failed`, cancellation remains INFO `cancelled`, and neither
  can be mislabeled as a successful attempt. An error returned in the same poll
  as terminal cancellation remains available as `last_error` without being
  classified or replayed.
- Hard-cut SQLx verification to compiled inputs. `AuthorityPolicyBuilder::build`
  now returns the only executable generic authority value, migration construction
  and mutation are fallible, and non-empty `VerificationPlan` constructors plus
  fallible cross-axis composition feed `verify`. This removes
  `VerificationPolicy`, `VerificationRequest`, `verify_request`, mutable executable
  policy fields, `CompiledExactRole::authority_policy`,
  `VerificationError::InvalidPolicy`, and
  `PolicyError::EmptyVerificationRequest`. Relation/sequence namespace conflicts,
  duplicate/contradictory declarations, capacities, and migration-ledger kind
  conflicts now fail during pure construction. Catalog-identity failures retain
  their typed `PolicyError` cause, and report coverage has canonical ordering.
  `MigrationPolicy` and `MigrationExpectation` fields are now private; use their
  fallible constructors, modifiers, and read-only accessors instead of struct
  literals or field access. Exhaustive matches must handle the new
  `PolicyError::{ConflictingRelationKind, InvalidObjectPrivilege,
  ContradictoryAuthorityPrivilege}` and
  `VerificationError::{CatalogIdentity, CatalogPolicyExpansion}` variants.
  Object-kind-invalid privileges now fail as `InvalidObjectPrivilege`, including
  invalid required privileges that previously surfaced as
  `ContradictoryRequiredPrivilege`; duplicate legacy PUBLIC privilege atoms with
  conflicting grant-option values now fail instead of being merged. Internal
  catalog-policy expansion now checks identities and appends normalized defaults
  during cooperative traversal instead of synchronously sorting and re-indexing
  the full generated policy in one executor poll.
- Replace the reference package's erased `ConfigMode`/`RootSettings` boundary
  with disjoint `ServingSettings` and `MaintenanceSettings`. Serving now follows
  the inert `runtime::prepare` to consuming `runtime::run` handoff, and the
  infallible router consumes `PreparedHttp`. Maintenance accepts only its
  database capability, ignores known serving-only environment values and has no
  serving conversion. Ambient `PG*` conflicts now fail as settings errors during
  preparation, before protected startup acquires resources.
- Replace blocking retained-panic inspection with `PanicPayload::try_inspect`;
  concurrent or recursive inspection now returns `PanicPayloadBusy`.
- Pin native runtime dependencies to pushed Git revision `d57ec6be61e9f00ccce373b19ca356cafe98f206`
  in both workspaces, removing the requirement for a sibling development checkout.
- Capture finite-command interruption at the final poll before future destruction.
  Distinguish native termination before initialization from process drain in
  managed reports, and correct the reference root's empty-registry description.
- Managed stop-control failures are published immediately, including later clock
  tightening while native settlement remains pending.
- The offline retirement CLI emits structured, redacted recovery facts and keeps
  the cancellation job ID, original refusal facts and cleanup disposition.
- Reference worker settings expose validated configuration rather than a native
  supervisor builder. The composition root prepares and transfers native work to
  managed registration; the configuration consumer test follows that ownership.
- Hard-cut the reference runtime startup error to
  `ProtectedRuntimeStartupFailure`, removing `RuntimeStartupFailure` rather than
  retaining a deprecated alias. Add `RuntimePoolCleanupFailure` with typed report
  access when an otherwise successful shutdown omits the required
  `postgres.pool` cleanup record.
- PostgreSQL lifecycle pool construction is now lazy and registers close ownership
  before return. Connectivity failures therefore identify the explicit
  `postgres.probe` stage rather than the former eager acquisition step.
- Reference process controls retain simultaneous assertion and shutdown errors,
  and executable TERM/INT cases require a signal-specific acknowledgement from a
  dedicated test binary plus the separate production binary's unchanged
  empty-stdout/fixed-stderr exit contract. The production entrypoint has no test
  signal mode, and the package explicitly retains that entrypoint as Cargo's
  default run target.

- Native runtime integration now accepts `PreparedSupervisor`, an owned validated
  launch value, instead of an arbitrary application closure. Registration rejection
  and dropping an unstarted process cannot start native work. Native preparation
  errors remain part of owned application startup and retain its cleanup report.

- Add `command::Command` for finite callbacks with independently owned LIFO
  finalization, concrete work results, retained panic/cleanup outcomes and optional
  absolute total reservation. Command cancellation stays below the parent and
  cannot cancel cleanup. Migrate the UDP finite-command example to this owner.
- Add adapter-facing managed component registration with library-owned native
  initialization acknowledgement, independently retained settlement and cleanup
  eligibility. Preserve reports after wrapper abortion, including native/future
  failures. Anchor process shutdown phases to one first-stop timestamp. Native
  adapter accepts inert native preparation and the reference root keeps readiness
  unapproved until the delivery handler exists. Remove production startup-control
  jobs, leases, owner epochs and reconciliation pools; preserve applied migrations.
  Add explicit offline retirement with identity/quiescence checks, scoped native
  cancellation and retained ambiguous outcomes. Linux acceptance includes the
  production-root HTTP probe and retirement command on both supported toolchains.
- Add `batter::settings` for explicit bounded sources, integer/duration parsing
  and redacted diagnostics. HTTP and reference roots own their schemas and native
  constructors.
- Add the optional `batter-sqlx` PostgreSQL lease, probe and pool-close adapter,
  plus unpublished postgres-lifecycle and reference-service example packages.
  Isolated fixtures are opt-in under `batter-sqlx/test-support`; provisioning stays
  in the external harness.
- Add owned `startup::Startup`, `health::HealthMonitor`, and Axum operational
  helpers (`operational_http`, `ReadinessPolicy`, `register_http`).
- Retain operation, HTTP and process-task tracing parents when diagnostic INFO
  spans are filtered.
- Add component-ownership comparisons and HTTP/1.1 transport/lifetime suites on
  the shared Unix process harness.
- Add the `finite_command` example; finite work and service startup have distinct
  owned lifecycle entrypoints.
- Add `ShutdownCause::FiniteTaskExit` for shutdown initiated by admitted finite
  task failures. `ComponentExit` remains specific to registered critical
  components. Downstream exhaustive matches must add the new variant; code that
  handled finite failures under `ComponentExit` must handle `FiniteTaskExit`.
- Remove `ShutdownHandle::observer`; obtain completion observers through
  `RunningSupervisor::observer` after `Supervisor::start` so every observer has
  an owned completion publisher.
- Split the virtual workspace into `batter`, `batter-axum`, `batter-sqlx`,
  independent generic test support, and two unpublished example packages. HTTP
  imports move to `batter_axum`; the old `axum` and `postgres-example` features
  are removed. Keep PostgreSQL provisioning external and retain Rust 1.94 for
  every package.
- Expose `batter::telemetry::with_current_dispatch` for adapter futures while
  preserving dispatch during polling and destruction. Keep the combined HTTP
  `RequestPolicy` readiness/deadline contract unchanged.
- Adopt Jig with locked core/all-feature/doctest gates and example-only SQLx
  database tooling disabled. Keep the existing two-toolchain and HTTP checks.
- Move shared example support to `support.rs`, extract HTTP event assertions,
  and isolate cleanup log-capture tests without changing failure assertions.
- Exclude transient Jig backups from static package link inspection.
- Select the official Jig v0.3.0 release and document revision-preserving updates.
- Fix manual CI file-budget comparisons, cache the selected Jig runtime, and
  surface conflicting Markdown plan edits. Add CI/cache/merge/archive regressions.
- Retain Rust/ExecPlan tooling and one Jig policy workflow alongside the
  existing Rust matrix. Remove unused
  frontend/proxy settings, duplicate workflows, and the checkout helper.
- Include Jig's launcher and durable records in source archives while excluding
  local runtime/cache data and the deprecated adoption receipt.

- Enable workspace Clippy complexity/length limits of 20/100. Extract private
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
- Optional adapter live cases have Linux
  evidence; the historical 58-entry reference inventory and separate maintenance
  probe passed on both supported toolchains. The current 64-entry inventory has
  five Linux-executed offline entries; its 59 database probes remain unexecuted.

## 0.1.0 — Initial source snapshot — 2026-09-07

Added critical-task lifecycle supervision, explicit phased shutdown/readiness,
LIFO asynchronous cleanup reports, deadline/cancellation contexts, opted-in
bounded retry policy, process-local concurrency admission, minimal tracing,
optional Axum middleware/probes, and a small test-support crate.

Added worker, HTTP, and native SQLx lifecycle examples; 67 authored failure-contract
tests; verification/packaging scripts; CI definition; architecture decisions;
Effect v4 design rationale; integration boundaries and a prioritized agent backlog.

Validation limitation: no Rust toolchain or dependency resolution was available
in the authoring environment. Compilation, tests, formatting, lints, documentation
builds, live examples, and MSRV compatibility remain unverified. Packages remained unpublished.
