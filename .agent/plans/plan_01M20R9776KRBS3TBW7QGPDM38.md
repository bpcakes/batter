# Exercise cancellation and admission under sustained scheduling

This living ExecPlan belongs to Bead batter-953 and follows `.agent/PLANS.md`. Beads owns its acceptance criteria. The baseline is Git `5db16f6f18fd918d3d3558a68843120bf9a13b79`; the initial dirty Beads export contains the user's approved task refinement and must be preserved. Do not commit or publish without a separate request.

## Purpose / Big Picture


Make the foundation's operational contracts harder to break with a green test suite. The completed work lets a maintainer run a bounded two/four-worker corpus, rerun one scenario seed, replay a controlled admission schedule, and verify that faulty capacity and a wedged runtime are rejected. This is exploration evidence, not an exhaustive scheduling or preemption guarantee.

## Progress


- [x] 2026-09-08: Re-read tracker/current source, confirm ready delivery task, claim batter-953.
- [x] 2026-09-08: Add deterministic shared-capacity regression and bounded subprocess/profile harness.
- [x] 2026-09-08: Implement independent admission/result accounting and sustained root/descendant workload.
- [x] 2026-09-08: Implement all eight seeded contract families, finite unjoined receipt evidence and controlled replay.
- [x] 2026-09-08: Both toolchains reject six capacity-mutant replays and accept six original replays each; watchdog/overflow/early-exit/redaction controls pass. Final readiness oracle passes three fresh corpora per worker count.
- [x] 2026-09-08: Update contract/testing/status/validation evidence; final Rust matrices and three HTTP modes passed; Jig verify profile and fresh required gates passed.
- [x] 2026-09-08: Explicit backend test command passed (35.2 s); final scope/evidence review found all implementation criteria covered.
- [x] 2026-09-08: Refresh fresh required Jig gates after final metadata; run the final backend test and finish the structured work plan successfully.
- [x] 2026-09-08: Review all acceptance items against source and executed evidence; close batter-953 and refresh its export.

## Surprises & Discoveries


The audit showed that existing 60 focused tests accept a descendant-independent semaphore variant. A capacity-one ancestor/child rejection rejects it. The existing two race tests have bounded iterations but no external hang containment. During implementation, the ledger was kept explicit about factory return preceding actual wrapper permit release; transient Full remains an allowed submission result and is handled within the case deadline. Final review found that observing only Stopped could mask an invalid late Ready publication, so readiness fixtures now hold acknowledged components alive until Draining is asserted. Early compile lifetime/move errors and Clippy complexity failures were repaired without weakening assertions. `bv` reports a legacy snapshot path; current `br show` and `br ready` confirmed the task is open and its prerequisites are closed.

## Decision Log


Use only test-local orchestration and standard-library Python process containment; keep native Rust/Tokio library APIs unchanged unless a deterministic failure demonstrates a bug. Use an independent event/result ledger, bounded gates, and named fixed action schedules alongside seeded yields. A seed controls scenario choices, not OS/Tokio scheduling. External subprocess watchdogs bound failure handling even when runtime timers cannot progress. Keep the existing non-yielding harness intact; its event/timing protocol is specific to that evidence.

## Outcomes & Retrospective


Implementation and focused failure controls are complete. The unchanged library passes the new oracle and an independently corrupted capacity variant fails it. No production bug or dependency change was required. Both final toolchain matrices and all three HTTP modes passed after strengthening readiness observation. The six fresh corpus runs each completed all families and 4,096 finite tasks in approximately 6.3 seconds. Jig required profile gates passed with fresh evidence. The final explicit backend test command passed (35.2 s). All implementation acceptance items have source and executed evidence. Task batter-953 is closed and its export is current. Required Jig evidence was refreshed after the final documentation/tracker updates and is fresh/passing. The final backend check passed and structured work finish succeeded. All planned work is complete; changes remain uncommitted.

## Context and Orientation


`crates/batter/src/lifecycle.rs` owns readiness, shutdown phases and result harvesting. `lifecycle/process.rs` owns shared queued/executing capacity and active descendant capabilities; `lifecycle/driver.rs` separates completion waiters from the running coordinator. `operation.rs` owns downward cancellation and deadlines. Existing tests in process_ownership.rs, lifecycle.rs and operation.rs remain deterministic baseline checks.

Add `crates/batter/tests/scheduling.rs` with supporting files under `tests/scheduling/`. A subprocess entry runs a Tokio runtime with two or four workers; ordinary Cargo test entries run bounded child profiles. `scripts/stress_scheduling.py` owns process launch, bounded output, external deadline, single-seed and named-schedule selection. A separate disposable mutation command can compile the real suite against the faulty shared-capacity variant without editing the working library. No dependency or production API addition is intended.

## Plan of Work


First add the capacity-one check and profile launch protocol, including a deliberate fixture that blocks the Tokio driver so in-runtime timeout is ineffective. The parent watchdog must kill/reap it and report a distinct failure with the last checkpoint. Put strict bounds on input, captured output and per-case records.

Then implement a seeded two/four-worker workload: at least 32 seeds and two successful supervisor cycles per seed, at least 64 completed finite tasks per cycle, overlapping parents/children, capacity-full rejection and reuse. Keep ledgers bounded to a cycle and use stable gates to check queued plus executing capacity. Check refused factories never run, receipts can disappear without releasing work, and every successful admitted ID reaches the expected result accounting after shutdown.

Add required scenario families for root/descendant admission against drain/force/expiry; operation completion/cancellation/deadline/drop; readiness approval/acknowledgement/critical exit/drain; complete task/cleanup failure retention; receipt, waiter, last-owner and caller-owned driver drop; and delayed-finish versus escalation. Explicit permutations cover required branches; seeded yields explore their overlapping execution. Named controlled schedules establish causality using acknowledgements, not later timestamps. Keep operation-child and process-descendant lifetimes separate.

Validate the harness with a capacity-one schedule on a disposable descendant-independent semaphore variant and a deliberately wedged child. The correct library must pass, the variant must fail the intended assertion, and the stuck fixture must be terminated by the external watchdog. Check generic/sanitized diagnostics with returned-error sentinels and retain the default panic hook only with generic fixture payloads. Repeat the full correct corpus in three fresh processes per worker count.

## Concrete Steps


Run commands from `/home/aa/Documents/batter`. Start with `cargo test -p batter --test scheduling --locked` as the discoverable target. The Python runner will accept `--binary`, `--workers`, `--seed` and `--schedule`; document its final exact commands in docs/testing.md. Keep every case wait under an independent timeout, the profile at 120 seconds and the external process deadline at no more than 150 seconds. Output includes scenario version, worker count, seed, case/family, checkpoints and count summaries.

After focused success, run `bash scripts/verify.sh`, `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`, build `cargo build -p batter-axum --example http_service --locked`, then run `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` in default, `--signal SIGINT` and `--deadline` modes. Run `scripts/jig work check`, inspect `work evidence` and `work gates`, and finish backend verification with `scripts/jig check test`. Use the plan ID generated by `work start` for structured commands.

## Validation and Acceptance


A profile must assert its workload minimums, all required family execution, shared-capacity bounds, monotonic readiness, concrete error identity/category retention and conservative cleanup after uncertain task termination. Counts alone are insufficient. A named replay must repeat the capacity mutant's intended assertion failure; nonzero compilation or unrelated timeout is not valid mutation evidence. A wedged child must fail the watchdog protocol with observed kill/reap and bounded checkpoint capture. Single-seed runs explicitly do not claim full-corpus minima. Stress traces do not promise exact scheduler replay; reduce any newly discovered product defect to a deterministic regression.

Record exact command/toolchain/platform/lock hash, test totals and failed attempts in docs/validation.md. Update docs/guarantees.md for evidence boundaries and docs/status.md with implemented facts. Linux/macOS claims must match actual execution; hosted CI and macOS stress are unverified unless executed. No database or HTTP load claim follows from this work.

## Idempotence and Recovery


New checks are repeatable and provision no services. Child process handles own kill/reap on failure. The mutation runner works in an isolated temporary copy; do not patch production sources in place. Re-run a failed case with its captured seed and schedule without retry-to-green. Plans and receipts remain connected; append-only Jig records are never rewritten. Preserve unrelated user edits and the approved Beads scope.

## Interfaces and Dependencies


Use existing Tokio barriers/channels/semaphores and native futures. The test runner may use Python 3's standard library, already required for repository verification. No library-level globals, panic hooks, new runtime abstraction or public API should be introduced. Per-profile and per-case integer IDs and a fixed action vocabulary are diagnostic data; internal reports and raw application errors are not serialization formats.

Plan created 2026-09-08 from the current tightened Bead; update all affected sections when implementation discoveries change execution details.

Execution update 2026-09-08: Added scheduling/{ledger,workload,admission,transitions,closure_races,operations,readiness,failures,ownership,escalation,families,profile,support}.rs and two Python runners. The discoverable target has eight entries. Each full profile asserts 32 seeds, 64 workload cycles, 4,096 task results and all eight families; final six fresh-process results are in validation/local/batter-953-final. Mutation artifacts are in validation/local/scheduling-mutation-1.98.1 and validation/local/scheduling-mutation-1.94.0. This records implementation evidence without moving delivery scope out of Beads.

Final verification update 2026-09-08: The final matrix passed on 1.98.1 (37.349 s) and 1.94.0 (39.043 s), each running 162 core entries, 183 workspace entries and two doctests. Explicit negative fixtures were killed/reaped at 3.001 s (blocked timer) and 3.012 s (unjoined finite receipt), retaining their required event records. No macOS or hosted scheduling run is claimed.

Completion 2026-09-08: batter-953 is closed. Jig work finish returned success with fresh required gates, plan receipt receipt_01M20TKN23DP0B209CKGAZP448 and session receipt receipt_01M20TKN2N0TYPT1E1J2YV4RKA. The final acceptance audit matched all ten criteria to source and execution evidence; macOS/hosted scheduling remains explicitly unverified.
