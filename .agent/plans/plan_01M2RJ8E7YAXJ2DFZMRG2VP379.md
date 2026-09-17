# Executable announcement failure proof

Owning Bead: `batter-x36`. Address review 165e5ce2's supporting coverage gap
using the user-authorized normal Jig workflow. Preserve the index and existing
worktree; no production behavior, API, migration or compatibility change.

## Progress

- [x] Inspect publisher, protected startup, production launcher and existing tests.
- [x] Add typed startup/cleanup and real executable failure probes.
- [x] Focused execution, full two-toolchain verification/live runs and ten HTTP smokes.
- [x] Independent scoped reviews and final fresh Jig gates; tracker closure follows.

## Surprises & Discoveries

Publisher unit tests did not prove the runtime's error propagation. Existing
process failure assertions alone cannot identify which initialization stage failed.
No runtime defect was reproduced and no API repair is warranted.

## Decision Log

Extend the existing production-readiness case with two bounded child phases.
The assertion child calls actual runtime::run and checks http.bind, retained
NotFound I/O cause and successful postgres.pool cleanup. The actual executable
must naturally exit1 with exact sanitized diagnostics, not after a sent signal.
Existing successful receiver/readiness/provider-work phases are the positive
control. Add the two reap bounds to the owned fixture budget; keep the66-case
inventory and each existing phase limit unchanged. All new files are within
existing exhaustive Jig source scopes.

## Execution and validation

Code is under tests/support/startup_process/watchdog/announcement_failure.rs,
with child dispatch/reexports in startup_process.rs and invocation/budget in
production_root.rs. Use scratch /tmp/batter-announcement-proof.DNODSC.
Restart the task-owned PostgreSQL18 clusters on35471/35472 with a temporary
fixture role. Run focused production_root_registers_provider_worker, both
scripts/verify.sh and full explicit scripts/test_reference_live.sh invocations,
and rebuild http_service for five smoke profiles on each toolchain. Run live
matrices without concurrent compilation after the prior task's non-reproduced
100ms pool-setup failure. Review final test sources independently, then freeze
inputs for Jig work check/evidence/gates. Drop only the temporary roles after
zero database/session checks and stop only these task-owned servers, retaining
their directories. Record results in docs/validation.md and update status,
testing and reference compatibility. Close the Bead and Jig plan after fresh
api:test evidence and all applicable gates pass.

## Outcomes & Retrospective

Focused production-root probe passed in11.07s. Both full verify scripts, ten
rebuilt HTTP smoke profiles and both complete66-case live matrices passed
(86.04s /87.79s), plus separate maintenance/state probes. Two independent scoped
reviewers found no actionable issue; one also ran the ordinary child_fixture
control. Source hashes match reviewed inputs. All five Jig targets passed and
are fresh; api:test receipt is `receipt_01M2RK0NNJRE2VWB6W5657YWBB`. Both
temporary roles were removed after zero database/session checks; both servers
stopped, retaining directories. No new public API or runtime edit.
