Owning Bead: batter-9s6. Baseline: 9aa2f8006000d460011638a7c9f12ace75e907ca. Preserve all existing uncommitted review fixes.

## Progress
- [x] Add rendered-chain regression and distinct shared-report context; preserve direct example count summaries by borrowing the underlying report.
- [x] Factor private SQLx example completion/exit helpers and cover success, startup-cleanup failure and shutdown failure; add explicit live pool checks.
- [x] Extend macOS CI and execute Linux/macOS verification and live PostgreSQL checks.
- [x] Update evidence, close Bead, run final Jig checks and close plan.

## Surprises & Discoveries
Previous report/source Display texts are identical. Existing database subprocess tests fail before resource acquisition. The previously used macOS host is reachable, and a local PostgreSQL 18 image is cached.
The rendered-chain regression failed before the display fix. Five portable exit tests and four smoke controls pass. Explicitly selected live tests without DATABASE_URL fail instead of silently skipping. Linux PostgreSQL 18.3 checks pass on both toolchains; no provisioning code was added to the repository. The first macOS verification attempt using the existing target directory received SIGKILL on the scheduling binary before test execution; later signature validation and listing succeeded. A fresh isolated target with four build jobs passed both Rust versions and all live/smoke checks. The final direct-report formatting callers also passed focused verification on both hosts and versions.

## Decision Log
Preserve the concrete ShutdownReport source and give its shared owner distinct Display context. Keep testable completion logic private to the example; no database adapter or provisioning abstraction. Live tests use externally provisioned PostgreSQL and fail when explicitly selected without DATABASE_URL. Isolate remote source snapshots and preserve remote worktrees. Do not publish or commit.

## Outcomes & Retrospective
Both toolchains passed full verification on Linux (443 passed, 3 explicitly live tests ignored per run) and macOS (439 passed, 3 ignored). All three live PostgreSQL tests passed explicitly on each host/version, and both SQLx signal smokes and five HTTP profiles passed on every build. Four Python controls passed on both hosts. Final worker/HTTP formatter changes passed focused example tests and all five HTTP profiles again on each host/version. The final local scripts/jig check test passed. All 111 selected source/config files matched the final remote snapshot; logs and the source manifest are retained locally. The dedicated database container and isolated macOS source/target directory were removed after evidence collection. No hosted CI run, dependency change, commit or publication is claimed. Bead batter-9s6 is closed and synced. Final Jig run run_01M239ATHG5Q2G3Z4XA41A2Q4C passed all five targets; work evidence and work gates confirmed fresh receipts matching the final inputs. Existing nonblocking file-size advisories remain for lifecycle.rs and process_ownership.rs. git diff --check passed.

## Execution
Edit lifecycle/driver.rs and process_ownership/report_usage.rs. Extract only the existing completion branch and exit handler in examples/postgres-lifecycle/src/main.rs, with sibling test modules. Test retained causes and actual output/exit status; live tests use native SQLx and external resources. Extend .github/workflows/ci.yml for the example and report tests on macOS. Update guarantees, status, references, testing and validation. Run both Rust toolchain verify scripts and five rebuilt HTTP profiles, portable macOS verification, explicit database tests, and the required final Jig test target. Close the Bead before refreshing final work gate receipts so tracker changes do not stale evidence. All temporary resources are task-owned; stop only those resources after use.
