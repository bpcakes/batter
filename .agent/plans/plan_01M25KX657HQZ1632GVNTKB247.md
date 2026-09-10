# Reconcile HTTP lifetime observations with latest master

Owning Bead: batter-u0m. Follow .agent/PLANS.md. This task pulls master, preserves
both independent implementations, resolves conflicts and confirms the review
finding; it does not fix the shutdown-synchronization gap.

## Progress

- [x] Saved all local work in stash 5b9d85b2a6bccf155b2fd8701771206a9d828c29; left .epicd untouched.
- [x] Fast-forwarded master from 9a49422 to 486e0b0 and reconciled tracked additions.
- [x] Kept upstream http_lifetime unchanged; renamed local suite to http_lifetime_observations and local ADR to 009.
- [x] Both focused targets pass; both implementations still emit their graceful marker before connections process it.
- [x] Attempted both toolchain matrices: Rust 1.94 passed 666 executions; Rust 1.98 failed unchanged subscriber test, reproduced on iteration 30 and tracked as batter-gg4.
- [x] Rebuilt both HTTP binaries and passed all ten smoke invocations.
- [x] Final Jig gates passed (Clippy, formatting, tests, contract, file budget); api:test receipt_01M25M9P0P0Q9C6FBJCS3J8C7C. This passing invocation does not repair batter-gg4.

## Surprises & Discoveries

Upstream independently delivered batter-u0m and a shared Unix process harness.
The native suite also releases admitted handlers/bodies after a marker inside the
shutdown future, without acknowledging per-connection graceful-shutdown handling.
Its success-marker requirement already protects it against zero-test child runs.

## Decision Log

Keep upstream source and process harness intact. Preserve our socket-drop,
upload-poll, post-deadline stream and write-half-close observations in a separate
normal Cargo target instead of overwriting either implementation. Keep both
append-only Jig histories and upstream tracker records; append local history to
the owning Bead using br. No commit or push is authorized.

## Outcomes & Retrospective

Integration and focused verification complete. Rust 1.94 and ten HTTP smokes
passed. Rust 1.98 full verification exposed unresolved intermittent core test
failure batter-gg4; final Jig outcomes are retained separately. The reviewed HTTP
synchronization issue remains unfixed. No complete two-toolchain pass is claimed.

## Execution and Acceptance

From repository root, run bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash
scripts/verify.sh. Build the HTTP example under each compiler and run all five
scripts/smoke_http.py profiles: default, --signal SIGINT, --deadline, --warn-filter,
and --warn-filter --deadline. Record results in docs/validation.md, then inspect
scripts/jig work evidence and work gates before final work check/finish. Require
no unresolved Git conflict, both suites preserved, clean formatting, current
api:test evidence and a source-grounded confirmation of the finding. Keep the
pre-pull stash as recovery evidence. No new macOS/hosted execution is claimed.
