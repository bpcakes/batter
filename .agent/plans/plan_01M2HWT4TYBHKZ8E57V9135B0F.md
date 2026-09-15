# Make readiness failure states unrepresentable

This ExecPlan is a living document. Keep `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` current while implementing it. Maintain it in accordance with `.agent/PLANS.md`.

The owning Bead is `batter-isq`, “Make readiness failure states unrepresentable.” The Git baseline captured by Jig is `aeab19992a14adf7be0a493ba18f031172fd1d59`.

## Purpose / Big Picture

After this change, safe Rust code cannot describe a healthy dependency as the reason a process is unready. The `batter` foundation will own the pure decision that combines lifecycle state with the latest dependency observation, including the rule that dependency health is sampled before lifecycle so an observed drain wins. `batter-axum` will translate that already-valid decision into HTTP status and tracing severity without reconstructing foundation policy.

The behavior remains observable through the existing `/ready` endpoint: lifecycle Ready plus fresh dependency success returns an empty HTTP 200 response; every unready decision returns an empty HTTP 503 response; Starting and Draining select INFO, while process Stopped and all dependency-unready reasons select WARN. Compile-fail rustdoc will additionally demonstrate that a `HealthStatus::Healthy` value cannot inhabit `ReadinessReason::Dependency`.

## Progress

- [x] (2026-09-15 06:42Z) Confirmed the invalid public state at the current baseline and ran the existing focused Axum readiness test plus all 17 foundation health tests on Rust 1.98.1.
- [x] (2026-09-15 06:42Z) Created and claimed owning Bead `batter-isq`; started Jig plan `plan_01M2HWT4TYBHKZ8E57V9135B0F`.
- [x] (2026-09-15 06:58Z) Added the foundation dependency-readiness projection, overall decision algebra, and evaluator with public rustdoc, exhaustive state tests, and a compile-fail proof for `Dependency(Healthy)`.
- [x] (2026-09-15 06:58Z) Cut `batter-axum` over to the foundation decision, migrated workspace consumers without an invalid compatibility shim, and passed the focused adapter, example, reference-service, doctest, formatting, and Clippy checks.
- [x] (2026-09-15 07:10Z) Updated the contract, architecture, status, usage, changelog, crate guidance, Bead, and executed validation record.
- [x] (2026-09-15 07:10Z) Passed focused validation, complete Rust 1.98.1 and 1.94.0 `verify.sh` matrices, all ten rebuilt HTTP smoke profiles, diff/format checks, the file budget, and all five applicable Jig targets; inspected the final API and documentation diff.

## Surprises & Discoveries

- Observation: the normal HTTP path is behaviorally correct even though the public type is not. `ReadinessPolicy::reason` returns `Ready` for `HealthStatus::Healthy`; only public callers can fabricate `Dependency(Healthy)` directly.
  Evidence: `cargo test -p batter-axum --test operational decisions_are_read_only_and_distinguish_all_dependency_and_process_states --locked` passed before edits.
- Observation: the current `Readiness::Ready => ReadinessReason::Dependency(health)` catch-all would silently classify any future non-Healthy `HealthStatus` variant as HTTP 503/WARN rather than forcing a foundation policy decision.
  Evidence: `crates/batter-axum/src/readiness.rs` compares only `HealthStatus::Healthy` and catches the remaining value.
- Observation: Jig's exhaustive Rust inputs already include `crates/*/src/**` and `crates/*/tests/**` in both `.jig.toml` and `.agent/jig-contract.json`; adding files below those roots does not require expanding the input declarations.
- Observation: repository Clippy policy limits a test scenario to 100 source lines even when the scenario intentionally traverses a state machine. Extracting assertion helpers preserved the single lifecycle narrative while satisfying the policy.
  Evidence: the first two focused Clippy runs reported only `too_many_lines` for `decisions_are_read_only_and_distinguish_all_dependency_and_process_states`; after extracting status/severity and dependency-reason helpers, `cargo clippy -p batter -p batter-axum --all-targets --locked -- -D warnings` passed.
- Observation: the focused source and documentation changes fit the current repository file-budget policy without expanding Jig input scopes.
  Evidence: `scripts/jig check repo:file-budget --plan-id plan_01M2HWT4TYBHKZ8E57V9135B0F` passed on 2026-09-15.
- Observation: appending the first successful Jig receipt to `docs/validation.md` made the repository-policy gate stale because that document participates in the exact worktree comparison, even though the Rust inputs themselves were unchanged.
  Evidence: all five targets passed under target-validation receipt `receipt_01M2HYAD8YZCXNKHVEF4VARM7P`; the next evidence read reported the gate stale immediately after the documentation append, requiring one final connected refresh against the recorded tree.

## Decision Log

- Decision: keep `HealthStatus` as the complete observation taxonomy and introduce a separate narrower dependency-readiness projection.
  Rationale: observation and admission are different meanings. An explicit exhaustive conversion forces every future observation state to receive a readiness classification while preserving the existing diagnostic status API.
  Date/Author: 2026-09-15 / Codex
- Decision: represent overall readiness as `ReadinessDecision::Ready` or `ReadinessDecision::Unready(ReadinessReason)`; `ReadinessReason` has no `Ready` variant.
  Rationale: the outer enum makes HTTP success versus rejection structural, and the inner reason cannot contradict that verdict.
  Date/Author: 2026-09-15 / Codex
- Decision: put the evaluator and semantic types in `batter`, while keeping `StatusCode`, `tracing::Level`, response extensions, and handler rendering in `batter-axum`.
  Rationale: sampling order and lifecycle/health combination are reusable operational invariants; HTTP and observation severity are adapter policy.
  Date/Author: 2026-09-15 / Codex
- Decision: perform a direct source-breaking cutover with no deprecated `Dependency(HealthStatus)` constructor.
  Rationale: all packages are unpublished version 0.1.0 path dependencies, pending migrations have not completed, and preserving the old variant would preserve the impossible state.
  Date/Author: 2026-09-15 / Codex
- Decision: do not add a dependency registry, multi-dependency aggregator, trait abstraction, allocation, or dynamic dispatch.
  Rationale: the implemented contract has one required dependency reader. The change should centralize proven semantics without claiming a more general application framework.
  Date/Author: 2026-09-15 / Codex

## Outcomes & Retrospective

The invalid state has been removed at the foundation boundary. `HealthStatus`
remains a diagnostic observation, while `DependencyReadiness` and
`DependencyUnreadyReason` express the narrower admission fact.
`ReadinessDecision::Ready` is structurally separate from
`ReadinessDecision::Unready(ReadinessReason)`, and dependency reasons accept only
the five non-healthy cases. `ReadinessEvaluator` now owns the exhaustive
dependency-first/lifecycle-second decision, while Axum owns only HTTP 200/503 and
INFO/WARN translation. The direct API cutover renames
`ReadinessPolicy::reason()` to `decision()` and changes severity callbacks and
response extensions to the complete valid decision.

Focused tests, exhaustive unit and compile-fail coverage, both full supported
toolchain matrices, all ten rebuilt HTTP process smokes, and all five Jig targets
passed locally on macOS 26.6.2 arm64. No dependency or lockfile changed. Current
Linux, hosted CI, live PostgreSQL, publication, deployment, commit and push were
not executed or claimed.

## Context and Orientation

`crates/batter/src/health/observation.rs` defines `HealthStatus`, `HealthSnapshot`, and `HealthReader`. A health status is an observation: Healthy, Unknown, Failed, TimedOut, Stale, or Stopped. `HealthReader::snapshot` calculates that value from the latest probe, its age, and whether the sole writer remains alive. Snapshot fields are private and therefore cannot be forged by consumers.

`crates/batter/src/lifecycle/capability.rs` defines the separate lifecycle `Readiness` states Starting, Ready, Draining, and Stopped. `LifecycleStatus` exposes read-only access to those states. The foundation currently documents that adapters must combine lifecycle and dependency health themselves.

`crates/batter-axum/src/readiness.rs` currently performs that combination. Its public `ReadinessReason::Dependency(HealthStatus)` variant admits `Dependency(Healthy)`, even though its `status` and `level` methods then produce HTTP 503 and WARN. The adapter's canonical `reason` method filters Healthy, but the public enum and its public methods do not. The module also samples dependency health first and lifecycle second so an observed process drain overrides cached success.

The relevant executable coverage is in `crates/batter/tests/health.rs` and `crates/batter-axum/tests/operational/readiness.rs`. The runnable Axum composition root is `crates/batter-axum/examples/http_service.rs`; the reference consumer assembles the same policy in `examples/reference-service/src/http.rs` but does not currently pattern-match `ReadinessReason`.

## Plan of Work

First, extend `crates/batter/src/health/observation.rs` with `DependencyReadiness` and `DependencyUnreadyReason`. Use variants Ready and Unready(reason) for the first type. Use Unknown, ProbeFailed, ProbeTimedOut, Stale, and WriterStopped for the reason type so the reason names describe probe and writer facts rather than pretending Unknown is an application failure. Add `HealthStatus::readiness` and `HealthSnapshot::readiness`, implemented with an explicit six-variant match and no wildcard. Preserve `HealthStatus`, `HealthSnapshot::status`, `HealthReader::is_healthy`, retained probe causes, and all timing/ownership behavior. Re-export the new types from `batter::health` and document them with a compiling example.

Next, add `crates/batter/src/readiness.rs` and export it from `crates/batter/src/lib.rs`. Define exhaustive, copyable `ReadinessDecision` and `ReadinessReason` enums. `ReadinessDecision` has only Ready and Unready(ReadinessReason). `ReadinessReason` has Starting, Draining, Stopped, and Dependency(DependencyUnreadyReason). Provide semantic observation methods such as `is_ready` and `reason`, but do not introduce HTTP or tracing types into this module.

In the same module, define `ReadinessEvaluator<E>` containing only `LifecycleStatus` and `HealthReader<E>`. Its constructor accepts those purpose-qualified read-only capabilities. Its `decision` method must obtain a dependency snapshot first, then read lifecycle readiness. Starting, Draining, and Stopped always produce the corresponding unready lifecycle reason. Lifecycle Ready maps dependency Ready to overall Ready and dependency Unready(reason) to the nested dependency reason. Use explicit matches without catch-all branches. Implement Clone without requiring `E: Clone`, matching `HealthReader<E>`.

Add rustdoc to the evaluator showing canonical construction and decision use. Add compile-fail rustdoc that attempts to put `HealthStatus::Healthy` in `ReadinessReason::Dependency` and fails because the variant accepts `DependencyUnreadyReason`. Add foundation tests that cover every lifecycle and dependency classification, prove the dependency-first/lifecycle-second drain behavior, prove reads invoke no probe, and preserve stopped-writer precedence. Reuse real public owners rather than introducing test-only public constructors.

Then rewrite `crates/batter-axum/src/readiness.rs`. `ReadinessPolicy<E>` should own a `ReadinessEvaluator<E>` plus its severity callback. Re-export `ReadinessDecision` and `ReadinessReason` from the Axum crate root for adapter ergonomics while retaining their single foundation type identity. Rename the public observation method from `reason` to `decision`. The callback supplied to `with_level` receives a valid `ReadinessDecision`. Private adapter functions map only Ready to HTTP 200 and all Unready values to HTTP 503; default severity maps Ready and unready Starting/Draining to INFO, and maps unready Stopped/Dependency to WARN. The response extension carries `ReadinessDecision`, never a standalone reason.

Migrate `crates/batter-axum/tests/operational/readiness.rs`, the Axum example, and any workspace references found with `rg`. Keep the wire response, route construction, read-only behavior, severity override, event count, cause redaction, drain precedence, and stopped-process behavior unchanged. Replace assertions on standalone reasons with valid decisions. The negative rustdoc is the source-level proof that the old impossible construction is gone.

Update `crates/batter/AGENTS.md`, `crates/batter-axum/AGENTS.md`, `README.md`, the crate READMEs, `docs/architecture.md`, `docs/guarantees.md`, `docs/integrations.md`, `docs/usage.md`, `docs/status.md`, `docs/testing.md`, `docs/validation.md`, `docs/references.md`, and `CHANGELOG.md` where their current claims name the old owner or type. Do not invent a new upstream semantic claim: the design is repository-owned and needs no web research. Record the direct unpublished cutover and the exact executed commands. Update Bead `batter-isq` with actual evidence, then close it only after every acceptance criterion passes. Flush Beads through `br sync --flush-only`; never hand-edit `.beads/issues.jsonl`.

## Concrete Steps

Work from `/Users/aa/Documents/batter`.

Use `apply_patch` for source, test, documentation, and plan edits. After the first source slice, run:

    cargo fmt --all -- --check
    cargo test -p batter --test health --locked
    cargo test -p batter --doc --locked

After the Axum migration, run:

    cargo test -p batter-axum --test operational --locked
    cargo test -p batter-axum --doc --locked
    cargo test -p batter-axum --example http_service --locked
    cargo test -p batter-example-reference-service --no-default-features --locked

Repair compiler and test failures by preserving the stated semantics. Do not relax exact status, severity, event-count, read-only, drain-precedence, or error-redaction assertions.

When focused checks pass, inspect Jig evidence and run the required full validation:

    scripts/jig work evidence --plan-id plan_01M2HWT4TYBHKZ8E57V9135B0F
    scripts/jig work gates --plan-id plan_01M2HWT4TYBHKZ8E57V9135B0F
    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh

Build the HTTP example with each toolchain and run the five smoke profiles documented in `docs/testing.md`: default, SIGINT, deadline, WARN filter, and WARN filter plus deadline. Record exact commands and results in `docs/validation.md`; do not claim macOS, Linux, hosted, or live database evidence that was not executed.

Finish with:

    scripts/jig work check --plan-id plan_01M2HWT4TYBHKZ8E57V9135B0F
    scripts/jig work evidence --plan-id plan_01M2HWT4TYBHKZ8E57V9135B0F
    scripts/jig work gates --plan-id plan_01M2HWT4TYBHKZ8E57V9135B0F
    git diff --check
    git status --short

Expect every applicable Jig gate to pass and the final API test receipt to be fresh for this plan. If `collection_limit` appears while inspecting evidence, repeat the evidence or gates command with `--freshness-timeout-ms 30000`; that reads existing evidence without rerunning checks.

## Validation and Acceptance

Acceptance requires source-level and runtime evidence. Rustdoc must reject construction of a dependency reason from `HealthStatus::Healthy`. Foundation tests must demonstrate every current `HealthStatus` classification and every lifecycle override. The Axum operational suite must demonstrate an empty 200/INFO response only for overall Ready, empty 503/INFO for Starting and Draining, and empty 503/WARN for Stopped and all five dependency-unready reasons. The severity override must change only the response extension used by observation, not status, body, reason, or HTTP outcome.

The complete Rust 1.98.1 and 1.94.0 verification matrices must pass. All required rebuilt HTTP smoke profiles must pass. `Cargo.lock` should remain generated by Cargo and should not change because no dependency changes are planned. The final diff must update the behavior contract, implemented status, owning Bead, and validation evidence without introducing a Markdown backlog or publication/deployment claim.

## Idempotence and Recovery

Source and documentation patches are repeatable when applied only once to their exact current context. Cargo and Jig checks are safe to rerun. If a full verification command fails, record the failure before repair and rerun the affected command after the cause is corrected; do not erase unrelated failures or replace them with a later broad success claim.

The existing `.beads/issues.jsonl` modification predates this plan and includes user-owned tracker history. Preserve it. All new tracker changes must go through `br`; `br sync --flush-only` may incorporate both database-backed histories into the export. Do not reset or rewrite the file. No commit, push, publication, or deployment is authorized.

## Artifacts and Notes

Pre-change focused evidence on Rust 1.98.1:

    cargo test -p batter-axum --test operational decisions_are_read_only_and_distinguish_all_dependency_and_process_states --locked
    test result: ok. 1 passed; 0 failed

    cargo test -p batter --test health --locked
    test result: ok. 17 passed; 0 failed

The current invalid surface is `ReadinessReason::Dependency(HealthStatus)` in `crates/batter-axum/src/readiness.rs`; its public methods map `Dependency(Healthy)` to HTTP 503 and WARN. The intended replacement is a foundation `ReadinessDecision::Unready(ReadinessReason::Dependency(DependencyUnreadyReason))` construction in which no Healthy reason exists.

## Interfaces and Dependencies

At completion, `batter::health` exposes:

    pub enum DependencyReadiness {
        Ready,
        Unready(DependencyUnreadyReason),
    }

    pub enum DependencyUnreadyReason {
        Unknown,
        ProbeFailed,
        ProbeTimedOut,
        Stale,
        WriterStopped,
    }

    impl HealthStatus {
        pub const fn readiness(self) -> DependencyReadiness;
    }

    impl<E> HealthSnapshot<E> {
        pub fn readiness(&self) -> DependencyReadiness;
    }

At completion, `batter::readiness` exposes:

    pub enum ReadinessDecision {
        Ready,
        Unready(ReadinessReason),
    }

    pub enum ReadinessReason {
        Starting,
        Draining,
        Stopped,
        Dependency(DependencyUnreadyReason),
    }

    pub struct ReadinessEvaluator<E> { /* private fields */ }

    impl<E> ReadinessEvaluator<E> {
        pub fn new(lifecycle: LifecycleStatus, dependency: HealthReader<E>) -> Self;
        pub fn decision(&self) -> ReadinessDecision;
    }

`batter-axum` re-exports the two decision enums and keeps `ReadinessPolicy<E>` as the HTTP-specific wrapper. It depends on `batter`, Axum, and tracing exactly as before. The foundation gains no dependency, runtime task, allocation, callback, global state, adapter dependency, wire type, or application policy.

Plan revision note, 2026-09-15: replaced the initial one-line Jig body with this self-contained execution plan after confirming the exact public surface, repository input scopes, owning Bead, and required validation.
