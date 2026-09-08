# Agent handoff: batter

## Mission and current evidence

Build an operational foundation around native Rust/Tokio, not an Effect port,
DI container, ORM, or application framework. Axum is an optional adapter.

This snapshot contains implementation, 123 failure-contract tests plus one
doctest, and five examples.
The original authoring environment had no Rust toolchain. Subsequent local
verification passed on Rust 1.94.0 and 1.98.1 after upgrading the dependencies;
`docs/validation.md` records exact commands, versions, outcomes, and limitations.
Cargo.lock was refreshed by Cargo. Both workspace packages have publishing disabled.

## Reading order

1. [README](README.md), [status](docs/status.md), [guarantees](docs/guarantees.md).
2. [Architecture](docs/architecture.md) and [ADRs](docs/adr/README.md).
3. [Testing](docs/testing.md) and [validation](docs/validation.md).
4. [Integration contracts](docs/integrations.md), [roadmap](docs/roadmap.md), and
   [Effect v4 rationale](docs/effect-v4-brief.md).

## Verification: BTR-001

The default toolchain is pinned to Rust 1.98.1. The workspace minimum is Rust
1.94 because SQLx 0.9.0 requires it. Run `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`, then build and execute the HTTP
smoke test described in `docs/testing.md`. Repair failures without relaxing
semantic tests. Update validation evidence when the code or dependency graph changes.

The original Rust 1.88.0 bootstrap gate was executed before the dependency
upgrade. It is historical evidence, not a supported target for the new graph.
The checkout is now under Git; committing or publishing requires an explicit user
request. An included CI
definition is not evidence of a hosted CI execution.

## Code map

`src/lifecycle.rs` and `src/lifecycle/` own critical/finite process tasks, readiness
acknowledgements, shutdown phases, and the separately driven completion report.
`src/cleanup.rs` owns explicit LIFO finalizers.
`src/operation.rs` owns deadline/cancellation boundaries and typed failures.
`src/retry.rs` owns replay policy, bounded attempts, and backoff.
`src/admission.rs` owns process-local concurrency permits.
`src/telemetry.rs` records outcomes without printing error contents.
`src/scoped_dispatch.rs` privately retains tracing dispatch through polling and
full inner-future destruction, without heap allocation.
`src/http.rs` adapts Axum and is behind the `axum` feature.
`crates/batter-test-support` contains dependency scripts and error combination.
`examples` contains worker, HTTP, and native SQLx composition roots.

## Preserve these invariants

A child's deadline cannot exceed its parent's. Cancellation travels downward,
not upward. A scope finishing cancels its child token but cannot claim to have
joined arbitrary spawned tasks. A never-polled factory does no application work.

Never retry automatically because an error string sounds transient. The caller
must authorize replay and classify a returned failure. Timeout, cancellation,
and panic are not expected retryable errors. Provider delay is a lower bound.
The initial call counts against max_attempts; backoff consumes the total budget.

Draining stops admission before cancelling already admitted work. Every direct
critical task exit must be observed, including early Ok and panics. Joining a
task is not proof that its detached descendants stopped. Do not replace the
current conservative cleanup skipping with "abort then call it clean".

Root finite admission is bounded and linearized with drain. Only an active
process scope can admit bounded descendants during drain; forced cancellation
closes both paths. A receipt is a waiter, not a work owner. Task-level errors
initiate drain; normal business denials belong in successful task values.
Readiness requires application approval, a running driver, and acknowledgement
from every registered critical component after its actual initialization.
Owned-driver waiter cancellation must not cancel cleanup. No runtime-death or
general async-drop guarantee is implied.
Caller-owned driver emergency cancellation is armed at ownership transfer, even
before first poll. Unobserved task results are not evidence of running work;
harvest ready results before escalation and abort only unfinished tasks.

Preserve all task and cleanup errors in reports. Do not log their Debug/Display
contents automatically; they may contain secrets. Reports are internal objects,
not HTTP payloads. Domain errors stay concrete. No global tracing subscriber or
panic-hook installation belongs in the library.
Owned-future dispatch context must also cover destruction, not just polling.
Keep observations and nested instrumented spans inside the protected future;
wrapping only a destructor's event leaves span destruction unprotected.

Finalizers register after successful acquisition, run in reverse dependency
order, and require explicit awaiting. Do not add an `async fn Drop` fiction or a
bracket helper that skips finalization when its outer future is cancelled.

## Dependency direction

Applications -> batter adapters -> native ecosystem libraries. Runlimit,
Runledger, and postgres-test-harness must not depend on batter. Keep their own
supervision, persistence, policy validation, and provisioning responsibilities.
Never implement a second job queue, workflow engine, outbox, limiter storage
engine, database harness, or repository abstraction here.

## Changes must include

Use generic scenario names in source, tests, examples, documentation, and updates.
Do not use application-specific project names or project-shaped labels.

Update the relevant contract, failure-path test, status row, and roadmap item.
New public APIs need rustdoc and an example. New claims need executable tests
or an explicit unverified label. Record external semantics against primary
sources in `docs/references.md`; re-check the actual upstream version before
implementing an integration. Research Effect v4, not v3 tutorials.

Do not hide body streaming, transaction commit ambiguity, non-yielding tasks,
unbounded semaphore waiters, or default panic-hook output behind generic words
like "safe" or "reliable". Test what those words would actually mean.

## Next priorities after validation

BTR-010: cancellation/shutdown race and real-transport hardening.
BTR-020: one native SQLx transaction -> Runledger -> worker reference path,
with postgres-test-harness isolation and rollback/idempotency tests.
BTR-021: thin Runlimit admission/observation integration.
BTR-030: bounded-cardinality metrics and request-to-job trace context.
BTR-040: per-attempt budgets and retry-token policy beyond injected jitter.

Detailed scopes, dependencies, and acceptance criteria are in the roadmap.
Publication and deployment require a separate user decision; do not infer it
from a request to implement or test this ZIP.

<!-- BEGIN JIG MANAGED BLOCK -->
This repository uses the shared `jig.sh` workflow. Keep repo-local business rules and ownership guidance in backend-level guides; keep generic agent workflow and repo policy here.

## Start Here

- Use this file for repo-wide defaults.
- Open [agent-map.md](./agent-map.md) before backend work.
- Read the nearest backend-level `AGENTS.md` before changing a package or crate when one exists.
- Use `.agent/PLANS.md` when writing an ExecPlan for a complex feature or refactor.
- Use `scripts/jig` for the typed repo contract and `scripts/jig mcp` for MCP clients.
- On a fresh machine, run `scripts/jig doctor`; follow its next step, including `scripts/jig agent bootstrap` when Jig Codex skills are missing.
- For substantial work, use `scripts/jig work start`, `scripts/jig work check`, `scripts/jig work evidence`, `scripts/jig work gates`, and `scripts/jig work finish` to keep plans, receipts, and required gates connected.
- A plan captures an exact Git baseline. Default `work check` runs required gates whose configured path policy applies and records explicit not-applicable evidence for the rest; use `--gate <id>` only when deliberately force-running one gate.
- `jig-contract` validates Jig harness wiring, not the application's API contract.
- Treat `.agent/state/*.jsonl` as append-only repo memory.

## Compatibility And Cutovers

- Prefer direct cutovers only for internal code-only changes that can ship in one coordinated deploy.
- Preserve compatibility or stage rollouts for persisted database state, queued job types, public API contracts, bookmarked routes, webhook boundaries, or source-of-truth moves that can straddle deploys.

- Never overwrite an existing database migration; add a new forward-only migration instead.

## Backend Defaults

- Treat `.`, `crates` as Rust crate roots.
- Add crate-level `AGENTS.md` files when a crate has meaningful ownership, entrypoint, or invariant guidance that should travel with that crate.

- Keep transport logic thin and business logic in the owning crate.

- Keep transaction boundaries explicit and deterministic.

## Frontend Defaults

No web apps or development proxy are configured in `.jig.toml`.

Jig database tooling is disabled: SQLx is currently an optional example only.

## Preferred Commands

- `scripts/jig bootstrap`
- `scripts/jig doctor`

- `scripts/jig check test`
- `scripts/jig check fmt`

- `scripts/jig check clippy`

- `scripts/jig work status`
- `scripts/jig work evidence`

- `scripts/jig check contract`

## Done Means

- Run the relevant local verification for the area you changed.
- For backend changes, finish with `scripts/jig check test`.

- Review the generated diff for stale docs, policy drift, or missing dependent updates.

## Backend Guide Conventions

When a backend package or crate has an `AGENTS.md`, use these sections:

- `## Purpose`
- `## Key entrypoints`
- `## Edit here for X`
- `## Invariants`
- `## Common commands`
<!-- END JIG MANAGED BLOCK -->
