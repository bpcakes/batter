# Completion observers require owned drivers

Owning Bead: `batter-50z`. Baseline: `46d708717f02e58acf171ed5a0d542c69923acf2`.
Scope and acceptance remain in Beads. The public source migration is
`handle.observer()` to `running.observer()` after `Supervisor::start`.
There are no persisted-state or dependency changes.

## Progress

- Removed shutdown-handle observer construction and moved the watch channel from
  shared lifecycle state into `Supervisor::start`; its monitor owns the sender.
- Added compile-fail API coverage and an immediate-owner-drop runtime regression.
  Updated the existing waiter-cancellation caller and the abandonment test, whose
  removed observer is no longer constructible. Readiness assertions remain intact.
- Focused validation passed: 33 integration tests and three foundation doctests.
- Full Rust 1.98.1 / 1.94.0 verification passed 426 test/doctest executions each;
  all five HTTP smoke profiles passed. Evidence is in `docs/validation.md`.
- Jig's Clippy, formatting, tests, contract and file-budget gates passed with
  fresh evidence. The final `scripts/jig check test` also passed.

## Surprises & Discoveries

The old abandonment test retained the control handle, keeping the sender alive.
Its pending assertion could not detect the reported channel-closure panic.
The new compile-fail test failed against the old API because the invalid observer
construction compiled; it passes after removal.

## Decision Log

Restrict observer construction instead of adding an unavailable-completion error.
Creating the channel only in `start` aligns the sender lifetime with the monitor
and removes completion state from supervisors that can never publish a result.
Existing `process_owned` remains the runnable example of the supported API.

## Outcomes & Retrospective

The completion channel is now confined to the owned driver. The supported API
preserves last-owner shutdown and retained results, and invalid control-handle
observer construction fails at compile time. Both full toolchain matrices and
HTTP smoke profiles pass, as do the Jig gates and final backend test command.
No dependencies, persisted state, readiness behavior or cleanup policy changed.
The Bead and Jig closure records complete this execution; changes remain uncommitted.

## Validation and recovery

Run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`.
Build `cargo build -p batter-axum --example http_service --locked`, then run
`python3 scripts/smoke_http.py --binary target/debug/examples/http_service` with
no extra flags, `--signal SIGINT`, `--deadline`, `--warn-filter`, and
`--warn-filter --deadline`. Record platform, versions, lock hash and results in
`docs/validation.md`. Run `scripts/jig work check`, `work evidence`, `work gates`
for this plan and finish backend verification with `scripts/jig check test`.
Close the Bead and work plan only after successful checks. Commands are rerunnable;
repair failures without weakening the contracts. No commit or publication.
