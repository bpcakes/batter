# Align operational composition ownership

This plan addresses the three repository-wide architecture findings while
preserving the documented lower-level compatibility APIs. The owning delivery
record is Bead `batter-7r3.6`, which specifically requires runnable roots to
adopt operational helpers and remove repeated consumer machinery.

## Progress

- [x] Confirmed the worktree was clean and retained the review's passing
  workspace check plus 48 focused tests as the pre-edit baseline.
- [x] Identified `batter-7r3.6` as the owning Bead through the tracker's
  authoritative JSONL mode.
- [x] Add library-owned health registration and its success/failure contracts.
- [x] Move panic payload storage to a neutral root module with compatible
  `startup` re-exports.
- [x] Convert canonical documentation and runnable service compositions to
  protected startup and adapter `_in` registration.
- [x] Update contract, status, validation evidence and the owning Bead.
- [x] Run both supported toolchains, rustdoc, and the documented HTTP smokes;
  inspect fresh Jig receipts before finishing the plan.

## Surprises & Discoveries

- The Beads SQLite cache is 23 issues behind `.beads/issues.jsonl`; normal
  reads fail on `batter-538`, and additive reconciliation reports 21 semantic
  conflicts. Use `br --no-db` for this plan's Bead reads/updates. Do not flush
  the stale database or resolve unrelated tracker conflicts.
- `Startup::scoped` and the `_in` adapter APIs already enforce the intended
  boundary. The remaining work is additive health ownership plus migration of
  the repository's primary consumer contracts, not a new startup mechanism.

## Decision Log

- Keep `Startup::new`, `HealthMonitor::run`, and the non-`_in` adapter helpers
  as explicit lower-level compatibility paths. The architecture finding is
  about the protected path being canonical, not about a breaking removal.
- Expose the shared panic payload at the crate root from a focused private
  module, then re-export it from `startup`. This gives command, lifecycle and
  startup reports a neutral dependency without inventing a generic failure
  hierarchy or breaking `batter::startup::PanicPayload`.
- Make `HealthMonitor::register_in` consume the sole writer, register its run
  future through sealed `RegistrationTarget`, and return only `HealthReader`.
  Registration failure must invoke no probe and leave no live writer.

## Outcomes & Retrospective

All three findings are resolved without removing compatibility APIs. The
foundation owns health registration through sealed authority; the canonical
documentation and three runnable service roots no longer require full supervisor
access; and command/startup/managed reports share neutral panic payload storage.
Two new health cases cover rejection and protected lifecycle behavior. Both
supported verification matrices, all ten rebuilt HTTP smokes, and the five-target
Jig profile pass on macOS arm64. Live PostgreSQL and hosted CI were not run.

The only failed final-style run was the first default matrix, where two new
rustdoc snippets returned `RegistrationError` directly from a `BoxError`
function. Adding the ordinary `Ok(...?)` coercion fixed the examples; the full
default matrix was then rerun successfully. Bead evidence is comment 66 on
`batter-7r3.6`. Its task remains open because these findings do not satisfy the
umbrella's unrelated external-adoption requirements.

## Context and orientation

`crates/batter/src/startup.rs` owns protected initialization and registration
authority. `crates/batter/src/health.rs` owns sequential dependency sampling.
`crates/batter/src/startup/report.rs`, command reports and managed lifecycle
reports all retain the same opaque panic payload today. The Axum example,
PostgreSQL lifecycle example and reference-service runtime are the runnable
composition roots that still teach direct supervisor access.

## Plan of work

First introduce the neutral panic payload module and health registration API,
with focused tests proving compatibility, inert registration failure, readiness
acknowledgement and stopped-reader ownership. Then migrate the three composition
roots and canonical rustdoc/usage text to `Startup::scoped`,
`with_unix_signals`, `HealthMonitor::register_in`, `register_http_in`, and
`batter_runledger::register_in`. Finally update guarantees/status and execute
the repository's required verification matrix and smoke tests.

## Concrete steps

1. Edit only the focused foundation modules and tests; format and run the
   foundation health/startup/command/lifecycle targets.
2. Migrate each runnable root independently and run its local tests/checks so
   protected `InitializationError<E>` mappings remain concrete and redacted.
3. Update public rustdoc and `docs/usage.md`, `docs/guarantees.md`,
   `docs/status.md`, and `docs/validation.md` with claims limited to executed
   evidence.
4. Run `bash scripts/verify.sh`, repeat with `RUSTUP_TOOLCHAIN=1.94.0`, and run
   the build/execute HTTP smoke sequence from `docs/testing.md`.
5. Inspect `scripts/jig work evidence` and `scripts/jig work gates`, run the
   applicable final work check, add the exact result to `batter-7r3.6` through
   `br --no-db`, and finish this plan.

## Validation and acceptance

Acceptance requires: both old and neutral panic payload paths compile; duplicate
health registration returns `RegistrationError` without polling a probe;
registered health participates in startup readiness and invalidates readers on
shutdown; all primary runnable service roots use protected startup; existing
failure reports retain their concrete causes; both supported toolchains and the
rebuilt HTTP smoke pass. No hosted CI, macOS, publication or deployment claim is
made without corresponding execution.

## Idempotence and recovery

All source edits are ordinary Git worktree changes and can be reviewed or
reverted by file. Verification is read-only apart from Cargo/Jig generated
evidence. Do not use the stale Beads database; JSONL-mode comments are additive.

## Interfaces and dependencies

The public addition is
`HealthMonitor::register_in(&mut impl RegistrationTarget, &'static str) ->
Result<HealthReader<E>, RegistrationError>`, with the existing `run` method
retained. `PanicPayload` and `PanicPayloadBusy` gain neutral
`batter::{PanicPayload, PanicPayloadBusy}` paths while their existing startup
paths remain source-compatible. No dependency versions or crate edges change.
