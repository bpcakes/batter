# Centralize lifecycle transitions and abandoned ownership

Owning Bead: batter-7dm. Follow .agent/PLANS.md. Baseline HEAD is 19aa9ff8e72995add6d75fd65d4c7790ebafe872 with the already-authorized terminal-admission repair uncommitted. Preserve those changes and the user-owned untracked .reviewignore. No commit or publication is authorized.

## Purpose

Make readiness irreversible after coordinator completion and make abandoned startup observable. All readiness/admission transitions share one private mutex-protected state, and a supervisor owns synchronous abandonment signaling from construction until the driver finishes or is dropped. No dropped owner invokes asynchronous finalizers or fabricates a report.

## Progress

- [x] Inspected every state write and ownership path; captured the existing source under /tmp/batter-7dm-baseline.
- [x] Added two failing abandonment regressions; implemented the private transition boundary and ownership transfer; focused tests pass.
- [x] Added seven unit/four public tests, including a rejected monotonicity mutation; updated contracts and guidance.
- [x] Both Rust matrices and all three HTTP smoke modes passed; recorded platform, counts and unchanged lockfile in docs/validation.md.

Final Jig gates and closure are recorded in this plan's append-only state events; freeze this body during those checks and record the final outcome in the finish resolution.

## Surprises & Discoveries

Readiness currently uses AtomicU8 while other admission facts use a mutex; the final STOPPED store bypasses that mutex. A dropped unstarted supervisor closes its queue but leaves readiness waiters pending. The prior repair remains in the working tree and must be retained. The initial all-mutex implementation passed functional tests but a bounded readiness-read test failed with Timeout while admission was held. Native enqueue wakes its receiver under that lock, so an atomic read snapshot was retained with all publication serialized by the same mutex. No unsynchronized state writer remains. The first Jig profile passed code checks but caught the six earlier admission regressions exceeding the 800-line test-file budget. Moved those tests unchanged into a dedicated process_ownership/terminal_admission.rs module and retained the existing limit; repeated verification after the move.

## Decision Log

Use the existing short-held mutex as the sole owner of readiness and admission transitions. Keep independent startup, forced-cancellation and failure facts; rename running to driver_started. Expose only operations and a bounded-admission guard from a private lifecycle/state.rs module. Keep an atomic published readiness snapshot for readers; its only writer is private and requires a MutexGuard. This preserves nonblocking readiness reads during enqueue. Explicit notifications and token cancellation execute after releasing the guard; native channel enqueue can wake the receiver while admission is held.

Construct an EmergencyShutdown ownership guard with Supervisor, store it before application captures, and transfer it using Option::take into a private CallerOwnedDriver wrapper. Its first-field guard precedes inner-future destruction without relying on async capture field order; the existing pin-project-lite dependency supplies safe projection. Dropping a never-started owner withdraws readiness, cancels tokens and wakes readiness waiters, while cleanup stays explicitly awaited. Stopped remains reserved for completed coordinators; abandoned startup reports Draining. Existing completion-observer semantics are unchanged.

## Context and milestones

The virtual workspace foundation is crates/batter. lifecycle.rs currently owns ShutdownHandle, Supervisor, state writes and task supervision; lifecycle/process.rs enqueues bounded finite work; lifecycle/driver.rs owns the completion monitor. Move state facts, notification/token operations and admission classification into lifecycle/state.rs, with private fields. Keep public signatures unchanged. Shared admission and enqueue must remain one locked operation; active-scope expiry and capacity release keep their existing ordering.

First add public abandoned-owner regressions in a focused integration target and run them before implementation to prove the missing signaling. Then replace raw state accesses with private state operations, transfer the ownership guard without starting factories, and run focused lifecycle/process tests. Add unit transition/admission tables over real state operations and controlled concurrent request/completion cases. Since all transitions use one mutex, deterministic serialized order checks cover the possible transition orders; retain an actual concurrent test as wiring coverage.

Update docs/guarantees.md, docs/architecture.md, docs/status.md, docs/testing.md, docs/validation.md and nearest ownership guidance. The contract must preserve descendants during drain, name/closure/startup/capacity precedence, terminal reports, explicit finalization, no automatic error logging and Unix-only scope.

## Validation and acceptance

From /Users/aa/Documents/batter run cargo test -p batter --locked --lib --test lifecycle --test process_ownership --test lifecycle_state. Expect terminal-state absorption, valid startup permutations, readiness waiter wakeup, inert rejected factories and unchanged existing failure assertions. Before code changes the abandoned-startup tests must fail. Then run bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh. Build cargo build -p batter-axum --example http_service --locked and run python3 scripts/smoke_http.py --binary target/debug/examples/http_service in default, --signal SIGINT and --deadline modes. Record exact executed platform and unchanged Cargo.lock hash; Linux execution is not inferred from macOS.

Use scripts/jig work check, work evidence and work gates for this plan, and finish backend verification with scripts/jig check test. Freeze all tracked files during read-only Jig gates; the previous task demonstrated that concurrent evidence edits invalidate receipts. Finish the plan once gates pass, recording final test/evidence identifiers in the finish resolution if further tracked evidence edits would stale gates.

## Recovery and outcomes

No dependency graph, database or public signature changes were made. Pre-existing edits remain intact. Commands are rerunnable; do not weaken semantic assertions to make tests pass. Source snapshots are diagnostic backups, not permission to overwrite later user work. Implementation is complete. The focused run passed 58 tests; initial abandonment tests and a temporary unconditional-drain mutation were rejected. Rust 1.98.1 and 1.94.0 matrices each passed 184 foundation / 205 workspace entries and two doctests on macOS arm64. Rebuilt HTTP smoke passed SIGTERM, SIGINT and deadline modes. Linux execution is unverified. Jig gate completion and the final backend test receipt belong in the finish resolution.
