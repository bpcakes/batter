# Bound HTTP diagnostic phases

Owning Bead: batter-88d; predecessor batter-27t. Baseline HEAD486e0b0f9f4c4439077418715843b30042205f7e (the Jig baseline record is authoritative). User staged all prior work before this follow-up; preserve index tree05085928e7feea7a8b0c2b494339621a2ef5f2d8. No staging, commit or publication requested.

## Progress
- [x] Research resolved Tokio1.53.1 cancellation/JoinHandle and tracing-core0.1.36 registration plus fixture ownership.
- [x] Implement shared startup/exercise/teardown allowances and observation driver with retained diagnostics.
- [x] Add slow combined-phase and startup failure controls; remove obsolete capture retention and prove release.
- [x] Update contracts, status, references, validation and Bead; pass both full compilers, ten rebuilt smokes, final Jig gates.

## Surprises & Discoveries
The old observation fixture's5s drain+2s cancel and independent4s report+3s body wait cannot fit the new8s parent watchdog. Copying only the parent bound lost its original assumption. Startup wait occurred before returning the running owner, so simply wrapping start in timeout would lose teardown ownership. Tokio timeout drops its inner future; timing out a JoinHandle would detach work. The task itself must own the timeout and remain joined. Filtered test still retains subscribers in an obsolete static Vec. The actual DefaultCallsite registers in the global list before rebuilding interest, contrary to the review's memory-based race hypothesis.

## Decision Log
Use one set of private HTTP phase constants (startup1s, exercise2s, teardown3.5s, margin1s) below unchanged parent8s/emergency10s. Keep per-fixture semantic assertions and interruption policies; reduce oversized observation shutdown allowances to fit teardown while preserving forced drain100ms/blocked-body cancel100ms. All phases are yielding diagnostic bounds; OS/runtime non-yield remains externally contained. The observation driver owns running before readiness wait. Place timeout inside exercise task, join it, then request/release teardown. Use a single absolute teardown deadline for report and reconciliation; retain report outside reconciliation so a later timeout/panic cannot erase it. No generic application framework or production API.

## Outcomes & Retrospective
Implemented; both full compiler matrices pass684 executions each, new slow/startup failure controls pass, all10 rebuilt smokes pass, mutation baseline/negative variants pass,200 telemetry binary repeats pass. Final Jig gates passed with all five targets fresh; api:test receipt receipt_01M25VH9MQV3TKPV3E83QJYWWF. Existing staged patch remains byte-for-byte unchanged. Initial focused failure was the old combined-error text matcher, corrected while preserving both failure payload assertions.

## Context, steps and validation
Modify tests/support/http_process.rs phase policy, http_lifetime/limits.rs imports, http_lifetime_observations/{driver,limits,fixture,cases}.rs orchestration and target controls. Add Weak output-storage regression to batter/tests/telemetry/filtered.rs after removing its static retention. Tests must retain original handler/body/native-ack assertions.
Run focused HTTP and telemetry/tracing tests, then bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh, rebuild http_service per compiler and run all five smoke profiles. Existing optional HTTP mutation runner checks semantic assertions after driver refactor. Final docs/Bead updates precede fresh scripts/jig work evidence/gates/check/finish.

## Recovery and dependencies
Root Cargo graph and lock remain unchanged. New changes unstaged; preserve exact existing index. No repo cleanup of unrelated .epicd. Append only Jig state and tracker exports; old plans and validation failures remain historical. No external database provisioning or hosted/macOS claim.
