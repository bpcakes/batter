# Finite task shutdown causes

Owning Bead: `batter-299`. Git baseline: `61a025f5ed3699405034c1407942cdb4c280297c`.
Delivery scope and acceptance remain in Beads. The observable change is that an
admitted finite task initiating shutdown produces `ShutdownCause::FiniteTaskExit`
while a registered critical component retains `ComponentExit`.

## Progress

Added the public variant, a runnable rustdoc example, and task-kind selection in
`crates/batter/src/lifecycle.rs`. Extended existing lifecycle/process-ownership
regressions instead of duplicating their fixtures. Updated the contract,
implemented status, test inventory and changelog.

Before the implementation change, the new cause assertions failed for finite
returned errors and factory panics. Afterward all 46 focused integration tests and
four foundation doctests passed. Rust 1.98.1 and 1.94.0 each passed 433 test/doctest
executions and the full verification script. All five HTTP smoke profiles passed
after rebuilding the example with Rust 1.98.1. Evidence is in `docs/validation.md`.
All five required Jig gates passed with fresh evidence. The final backend test
command also passed 433 test/doctest executions on the final code/fixture layout.

## Surprises & Discoveries

`TaskSet::record` already has the task kind but previously returned only its name.
The finite error path closes admission before result publication; it does not
signal drain itself. The coordinator still observes the task as the initial
trigger. Failures collected during shutdown must retain the original cause.

The initial Jig gate rejected source/test file-size growth and failed in the
workspace non-yielding test target; its retained output omits the assertion.
The isolated subprocess suite rerun passed all 46 tests. Moving the example to
the existing `ProcessHandle::try_spawn` rustdoc and using an explicit receipt
binding meets the file limit without removing any assertions. An intermediate
rerun was invalidated by updating validation notes during execution. The subsequent
unchanged full gate run and final backend tests passed; the earlier subprocess
failure did not recur, and remains undiagnosed.

## Decision Log

Return `Option<ShutdownCause>` from the private recording helper so the initial
trigger preserves the existing finite/critical distinction. Keep shutdown order,
typed error retention and conservative cleanup unchanged. Downstream exhaustive
matches must add the new variant, and finite-error handling must move from the
critical-component arm. No dependencies or persisted formats change.

## Outcomes & Retrospective

Focused validation confirms both task categories and preservation of `Requested`
for later finite errors/panics. Both toolchains and all five HTTP smoke profiles
pass on macOS arm64, with Cargo.lock unchanged. The final layout also passed
doctests on both toolchains, all required Jig gates and the final backend tests.
The accepted implementation and its regression coverage are complete. Changes
remain uncommitted; work-plan and Bead closure records complete this execution.

## Validation and recovery

Run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`.
Rebuild `cargo build -p batter-axum --example http_service --locked`; execute
`python3 scripts/smoke_http.py --binary target/debug/examples/http_service` with
no extra flags, `--signal SIGINT`, `--deadline`, `--warn-filter`, and
`--warn-filter --deadline`. Run the required Jig work checks, inspect evidence
and gates, and finish backend verification with `scripts/jig check test`.
Commands can be rerun after repairing failures without relaxing the contracts.
Record actual outcomes and platform scope in `docs/validation.md`, then close
the Bead and work record. Do not commit or publish without a separate request.
