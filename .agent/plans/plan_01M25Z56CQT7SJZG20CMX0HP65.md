# Retain wait diagnostics at phase cancellation

Owning Bead batter-538, following batter-rv8. Baseline HEAD486e0b0f9f4c4439077418715843b30042205f7e. Preserve the existing staged binary patch against /tmp/batter-diagnostics-staged-before.patch. No staging, commits, publication, or external messages.

## Progress
- [x] Research resolved tracing implicit Dispatch conversion, Tokio cancellation and Notify behavior; inspect current ownership and assertions.
- [x] Remove observation event/wire private timers and retain interrupted-wait diagnostics; add exercise/disconnect/reconciliation controls including slow report acquisition.
- [x] Restore terminal handler-count assertions with late-entry negative controls; explicitly budget delayed-success drain; route raw subscriber conversions through the shared helper.
- [x] Update contracts/status/references/validation/Bead and pass both full matrices, ten rebuilt smokes, concurrent repetitions, semantic mutations and final fresh Jig gates.

## Surprises & Discoveries
WithSubscriber accepts Into<Dispatch>; raw subscriber arguments in foundation telemetry, cleanup observations, scheduling failures and adapter telemetry bypass the helper despite no explicit Dispatch::new at those sites. Already-built Dispatch clones do not. Tokio drops timed-out owned futures; cancellation diagnostics therefore belong to their owned wait guards, independent of which containing deadline expires. Notify stores a permit for notify_one before awaiting, so the current single-waiter check is not a demonstrated lost wakeup; event history plus notify_waiters and a pre-check notification future also supports concurrent event waiters. Existing 3s diagnostic timers compete with 2s exercise/1s disconnect/remaining teardown and require an unstable panic shape. Final handler counts were left at EOF after moving report waits.

## Decision Log
Keep deadline policy in fixture phase owners. Remove event and client read timers; use private owned pending-event/read guards that retain their description, partial wire bytes and event snapshots on Drop into fixture diagnostics, with nonpanicking stderr output for process watchdog visibility. Driver retains capture and diagnostics after joined future destruction. Real server/body oracles and intentional bounded pending-read/native-ack/disconnect checkpoints remain. Missing reconciliation uses the governing timeout and must retain named evidence for both fast and deliberately delayed reports. Terminal counts run after the report in both suites; controlled late event injection verifies the oracle moved, without claiming it is an actual routing defect. Delayed-report drain gets an explicit400ms allowance, separate from100ms cancel/reap; total3.4s remains below3.5s teardown. Wrap every remaining raw WithSubscriber conversion through test_dispatch::new, retaining existing capture semantics and subscriber filtering.

## Outcomes & Retrospective
Research and implementation complete. Both full toolchains passed 693 Rust test/doctest executions and all Python/format/Clippy/rustdoc phases; 10 rebuilt smokes, 470 concurrent repeats and all 12 semantic mutation cases passed. Final Jig verify profile passed all five targets with fresh receipts, including api:test receipt_01M25ZS7255DRSPAW3ZXHGSSF3; run_01M25ZQPRRWBJW1CXAMSNBHP23. Bead closed. Staged patch remains byte-identical; Cargo.lock unchanged and local Markdown links resolve. One nonblocking file-budget notice remains on the existing raw scenario file. No production API, dependency graph, platform or publishing scope change.

## Work and validation
Edit observation resource/client/driver/cases/fixture/limits sources and tests, raw scenarios/server and tests, shared HTTP delayed budget, and raw subscriber call sites. Introduce a private diagnostic source only if it keeps ownership clear; do not build a general async framework. Test cancelled missing events, partial wire marker/EOF, disconnect and reconciliation, destructor ordering, final counts, and remaining subscriber sites. Run focused tests before bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh; rebuild five HTTP smoke profiles per compiler. Use concurrent repeats for timing controls and affected tracing binaries. Run the existing isolated Axum mutation runner; require exact intended failures. Update docs/tracker before final Jig work evidence/gates/check/finish, preserving the index and root Cargo files.

Evidence: docs/validation.md section Cancelled wait diagnostics and terminal assertions; /tmp/batter-diagnostics-verify-{1.98.1,1.94.0}.log; /tmp/batter-diagnostics-stress.json; /tmp/batter-diagnostics-smokes.json; /tmp/batter-http-graceful-mutation-0errvs_p/evidence.json. Initial Clippy unused-clone failure and temporary repetition inventory failure were corrected before final successful checks.

## Recovery and limits
Keep earlier work and append-only Jig state. No runtime-death or non-yielding async-drop guarantee; independent Unix watchdog remains. MacOS/hosted and live PostgreSQL validation require separate actual evidence. Do not replace missing completion with a narrower claim.
