# Preserve simultaneous startup results and Unix signals

This ExecPlan is a living document governed by `.agent/PLANS.md`. Its owning
delivery task is reopened Bead `batter-lp2.2`. It repairs the post-commit review
defects in commit `b87708db06640fa9fd81f6d15cd2f2cfa538f7be`; completed earlier
plans remain historical and must not be rewritten.

## Purpose / Big Picture

Protected startup must never discard a concrete initializer error merely because
SIGTERM or SIGINT became ready in the same scheduler poll. A signal observed at a
successful final poll must remain owned through reserved registration and prevent
false readiness without needing a second signal. Legacy `Startup::new` must retain
its prior drain/interruption classification when future destruction also panics.
The change is observable through deterministic private signal controls and real
Unix subprocess cases, not timing guesses.

## Progress

- [x] (2026-09-12 16:36Z) Reopened and claimed `batter-lp2.2`; recorded the ADR-010 recurring-defect assessment.
- [x] (2026-09-12 16:42Z) Researched pinned Tokio 1.53.1 select and Unix signal semantics and PostgreSQL 18 authentication semantics from primary sources.
- [x] (2026-09-12 17:04Z) Replaced winner-takes-all startup selection with private observe-then-arbitrate polling.
- [x] (2026-09-12 17:09Z) Added deterministic simultaneous result/signal and legacy destruction/drain regressions.
- [x] (2026-09-12 17:18Z) Added repeated-signal held-cleanup subprocess coverage and updated contracts/references/status.
- [x] (2026-09-12 17:33Z) Ran focused, two-toolchain, all ten HTTP smoke and final Jig verification; all required gates pass.

## Surprises & Discoveries

- Observation: `InstalledSignals::received` records a consumed notification for
  later registration, but the current signal branch always returns `Draining`, so
  successful handoff cannot exercise that state.
  Evidence: `startup/driver.rs` lines 397-410 and `lifecycle/unix.rs` lines 93-147.
- Observation: Tokio 1.53.1 documents that `biased;` polls top-to-bottom, losing
  branches are cancelled, `Signal::recv` is cancellation-safe, notifications are
  coalesced, and a registered OS handler is never restored during the process.
  Evidence: pinned registry source and the versioned docs.rs API documentation.
- Observation: the finite `Command` path already preserves a returned result and
  separately snapshots interruption at its completion boundary. Startup needs the
  analogous private arbitration rule while retaining its stricter readiness check.
  Evidence: `crates/batter/src/command/driver.rs::run_work`.

## Decision Log

- Decision: Keep the public `StartupCause`, `InitializationError`, builders and
  adapter signatures unchanged.
  Rationale: the ownership abstraction is correct; the defect is private outcome
  arbitration, and the Bead explicitly forbids a new source-incompatible cause.
  Date/Author: 2026-09-12 / Codex.
- Decision: Poll initializer and lifecycle sources in one private boundary and
  arbitrate after observing every ready fact in that poll. Preserve ready
  application errors and panics; for a ready success retain the existing drain,
  cancellation and deadline checks. A consumed signal is carried in the same
  owned signal object into reserved registration before the final check.
  Rationale: merely moving one `select!` branch fixes one ordering but leaves the
  mutually-exclusive representation defect and cannot prove consumed handoff.
  Date/Author: 2026-09-12 / Codex.
- Decision: Use a private signal-source trait solely for native ownership and
  deterministic tests.
  Rationale: real OS process tests prove installation and delivery, while an
  injected poll source proves the exact same-poll state transfer without sleeps.
  Date/Author: 2026-09-12 / Codex.

## Outcomes & Retrospective

The public protected-startup design remains intact. The repair is confined to a
private signal-source seam and one polling/arbitration boundary. A concrete
initializer result and lifecycle facts can now coexist rather than competing as
exclusive select branches. Legacy startup again performs its final lifecycle
check independently of a successful future's destruction panic. Deterministic
controls fail under either branch-order shortcut, while fresh Unix children prove
repeated signals during held cleanup preserve one bounded unsuccessful report.

Both complete Rust 1.98.1 and 1.94.0 verification matrices and all five HTTP
profiles on each toolchain pass on Linux. The first Jig check exposed two lines
of new file-budget debt in the signal integration test; inlining its one-off
budget reduced the file below the error boundary without weakening the test.
The fresh API test, Clippy, formatting, contract and file-budget receipts all pass.

## Context and Orientation

`crates/batter/src/startup/driver.rs` owns initialization, destruction, final
readiness validation, cleanup and handoff. `crates/batter/src/lifecycle/unix.rs`
owns installed Tokio listeners and reserved signal-component registration.
`crates/batter/tests/startup.rs` owns legacy behavior, while
`crates/batter/tests/startup_signals.rs` owns real fresh-process signal evidence.
`docs/guarantees.md`, `docs/references.md`, `docs/status.md`, `docs/testing.md` and
`docs/validation.md` distinguish contracts, upstream semantics, implemented state,
test instructions and executed evidence.

Observe-then-arbitrate means polling each relevant future once, retaining all facts
that became ready in that poll, and only then choosing the public outcome. It does
not mean running initialization after an earlier drain: the existing preflight and
pending-initializer interruption still stop application work.

## Plan of Work

First add a private trait implemented by `InstalledSignals` with direct polling and
reserved registration. Make the existing public `received()` use the same polling
primitive so native and injected paths cannot drift. Generalize only the private
startup driver over that trait.

Replace the biased `select!` in `initialize_owned` with one `poll_fn` boundary.
It polls the initializer and interruption sources, marks signal reception without
discarding a simultaneously completed initializer result, then applies precedence:
a returned application error or polling panic is retained; a successful or pending
initializer observes drain, cancellation and deadline; a pending initializer with
a signal becomes `Draining`; a successful initializer with a signal transfers the
received state, and reserved registration requests drain before the final check.
After destruction, always run the legacy final check when initialization returned
success, while guarding only signal registration and readiness on destruction panic.

Add private injected tests proving pending signal drain, simultaneous error plus
signal retention, simultaneous success plus consumed signal transfer, and no Ready.
Add a public legacy test whose successful future requests drain and panics in Drop,
requiring `Draining` plus the retained panic. Extend the fresh-process suite with a
held cleanup that receives repeated TERM/INT and demonstrates one cleanup invocation,
one unchanged stop clock and truthful completion under its original budget.

Update primary-source notes and public contract/evidence documents without claiming
macOS or hosted execution. Do not change application error formatting or signal
handler restoration limitations.

## Concrete Steps

Work from `/home/aa/Documents/batter`.

Run focused tests while implementing:

    cargo test -p batter startup::driver::tests --lib --locked -- --nocapture
    cargo test -p batter --test startup --locked -- --nocapture
    cargo test -p batter --test startup_signals --locked -- --nocapture
    cargo clippy -p batter --all-targets --locked -- -D warnings

After documentation and tracker updates, run both complete verifiers, rebuild the
Axum example and execute all five smoke profiles on Rust 1.98.1 and 1.94.0. Finish
with the owning Jig plan gates and inspect evidence before rerunning a fresh target.

## Validation and Acceptance

Acceptance requires a deterministic injected source to report that signal polling
and reserved registration both occurred in the successful same-poll case, followed
by `StartupCause::Draining`, no Ready, complete cleanup, and no second signal. The
error case must retain the exact `InitializationError::Application` value while the
same poll observes the signal. The legacy destructor case must retain `Draining`
and the independent panic payload. Real TERM/INT children must still cover start,
held initialization, running supervision, owner/waiter loss and repeated signals
during held cleanup with bounded capture and reaping.

Both `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` must pass. Each toolchain must pass
the five documented HTTP smokes. Linux evidence is not macOS evidence.

## Idempotence and Recovery

Tests and documentation edits are repeatable. Real signal tests always use fresh
children because Tokio does not restore installed dispositions. If a child stalls,
the existing bounded capture owner kills and reaps it. Do not reset or discard the
already committed baseline. No commit or push is authorized by this plan.

## Interfaces and Dependencies

Public interfaces remain unchanged. Add only a private driver-facing signal trait
with methods equivalent to:

    fn poll_received(&mut self, cx: &mut Context<'_>) -> Poll<()>;
    fn register_reserved(self, supervisor: &mut Supervisor);

`InstalledSignals` implements it using pinned Tokio 1.53.1 `Signal::poll_recv` and
its existing received flag. Test sources implement it without OS effects. No new
crate dependency, runtime, framework, Windows path or global handler is introduced.

Plan revision note: created after the all-reviewer post-commit findings and ADR-010
assessment so the structural remedy, deterministic acceptance and evidence boundary
remain restartable without modifying the historical delivery plan.
