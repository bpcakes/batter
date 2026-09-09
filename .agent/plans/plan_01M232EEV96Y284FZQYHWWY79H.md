# Extracted cleanup cancellation and abandonment coverage

Owning Bead: batter-vtx. Baseline: 22848ea6884f13b014c74850e7bd888e0b8a94d8.
Repository: /Users/aa/Documents/batter. No commit or publication is authorized.

## Progress

- [x] Confirmed review follow-ups remain at the baseline.
- [x] Clarified take_cleanup rustdoc, contracts, usage, status and crate guidance.
- [x] Added explicit extracted-cleanup coverage and strengthened capture/waiter checks; 13 focused tests passed.
- [x] Both toolchain matrices and five HTTP smoke modes passed; recorded evidence.

Final Jig gate and task closure results belong in the append-only finish resolution.

## Surprises & Discoveries

Abandonment cancellation is intentional. The missing contract is that extracting
finalizers does not detach operation tokens captured by those hooks. Normal
shutdown also cancels process operations before cleanup. No runtime change is needed.
One aggregate wake count could hide a missing notification behind a redundant one;
independent probes preserve per-waiter coverage while permitting redundant wakes.

## Decision Log

Preserve ownership and cancellation semantics. Finalizers await native teardown or
use a fresh OperationContext::new under the existing CleanupBudget. The new public
test checks a cancelled process token, inert owner drop, and completed independent
asynchronous teardown after extraction and owner drop. The capture-order test now
observes both component and cleanup captures under direct and unpolled driver drop.
Each readiness/drain/cancellation waiter has its own probe, requiring at least one
wake outside the lock and a ready result afterward.

## Outcomes & Retrospective

Implementation and verification are complete: 13 focused tests and 423 test/doctest
executions on each of Rust 1.98.1 and 1.94.0, plus all five HTTP smoke profiles,
passed on macOS arm64. No dependency, API signature or production runtime behavior
changed. Linux execution remains unverified. Final Jig receipts and gate closure
will be retained in the finish resolution without editing files during the checks.

## Validation and recovery

Run cargo test -p batter --locked --lib --test lifecycle_state (13 tests).
Run bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh.
Build cargo build -p batter-axum --example http_service --locked; run
python3 scripts/smoke_http.py --binary target/debug/examples/http_service in
default, --signal SIGINT, --deadline, --warn-filter and --warn-filter --deadline modes.
Record exact executed platform and unchanged Cargo.lock hash in docs/validation.md;
do not infer Linux execution. Run scripts/jig work check, evidence and gates for
this plan and finish backend verification with scripts/jig check test. Freeze all
tracked files during Jig checks. Fix failures without relaxing semantic assertions;
commands are rerunnable. Finish the plan, then close and export batter-vtx.
