# Add opt-in per-attempt retry deadlines without renewing the total budget

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept current while implementation proceeds. Maintain this plan under the repository rules in `.agent/PLANS.md`.

The owning delivery record is Bead `batter-4c4`, “Add per-attempt deadlines capped by remaining total budget.” Beads owns scope, acceptance, priority, dependencies, and completion. This file records only how to implement and verify that Bead.

## Purpose / Big Picture

After this change, an application can give a retry sequence one total deadline and separately cap every provider call. A slow call can therefore stop at, for example, two seconds while a larger fifteen-second total context still has time remaining. The attempt cap never grants a later attempt fresh total time: every attempt deadline is the earlier of the original input context deadline and the time at which that attempt starts plus its configured maximum.

The existing `retry::execute` and `retry::execute_with_jitter` APIs keep their signatures, results, and default behavior. Applications opt in through a new options-based entrypoint and an extensible typed error that distinguishes a per-attempt deadline from cancellation or expiration of the original total context. The runnable `operation_budget` example and paused-time tests demonstrate the difference.

## Progress

- [x] (2026-09-14 19:05Z) Read the repository and foundation guides, inspected retry and operation boundaries, verified the consumer gap, reconciled the stale local Beads database losslessly, and claimed `batter-4c4`.
- [x] (2026-09-14 19:07Z) Started Jig plan `plan_01M2GMYGFXXAXEDRBC38HV9ZVV` at Git baseline `150d16df364379b4ae64a5af4eb82a68b08a0ff0` and wrote this implementation plan.
- [x] (2026-09-14 19:23Z) Implemented the validated options/result boundary and routed both legacy APIs plus deterministic/jittered opt-in execution through one private retry loop.
- [x] (2026-09-14 19:27Z) Added 14 focused paused-time, destruction, panic, composition, and legacy-exhaustiveness regressions covering the complete Bead acceptance matrix.
- [x] (2026-09-14 19:29Z) Updated the runnable example, public contracts, status/testing facts, pinned primary-reference note, changelog, and this plan; only executed validation evidence remains to append.
- [x] (2026-09-14 19:35Z) Passed focused formatting, 18 legacy and 14 new retry tests, 35 doctests, warning-denied rustdoc, strict Clippy, diff hygiene, and the runnable example on the default toolchain.
- [x] (2026-09-14 19:35Z) Passed both complete supported-toolchain verification matrices, ran the example on both toolchains, rebuilt the HTTP example for each, passed all ten required smoke profiles, and recorded exact local evidence in `docs/validation.md`.
- [x] (2026-09-14 19:38Z) Passed all five applicable Jig targets under target-validation receipt `receipt_01M2GPTBWX5SAD9P99WM2DEGCK`; fresh evidence and gate inspection reported no unresolved gate.
- [x] (2026-09-14 19:42Z) Audited every acceptance criterion against source, focused tests, public contracts and executed evidence; closed and flushed `batter-4c4`, confirmed healthy tracker synchronization, and refreshed tracker-only Jig evidence under target-validation receipt `receipt_01M2GPZKEV0W2C3VPRCE8X2MPF` with no unresolved gate.

## Surprises & Discoveries

- Observation: The checked-in `.beads/issues.jsonl` was newer than the SQLite tracker after prior work; automatic import failed on a semantic verification mismatch for `batter-k8m`.
  Evidence: `br sync --status --json` reported `jsonl_newer`, 124 DB issues, and 125 JSONL issues. `br sync --reconcile --dry-run --json` planned one create and two newer updates with no deletes or DB-only records. The applied additive reconciliation preserved 122 equal records and then allowed `batter-4c4` to be claimed.
- Observation: `OperationContext::run` already enforces the exact boundary precedence required here and cancels the child context on every exit or dropped run future.
  Evidence: `crates/batter/src/operation.rs` uses a biased `tokio::select!` ordered cancellation, deadline sleep, then work completion, with a cancellation-token drop guard surrounding the selected work future.
- Observation: Runtime duration validation already has the needed common policy.
  Evidence: `crates/batter/src/validation.rs::positive` rejects zero, durations over one year, and values not representable by Tokio `Instant::checked_add`, using redacted `ConfigurationError` field names.
- Observation: A separate jitter-plus-options free function violates this repository's public argument-count lint, and adding the options parameter pushed the shared loop over its line limit.
  Evidence: The first strict `cargo clippy -p batter --all-targets --locked -- -D warnings` rejected `execute_with_jitter_and_options` at eight arguments and `execute_with_delay` at 107 lines. Moving the sampler into generic options and extracting `run_attempt` made the next strict Clippy invocation pass without lint allowances.

## Decision Log

- Decision: Add a private-field `RetryOptions` value with `new`, a fallible `with_attempt_maximum` modifier, and a read-only accessor. An empty value means no attempt cap.
  Rationale: Construction validates configuration before execution can invoke a factory, private fields permit the dependent retry-token Bead to extend the same opt-in boundary source-compatibly, and the empty form supports future token-only policy without changing legacy APIs.
  Date/Author: 2026-09-14 / Codex
- Decision: Add a `#[non_exhaustive] RetryExecutionError<E>` with `Stopped`, `Interrupted`, and `AttemptDeadlineExceeded` variants rather than changing `RetryError<E>`, `StopReason`, or `Interruption`.
  Rationale: Current consumers may exhaustively match every legacy enum. A separate non-exhaustive boundary provides direct typed outcomes and leaves room for the dependent retry-token exhaustion outcome without another breaking error enum.
  Date/Author: 2026-09-14 / Codex
- Decision: Expose `execute_with_options` and `execute_with_jitter_and_options`, while leaving `execute` and `execute_with_jitter` unchanged. All four call one private scheduling loop.
  Rationale: Per-attempt caps must compose with the existing injected equal-jitter and provider-floor behavior, and the dependent Bead must later combine options without creating a second retry scheduler.
  Date/Author: 2026-09-14 / Codex
- Decision: Supersede the separate `execute_with_jitter_and_options` function after its focused Clippy failure. Make `RetryOptions<S>` hold an optional per-execution sampler, add `with_jitter`, consume the options value in the single `execute_with_options` function, and retain the two legacy functions unchanged.
  Rationale: This directly composes cap, jitter, provider floors, and future token state while keeping the public call at seven arguments. It avoids a lint exception and gives the dependent Bead one opt-in entrypoint instead of parallel schedulers.
  Date/Author: 2026-09-14 / Codex
- Decision: Derive a temporary attempt context with `OperationContext::child` immediately before calling `OperationContext::run`, and classify its deadline as an attempt cap only when its absolute deadline is strictly earlier than the original input deadline.
  Rationale: `child` computes `min(original deadline, now + maximum)` with downward cancellation. Strict inequality makes a tied cap/total boundary report the original total deadline. Reusing `run` preserves cancellation-first and deadline-before-completion precedence, panic propagation, future destruction, tracing scope, and child cancellation.
  Date/Author: 2026-09-14 / Codex
- Decision: Put the new focused timing matrix in `crates/batter/tests/retry_attempt_deadlines.rs` while retaining existing legacy coverage in `crates/batter/tests/retry.rs`.
  Rationale: The existing test file is already over 500 lines. A sibling integration test remains inside the existing exhaustive Jig `crates/*/tests/**` input scope, is automatically discovered by Cargo, and keeps the new contract readable without creating a new source root or fixture system.
  Date/Author: 2026-09-14 / Codex

## Outcomes & Retrospective

The opt-in per-attempt deadline boundary is complete. The implementation keeps
one retry scheduler and one operation boundary: options carry the relative
attempt maximum and optional jitter sampler, while `OperationContext::run`
continues to own cancellation/deadline precedence and future destruction. A
separate non-exhaustive result leaves every legacy exhaustive match intact and
reserves future extension space for the dependent retry-token work.

Fourteen focused tests prove the cap formula for initial and later attempts,
total clamps and ties, finalization reserve, cancellation/deadline precedence,
factory/classifier counts, prior-error retention, destruction, panic propagation
and jitter/provider-floor composition. Both full supported-toolchain matrices,
both example runs and all ten rebuilt HTTP profiles passed. Final Jig evidence
is fresh with no unresolved gate, and the owning Bead is closed with a healthy
tracker export.

The initial separate jitter-plus-options function would have expanded the API
and violated the repository's argument-count lint. Moving the sampler into the
options value yielded one extensible entrypoint suitable for the dependent token
policy without lint exceptions or a second scheduler. The work establishes
local Linux behavior only; Studio adoption, remote-provider cancellation,
macOS and hosted-CI execution remain outside this result.

## Context and Orientation

This repository is a Unix-only virtual Cargo workspace. `crates/batter` is the native Tokio foundation. Its `crates/batter/src/retry.rs` module owns fresh attempt factories, caller-declared replay safety, application error classification, bounded attempts, provider delay floors, deterministic exponential backoff, and injected equal jitter. The current `RetryError<E>` separates terminal application errors from cancellation or expiration of one total `OperationContext`; it must remain source compatible.

`crates/batter/src/operation.rs` owns `OperationContext`. A context contains an absolute Tokio `Instant` deadline and a Tokio cancellation token. `OperationContext::child(maximum)` creates a downward-cancelled child whose deadline is `min(parent deadline, Instant::now() + maximum)`. The implementation now shares that derivation through a private prevalidated helper used by retry options. `OperationContext::run` performs a preflight and then polls cancellation, its own deadline, and a freshly created application future in that priority order. A drop guard cancels the scope handed to the application when execution succeeds, fails, times out, is cancelled, or is dropped. It does not join arbitrary tasks spawned by the application.

The “total context” in this plan is exactly the `OperationContext` passed to retry execution. It may already be `OperationPhases::work()`, whose deadline is intentionally shorter than its parent so finalization time is reserved. Retry code must not reach through that context to recover the parent deadline.

`crates/batter/examples/operation_budget.rs` is a registered runnable example that currently combines a shortened work phase, injected jitter, and finalization. Extend it to construct a total work allowance and a shorter attempt cap, fail once, then visibly succeed without skipping finalization. Keep error values concrete and do not imply rollback, remote cancellation, or exactly-once behavior.

`docs/guarantees.md` owns the retry behavior contract. `docs/usage.md` owns the canonical consumer snippet. `docs/status.md` records implemented capability facts, `docs/testing.md` names coverage, `docs/references.md` records the pinned upstream Tokio semantics checked for the implementation, `docs/validation.md` records commands actually executed, and `CHANGELOG.md` records the additive public API. Do not add a Markdown backlog.

The concrete consumer motivation is `/home/aa/Documents/pancake-studio/docs/batter-adoption.md`: its `ResendMailer` already uses three attempts under a child deadline capped at fifteen seconds, but the pinned Batter revision has no per-attempt cap. This task changes Batter only. Updating the external consumer belongs to the dependent adoption work and is not authorized here.

## Plan of Work

First edit `crates/batter/src/retry.rs`. Define `RetryOptions` beside `RetryPolicy`. `RetryOptions::new` returns an empty options value. `with_attempt_maximum` must call the existing shared positive-duration validator with a generic field label such as `retry attempt maximum`; on success it returns the updated value. Expose the optional configured duration through an accessor. Do not store an absolute deadline in options because every attempt starts later and must derive its own absolute cap.

Define `RetryExecutionError<E>` beside `RetryError<E>`. Give it the same `Stopped` and `Interrupted` fields and meanings as the legacy error, plus `AttemptDeadlineExceeded { attempts, last_error }`. Mark the enum non-exhaustive and document that timeout/cancellation can leave external outcome unknown. Add a private lossless conversion from the extended stopped/interrupted forms to the legacy error. Do not add an attempt-timeout variant to `Interruption` because that would break exhaustive matches and would incorrectly describe expiration of the original operation context.

Keep public legacy entrypoints exactly source compatible. Add the generic `execute_with_options` entrypoint described in the superseding Decision Log. `RetryOptions::with_jitter` changes only the sampler type and retains the attempt cap; options without a sampler remain deterministic. Route all entrypoints through the existing single private loop, generalized around private execution settings. Legacy callers pass no attempt maximum and convert the impossible-attempt-timeout extended result back to `RetryError<E>`.

In the private loop, retain the total-context preflight before each attempt. Immediately afterward create an optional attempt child from the configured maximum, choose that child as the context passed to `run`, and remember whether its absolute deadline is strictly less than the total input context deadline. Keep incrementing the attempt count inside the factory closure so a preflight interruption does not count work that never started. When `run` returns cancellation, report `Interrupted::Cancelled`. When it returns deadline expiration, report `AttemptDeadlineExceeded` only for a strictly earlier attempt deadline; otherwise report `Interrupted::DeadlineExceeded`. Return immediately in either case without calling the classifier or starting another factory. Leave returned application-error classification, replay permission, maximum-attempt logic, provider floor, jitter, remaining-total-budget check, and total-context backoff unchanged.

Then add `crates/batter/tests/retry_attempt_deadlines.rs`. Use Tokio paused time and exact recorded absolute deadlines rather than wall-clock tolerances. Cover: zero and too-large configuration rejection before a factory can exist; first-attempt cap expiration while total time remains; later-attempt cap after backoff with the prior concrete error retained; a parent total deadline shorter than the cap; equal attempt and total deadlines reported as total; a cancelled and expired context reported as cancellation with zero factories; cancellation when an attempt deadline is also ready; deadline beating simultaneously ready successful completion; a shortened `OperationPhases::work()` context clamping the attempt without recovering finalization time; cancellation during backoff retaining the prior error and count; no classifier call for an interrupted attempt; actual application-future destruction and cancellation of the attempt scope; panic propagation without classification; jitter/provider-floor composition; and exhaustive matching of every unchanged legacy enum. Assert factory and classifier counters so every stated non-invocation is executable evidence.

Update `operation_budget.rs` to attach a jitter sampler to `RetryOptions` and pass it to `execute_with_options` with a total work deadline longer than its attempt cap. The first attempt should be a returned retryable error and the second a bounded success, keeping finalization explicitly awaited. The output should still report successful work and finalization. Add rustdoc on all public items and functions; `docs/usage.md` supplies the deterministic public example while the registered runnable example proves jitter composition.

Finally update the owning contracts and evidence files. Remove the obsolete “no per-attempt budgets” statement from `docs/effect-v4-reconciliation.md` as well as the broader guarantees/status gap. State precisely that an attempt timeout drops the attempt future and stops the sequence; it neither retries that interrupted call nor proves its external outcome. Do not claim retry tokens, circuit breaking, provider cancellation, or consumer adoption. Record the actual upstream Tokio 1.53.1 source/docs used for `select!`, `Instant::checked_add`, and cancellation tokens in `docs/references.md`.

## Concrete Steps

Run every command from `/home/aa/Documents/batter`.

After source and focused tests are written, format and execute the directly affected target:

    cargo fmt --all
    cargo test -p batter --test retry --locked
    cargo test -p batter --test retry_attempt_deadlines --locked
    cargo test -p batter --doc --locked
    cargo clippy -p batter --all-targets --locked -- -D warnings
    cargo run -p batter --example operation_budget --locked

The new test target must pass all 14 tests with zero ignored cases. The example must exit zero and print `work succeeded: true; finalization succeeded: true`.

After focused repair, run the repository-required supported matrices:

    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh

Build and exercise the HTTP adapter on each toolchain. For the default toolchain:

    cargo build -p batter-axum --example http_service --locked
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline

Repeat the build and five commands with `RUSTUP_TOOLCHAIN=1.94.0` applied to both Cargo and Python commands so the rebuilt binary and smoke invocation are tied to the same toolchain evidence.

Append exact operating system, architecture, `rustc --version`, lockfile hash, commands, counts, outcomes, and any unavailable prerequisite to `docs/validation.md`. Then run:

    scripts/jig work check --plan-id plan_01M2GMYGFXXAXEDRBC38HV9ZVV
    scripts/jig work evidence --plan-id plan_01M2GMYGFXXAXEDRBC38HV9ZVV --freshness-timeout-ms 30000
    scripts/jig work gates --plan-id plan_01M2GMYGFXXAXEDRBC38HV9ZVV --freshness-timeout-ms 30000

After an evidence-only validation append, refresh only checks whose exhaustive inputs include changed files; use the repository’s receipt-reuse rules rather than automatically repeating unchanged Rust work. Close `batter-4c4` only after the completion audit succeeds, then run `br sync --flush-only` and `scripts/jig work finish --plan-id plan_01M2GMYGFXXAXEDRBC38HV9ZVV` using the exact supported command syntax shown by `--help` if it differs.

## Validation and Acceptance

Acceptance is behavioral. A paused-time first attempt with a ten-second total context and one-second attempt maximum must end after one second as `RetryExecutionError::AttemptDeadlineExceeded { attempts: 1, last_error: None }`, while `context.check()` still succeeds. No classifier and no second factory may run.

A first returned error followed by backoff and a slow second attempt must time out at the second attempt’s start plus the maximum, retain the first concrete error, and report exactly two invoked factories. The backoff and both attempts remain under the unchanged original total deadline. A total or shortened work deadline earlier than the requested maximum must instead produce `RetryExecutionError::Interrupted` with `Interruption::DeadlineExceeded`. Equal absolute attempt and total deadlines must take that same total result.

Cancellation must beat both deadline classes, and an expired deadline must beat a simultaneously ready success. Dropping or interrupting an attempt must destroy its owned future and leave the exact `Attempt.context` cancelled. Panics must escape the retry helper to the Tokio task boundary without entering the classifier. Invalid options must return `ConfigurationError` without any factory invocation.

Legacy `execute` and `execute_with_jitter` calls and current exhaustive matches over `RetryError`, `StopReason`, and `Interruption` must continue compiling and passing unchanged. The extended error must require a wildcard for external exhaustive matching, preserving room for retry-token exhaustion. Equal jitter and provider delay floors must remain active when used through the options entrypoint.

Repository acceptance additionally requires both complete Rust 1.98.1 and 1.94.0 verification matrices, all ten rebuilt HTTP process profiles across those toolchains, a successful runnable example, fresh applicable Jig receipts, current documentation, and a final diff/Bead audit. These local results do not prove Studio adoption, hosted CI, macOS execution, provider cancellation, or remote outcome certainty.

## Idempotence and Recovery

Formatting and all validation commands are safe to repeat. Cargo must keep `Cargo.lock` generated and locked; do not hand-edit it. The new code is additive around the public surface and should require no migration or durable-state recovery.

If a focused timing test hangs, run only that exact test name and inspect whether paused time can reach every timer; do not replace an exact virtual-time assertion with an arbitrary wall-clock sleep. If full verification fails in unrelated existing work, establish the exact failing target and worktree bytes before deciding whether it is independent. Do not erase user changes or use destructive Git commands.

Tracker recovery must preserve the now-reconciled JSONL as authoritative. Do not force-export a stale DB. Before any final tracker mutation, inspect `br sync --status --json`; if drift recurs, use the lossless reconcile dry run and review its create/update/delete counts before applying it.

## Artifacts and Notes

The plan baseline is:

    150d16df364379b4ae64a5af4eb82a68b08a0ff0

The external consumer currently records:

    ResendMailer: three attempts, 250/500 ms backoff, one child total budget capped at 15 seconds.
    Remaining gap at its pin: per-attempt retry deadlines/tokens are not implemented.

The implementation must satisfy the deadline identity:

    attempt_deadline = min(input_context.deadline(), attempt_start + attempt_maximum)

The comparison used to label the terminal outcome is:

    attempt_deadline < input_context.deadline()  => per-attempt deadline
    attempt_deadline == input_context.deadline() => original total deadline

## Interfaces and Dependencies

At completion, `crates/batter/src/retry.rs` must expose equivalent APIs to:

    #[derive(Clone, Debug)]
    pub struct RetryOptions<S = fn() -> u64> { /* private fields */ }

    impl RetryOptions<fn() -> u64> {
        pub fn new() -> Self;
    }

    impl<S> RetryOptions<S> {
        pub fn with_attempt_maximum(
            self,
            maximum: Duration,
        ) -> Result<Self, ConfigurationError>;
        pub fn attempt_maximum(&self) -> Option<Duration>;
        pub fn with_jitter<T>(self, sample: T) -> RetryOptions<T>
        where
            T: FnMut() -> u64;
    }

    #[non_exhaustive]
    pub enum RetryExecutionError<E> {
        Stopped { attempts: u32, reason: StopReason, error: E },
        Interrupted {
            attempts: u32,
            reason: Interruption,
            last_error: Option<E>,
        },
        AttemptDeadlineExceeded {
            attempts: u32,
            last_error: Option<E>,
        },
    }

    pub async fn execute_with_options<T, E, F, Fut, C, S>(
        context: &OperationContext,
        operation: &'static str,
        safety: ReplaySafety,
        policy: &RetryPolicy,
        options: RetryOptions<S>,
        factory: F,
        classify: C,
    ) -> Result<T, RetryExecutionError<E>>
    where
        S: FnMut() -> u64;

Exact formatting may change during implementation, but these semantics and the legacy signatures may not. Use only the existing `std`, Tokio 1.53.1, tokio-util 0.7.19, tracing, thiserror, and internal validation/operation modules. Do not add an ORM, dependency injection, retry crate, RNG dependency, task owner, global state, or adapter dependency.

Revision note (2026-09-14): Replaced the initial one-line Jig body with a self-contained implementation and validation plan after inspecting the live retry/operation contracts and the lossless tracker reconciliation.

Revision note (2026-09-14): Recorded the focused Clippy discovery and superseded the separate jitter-plus-options function with a generic sampler carried by the single options value. Updated progress, work steps, test count, interfaces, and locked tokio-util version to match the implemented surface.

Revision note (2026-09-14): Recorded the completed focused, two-toolchain and
HTTP smoke evidence, exact Linux/toolchain/lockfile metadata, and the remaining
Jig/tracker completion boundary.

Revision note (2026-09-14): Recorded the acceptance audit, closed synchronized
Bead state, final tracker-only Jig refresh, and completed outcome.
