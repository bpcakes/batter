# Prioritized implementation roadmap

This is a backlog, not a promise that the following features exist. Each task
must update implementation status, tests, and validation evidence when complete.
The [Effect v4 reconciliation](effect-v4-reconciliation.md) groups these items
into a complete reference application path. It preserves the existing ownership
decisions and identifies missing conventions without adding a second framework.

## BTR-001: Validate the source snapshot

**Priority P0. Status LOCALLY VALIDATED; initial commit created; publication not authorized.**

2026-09-07: the original graph passed on Rust 1.88.0 after two diagnostic fixes.
All nine direct dependency requirements were then updated to their latest stable
releases, including SQLx 0.9.0. The minimum is now 1.94; the default toolchain is
1.98.1. The upgraded graph passes the full verification matrix on both versions.
HTTP SIGTERM smoke and exact evidence are recorded in [validation](validation.md).
The initial snapshot was committed as `3e64cb2` at the owner's request.
Further commits and publication require a separate request.
Hosted CI has not been executed.

2026-09-08: enable workspace Clippy cognitive-complexity (20) and function-length
(100) limits. Extract private cleanup/shutdown helpers and retain existing
failure assertions; add a regression for shutdown-future lifetime through cleanup
following component failure. Validation is recorded in [validation](validation.md).

Jig adoption verification: remove inferred database gates for the optional SQLx
example, run locked core/all-feature/doctests through Jig, satisfy the 100-line
and module-layout lints, and isolate cleanup log-capture tests. Local checks and
remaining limitations are recorded in [validation](validation.md).
The commit-readiness audit selects the official Jig v0.3.0 release, removes
unused language configuration and duplicate CI/helper files, and checks fresh
installation, MCP startup, and source-archive contents. Use `update --recopy` to
retain the release. Future adoption can use a persistent version pin once Jig
supports it; harness updates must preserve the repository customizations.
Review follow-up fixes manual CI comparison against `origin/master`, caches the
installed runtime, and restores conflict reporting for edited Markdown plans.
Seven standard-library Python regression tests exercise the CI, cache, merge,
and archive boundaries. Hosted CI/cache execution remains external evidence.

Scope: run scripts/verify.sh --bootstrap on 1.88.0, fix compiler/lint/test issues,
commit rustfmt output and a real Cargo.lock, then verify stable. Run the HTTP
process smoke. Review the generated dependency graph and target/MSRV behavior.

Acceptance: core-only and all-feature builds pass, all authored tests and
rustdoc tests pass or any real defect is fixed with an explicit regression test,
Clippy/rustdoc warnings are resolved, HTTP SIGTERM smoke succeeds, exact commands
and versions are recorded, and no unverified claim is silently promoted.
Do not disable failure-path assertions merely to obtain a green run.

Files: all source as required, Cargo.lock, docs/validation.md, CI, changelog.
No new framework abstractions until this gate is closed.

## BTR-002: Separate adoption packages

**Priority P0. Status IMPLEMENTED; validation recorded separately.**

Use a virtual workspace with `batter`, `batter-axum`, generic
`batter-test-support`, and the unpublished `batter-example-postgres-lifecycle`
executable. Move core/HTTP tests and examples with their owners. Remove the old
foundation adapter/example features; preserve the combined HTTP readiness and
deadline behavior. Expose a tested dispatch-preservation future for adapters
while keeping pin/drop machinery private. Retain Rust 1.94 and disabled
publication explicitly in each manifest.

Acceptance: foundation-only dependencies exclude Axum/SQLx/test utilities;
generic test support stays independent; existing failure contracts and new
dispatch/budget-boundary regressions pass; both Rust toolchains, all package
targets, docs, and HTTP smoke pass. Update package guides, commands, source
links, and Jig/CI wiring. The PostgreSQL harness stays external and unchanged;
no new upstream integration or live database claim is part of this work.
See [ADR-006](adr/006-workspace-packages.md) and [validation](validation.md).

## BTR-010: Harden lifecycle and cancellation under real scheduling

**Priority P0 before production. Status PARTIAL. Depends on BTR-001.**

Implemented: completion-time early-exit classification; deterministic startup,
coordinator-drop and cleanup-close-drop tests; startup acknowledgements;
multi-thread bounded-admission/drain regression. HTTP smoke now covers SIGTERM,
SIGINT, middleware deadlines, application error envelopes, and default telemetry.
Completed-but-unobserved results no longer become false abort targets; unpolled
caller-owned driver drops signal shutdown. Every skipped finalizer is reported
and logged, with captured values dropped in LIFO order.
Non-yielding child-process and full transport/hidden-descendant cases remain open.

Scope: coordinator-drop/cleanup-close-drop tests; racing cancellation/completion;
non-yielding tasks isolated in child processes; panic/abort of a component with
internally owned children; readiness startup acknowledgement; actual HTTP
keep-alive, slow-body, disconnect, streaming, and shutdown behavior.

Acceptance: no false all-children-joined guarantee; emergency drops do not detach
directly owned tasks silently; unjoined/unsafe cleanup paths remain observable;
non-yielding tests cannot hang the runner; the component contract states exactly
what startup and transitive termination have been acknowledged. Document whether
streaming gets an explicit adapter or stays excluded. Retain separate drain and
forced-cancel phases. Keep the conservative teardown rule unless a stronger
protocol has been proven and recorded in an ADR.

Reconciliation follow-up: decide whether the reference HTTP application needs an
opt-in panic boundary using existing Tower facilities. No catcher exists today.
If added, test sanitized responses for unwinding handler panics and middleware
ordering; keep aborting panics, default hook output, shared-state recovery, and
body panics after headers outside the conversion guarantee. Do not translate
panics into expected/retryable domain failures.

Files: lifecycle/cleanup/operation/http tests and modules, guarantees, operations.

## BTR-011: Finite process ownership and separately driven shutdown

**Priority P0. Status IMPLEMENTED; validation recorded separately.**

Bound queued plus executing work, keep permits with work rather than receipts,
linearize root admission with drain, allow only active bounded descendants during
drain, and close admission at forced cancellation/failure. Retain typed task
failures; treat normal business denial as a value. Finite successes use a counter.
Start an owned driver whose completion observers cannot cancel cleanup; preserve
coordinator JoinError as well as successful reports. Tests and the process-owned
example cover lost request/shutdown waiters and startup/owner-drop behavior.

## BTR-020: Native SQLx -> Runledger reference path

**Priority P1. Status NOT IMPLEMENTED. Depends on BTR-001; coordinates BTR-022.**

Scope: add one small reference service with a validated business command, native
SQLx transaction, transactional Runledger submission, a registered real handler,
a hosted upstream supervisor, and a readable durable result. Inspect actual
upstream versions/APIs before writing the adapter; do not guess from this brief.

Acceptance: rollback removes both application write and job submission; same-key
same-payload submission deduplicates; mismatched payload conflicts remain typed;
worker startup/shutdown failures reach the process report; root/worker budgets
are aligned; job attempts use a fresh durable execution context; request expiry
does not cancel committed durable work; at-least-once external effects are
explicit; all examples and real-PostgreSQL tests run in CI.

Do not create an outbox, scheduler, queue, DAG engine, lease system, or generic
transaction retry abstraction in Batter. Preserve original upstream errors and
ambiguous commit outcomes. Prefer an example/local adapter first; extract only
common mechanics shared by a second consumer.

## BTR-021: Runlimit HTTP and observation integration

**Priority P1. Status NOT IMPLEMENTED. Depends on BTR-001.**

Scope: connect upstream runlimit-axum to one application router and bridge its
observer events into Batter's observability conventions. Keep identity extraction
and trust policy in the application. Preserve existing decisions and semantics.

Acceptance: enforced/shadow denial, capacity exhaustion, backend failure, and
consumption certainty remain distinct; no raw subject/key enters telemetry;
upstream admission precedes body consumption where intended; internal retry does
not double-charge user admission by default; middleware order is explicit;
backend failures do not silently turn fail-open. Add compile/runtime tests against
the pinned upstream release and a second-consumer evaluation before generalizing.

## BTR-022: postgres-test-harness application fixture adapter

**Priority P1. Status NOT IMPLEMENTED. Depends on BTR-001.**

Scope: an application/example test fixture providing native SQLx setup and teardown
around the external harness's templates/leases. Keep it outside the generic
`batter-test-support` leaf crate. Cache harness/template per test
process; fingerprint ordered application and dependency migrations and behavior
revisions; support existing-server and owned-container modes through upstream.

Acceptance: tests have isolated databases; templates cannot become stale after
migration/setup changes; pool limits respect the harness budget; pools close
before lease cleanup; body, close, lease, and deferred-drain failures all survive;
empty-database migration tests remain separate; missing prerequisites cannot
silently pass; no copied provisioning or unsafe name-prefix cleanup appears.
No harness relocation or new fixture package is authorized by the workspace
reorganization. Extract shared fixture mechanics only after actual reuse.

## BTR-030: Cross-boundary telemetry and metrics

**Priority P1. Status PARTIAL: useful local tracing; no metrics/exporters. Depends on BTR-001;
durable correlation depends on BTR-020.**

Scope: optional exporter/metric adapters, versioned safe correlation envelopes,
request-to-job trace links, stable outcome/attempt/cleanup metrics, ordered exporter
flush. Keep the application in charge of subscriber installation.

Start the correlation portion with BTR-020's reference path. Define typed trusted
request metadata separately from OperationContext and application dependencies.
Use explicit extensions/arguments first; task-local access needs a concrete
consumer and documented absence/spawn behavior. The HTTP request-ID example is
not an ambient tenant/principal context or distributed propagation adapter.

Implemented local baseline: operation completion visible with an ordinary INFO
subscriber, separate HTTP status/outcome/latency, cleanup span context, scoped
subscriber propagation, and tests excluding raw error/request contents.
Scoped dispatch now covers owned-future destruction as well as polling, including
actual task abort and nested HTTP span destruction under another subscriber.

Acceptance: trace continuity survives a durable boundary without persisting
CancellationToken/Instant; labels have a documented finite cardinality budget;
redaction tests reject subjects and source messages in automatic outputs; startup,
shutdown, cancellation, uncertain commit, and skipped-cleanup telemetry are distinct;
exporter failure is observed without changing admission or application results.
Account for panic-hook output separately rather than claiming tracing sanitizes it.
Prove concurrent requests cannot exchange metadata, untrusted headers cannot
become authority, worker authorization is explicit, and a committed job outlives
request cancellation. Persist only a validated, versioned durable representation.

## BTR-040: Fleet-safe retry policy extensions

**Priority P1 before multi-replica retry adoption. Status PARTIAL.**

Implemented: caller-sampled equal jitter and explicit operation-finalization
reserves, with deterministic boundary/provider-floor/composition tests. Existing
retry execution remains deterministic unless jitter is selected. Per-attempt
deadlines, retry tokens, and provider parsing adapters remain unimplemented.

Scope: injected/testable jitter; optional per-attempt deadline capped by remaining
total budget; explicit retry-token budgets; provider Retry-After parsing adapters.
Consider fallback/circuits only for a concrete dependency with measured need.

Acceptance: deterministic tests can assert delay ranges without real sleeps;
provider lower bounds are never shortened by jitter; no attempt resets the total
budget; timeout/uncertain mutation replay still requires an explicit protocol;
last errors and attempt counts survive interruption; generated delays cannot
overflow or spin; a retry budget is not conflated with user quotas/concurrency.
Retain one retry owner. Do not add an automatic universal is_retryable trait.
Prove adapter composition does not multiply attempts or wrap upstream durable
rescheduling in a second loop; any handler timing translation requires a checked
upstream API. Shared policy mechanics do not authorize transaction replay or
resolve uncertain commit outcomes. No replacement with backon is required.

## BTR-050: Schema and API contract coherence

**Priority P2. Status NOT IMPLEMENTED. Depends on a real reference API.**

Scope: validated input/newtypes, consistent domain-error mapping, OpenAPI/client
integration using existing Rust ecosystem facilities first.

Prove the first recipe in BTR-020's reference application. Keep concrete domain
errors and map them explicitly into the same sanitized envelope used for
infrastructure failures. Basic Problem JSON exists; a universal Error or Cause
type is not required. Document stable problem types/codes and validation errors;
select/version-check extractor and schema libraries during implementation.

Acceptance: invalid boundaries are rejected; generated contract/client round-trip
checks run; internal causes never serialize accidentally; database row/domain/
request/response types remain separate when their meaning differs; no bespoke
endpoint macro language is introduced without a demonstrated unmet requirement.
Check response status/body agreement and route coverage in the generated spec,
including malformed/invalid input and declared error responses. Promote reusable
boundary types only after the example establishes their semantics.

## BTR-060: Configuration and secret handling

**Priority P2. Status NOT IMPLEMENTED beyond argument validation/example parsing.**

Scope: typed configuration loading at the composition root, explicit source
precedence, redacted secret containers, actionable startup diagnostics, safe test
overrides. Choose existing facilities over a second config framework.

Acceptance: invalid/unknown fields fail intentionally; secrets never appear in
Debug/telemetry/error bodies; environment mutation is not used unsafely in
concurrent tests; source precedence is tested; global mutable configuration and
hidden dependency lookup are excluded. Do not automatically treat serde as
business validation.
Compose configuration in the reference root using upstream public options;
do not require upstream libraries to adopt Batter's loader or depend on Batter.
Keep test overrides explicit and register exporter/pool cleanup after acquisition.

## BTR-070: Only extract proven repeated mechanics

**Priority deferred. Status intentionally NOT IMPLEMENTED.**

Candidates: request batching/deduplication, cache stampede protection, bounded
queues, construction graphs, and lifecycle-managed service replacement. Require
two actual consumers, shared failure semantics, and an ADR before adding them.
No cache invalidation or generic DI subsystem is justified by matching Effect's
feature list. Delete convenience wrappers that add vocabulary without strengthening
an invariant.

## Publication gate

No publication is authorized by this package. Before any public release, review
name ownership/availability, license/attribution, threat model, dependency audit,
API ergonomics in two consumers, MSRV policy, locked CI evidence, and outstanding
P0 items. Remove publish=false only after a separate owner decision.
