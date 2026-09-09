# Observer review follow-up

Owning Bead: `batter-w3o`. Git baseline: `46d708717f02e58acf171ed5a0d542c69923acf2`.
The prior `batter-50z` changes are staged and remain intact. This follow-up changes
documentation and tests only, with no production behavior, dependencies or
persisted state changes. Delivery scope and acceptance remain in Beads.

## Progress

- Added panic documentation to `SupervisorObserver::wait` and the forwarding
  `RunningSupervisor::wait`/`shutdown` methods, plus the Unreleased migration note.
- Added three deterministic regressions to `driver_observer.rs`: immediate-owner
  loss plus coordinator panic, constructing an observer after publication from
  an owner clone, and runtime destruction before publication. The late-observer
  test also proves that a published report survives runtime destruction.
- Focused tests passed: 31 integration tests and three foundation doctests.
- Both full Rust matrices passed 432 test/doctest executions each; all five HTTP
  smoke profiles passed. Evidence is recorded in `docs/validation.md`.
- Jig Clippy, formatting, tests, contract and file-budget gates passed with fresh
  evidence; the final `scripts/jig check test` also passed.

## Surprises & Discoveries

The coordinator-panic fixture must panic while dropping a skipped cleanup capture,
not inside a finalizer task. A capture counter and zero factory-call assertion
prove that the error is a coordinator panic, retained by the separate monitor.
Entering a current-thread runtime permits `start` without polling the coordinator;
destroying that runtime deterministically loses the unpublished sender.

## Decision Log

Keep the existing runtime-liveness precondition and document its panic behavior.
Do not invent an outcome when no report or coordinator JoinError was published.
Keep all existing observer tests; new waits use bounded test timeouts. No index
changes, commit or publication are authorized by this follow-up.

## Outcomes & Retrospective

The accepted review findings and both test gaps are covered. Both toolchains,
all HTTP smoke profiles, Jig gates and final backend tests pass. Production
behavior and the prior staged changes are intact; the follow-up is uncommitted.
The work-plan and Bead closure records complete this execution.

## Validation and recovery

Run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`.
Build `cargo build -p batter-axum --example http_service --locked`, then run
`python3 scripts/smoke_http.py --binary target/debug/examples/http_service` with
no extra flags, `--signal SIGINT`, `--deadline`, `--warn-filter`, and
`--warn-filter --deadline`. Run `scripts/jig work check`, `work evidence`,
`work gates` for this plan and finish backend verification with
`scripts/jig check test`. Record results, versions, platform and unchanged lock
hash. Repair failures without weakening contracts; all commands are rerunnable.
Close the work plan and Bead after successful checks.
