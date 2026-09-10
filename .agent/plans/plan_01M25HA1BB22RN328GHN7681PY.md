# Measure HTTP/1.1 lifetimes through drain and shutdown

Owning Bead: batter-u0m. Maintain this living ExecPlan under `.agent/PLANS.md`.

## Purpose

Prove the existing Axum response-construction boundary with real loopback sockets: handler admission and destruction, body polling/destruction, HTTP framing, connection closure, direct server outcome and dependency cleanup are distinct observations. No production lifetime API is added.

## Progress

- [x] 2026-09-10: Read the task, package guidance and current native serving composition; claimed batter-u0m.
- [x] 2026-09-10: Implemented eleven subprocess-contained lifetime cases, runtime-stall/reap and dual-failure controls; 14-test focused target and focused Clippy pass on Linux / Rust 1.98.1.
- [x] 2026-09-10: Verified explicit transport outcomes; added ADR-008, contract/status/testing updates and resolved upstream source inspection.
- [x] 2026-09-10: Final complete matrices pass on Rust 1.98.1 and 1.94.0 (631 test executions, 29 intentional ignores each); five HTTP smoke profiles pass on each toolchain.
- [x] 2026-09-10: All five final Jig gates pass; work evidence and gates are fresh, including api:test receipt_01M25JDQ3RVBZHXG2KXV016AE0. Requirement-by-requirement acceptance audit complete.
- [x] 2026-09-10: Bead closed/exported after acceptance audit. Final tracker/documentation edits require an archival Jig gate refresh before work finish.

## Surprises & Discoveries

One archival api:test refresh failed in the unchanged core-test pass, while the
workspace pass (including the complete lifetime target) passed. Jig's preview
omitted the exact assertion. The standalone core command and 100 runs of the
same twelve-test core library binary passed without edits; no root cause or fix
is claimed. Preserve the failed receipt and require a new passing full matrix
and fresh Jig evidence before archival closure. See docs/validation.md.

Measured full close drops the pending handler or blocked response body before any fixture release; write-half close also drops the pending handler with the resolved native default. Forced request cancellation returns 503 and permits clean shutdown; wrapper abort retains a live blocked body/socket at report inspection and skips cleanup. A test-owned native socket wrapper acknowledges actual handle destruction independently of body drop.

The baseline already has register_http and a limited blocked-body abort regression from operational-default work. It does not cover the requested socket and disconnect matrix. The native helper still uses the same axum::serve / with_graceful_shutdown composition.

## Decision Log

During final review, tightened the incomplete-upload case to begin drain after the acknowledged pending body read and before the request deadline. Both full matrices were rerun on that final source.

Use test-owned instrumentation around the native serving future to record its actual returned result; use the public admission and observation middleware unchanged. Ordinary cases deliver drain directly to graceful shutdown. Only the admission control withholds delivery using a test-owned signal. Reuse scripts/scheduling_process.py for a separate process watchdog, including direct Cargo invocation. Keep request and teardown results separately and reconcile both before failing.

## Outcomes & Retrospective

All eleven lifetime cases and two negative controls pass under both complete toolchain matrices. All ten HTTP smoke invocations pass. Final Jig gates pass with fresh evidence. All delivery work is complete; the final work-finish command archives this execution record after refreshing required gates on the final metadata snapshot. The implementation adds no production API, dependency or example behavior; Cargo.lock remains unchanged. New macOS execution remains explicitly unverified.

## Context and Orientation

crates/batter-axum/src/lib.rs admits on a Ready state read and bounds construction of a Response. That response may contain a future-polled body which is outside request ownership. src/serving.rs registers a critical wrapper; native Axum spawns connection tasks internally. The supervisor reports direct task outcomes and skips finalizers after unsafe exits. scripts/scheduling_process.py owns Unix child process termination, output collection and reaping independently of Tokio. Normal workspace discovery is scripts/test_matrix.py and the api:test Jig action.

## Plan of Work

First add crates/batter-axum/tests/http_lifetime.rs as normal Cargo tests launching a private child test through a Python watchdog. Fixtures under tests/http_lifetime own raw HTTP/1.1 sockets, acknowledged handler/body milestones, release controls, the running supervisor and captured HTTP observations. Cover idle keep-alive drain, ordinary second request, withheld-graceful admission rejection, active handler drain, incomplete upload deadline, pending stream cooperative completion, forced handler cancellation, blocked stream wrapper abort, and disconnect before/after response construction. Define full-close versus write-half-close actions explicitly. Preserve body completion, wire framing and EOF as separate events.

Next establish measured disconnect expectations with bounded checkpoints and resource drops before test release. Inspect report before releasing the blocked abort body. Record actual task outcomes and unsafe cleanup skips separately from connection failures. Add the runner to Jig evidence inputs, and a deliberate stalled-runtime child proving watchdog failure and reaping. All tests must fail if the watchdog unexpectedly fires.

Finally add docs/adr/008-http-connection-lifetimes.md (choose the next unused number if occupied), update ADR index, guarantees, integrations, testing, status, validation and references. Run `cargo test -p batter-axum --test http_lifetime --locked`, `bash scripts/verify.sh`, `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`, build the HTTP example and run scripts/smoke_http.py in default, --signal SIGINT, --deadline, --warn-filter, and --warn-filter --deadline modes. Inspect `scripts/jig work evidence` and `scripts/jig work gates`, then run applicable work checks and finish with a current api:test receipt. Record exact platform, versions, lock hash and results.

## Validation and Acceptance

Final audit (2026-09-10): the eleven normal child cases independently establish
idle keep-alive closure; ordinary second-request rejection/closure; positively
witnessed admission rejection with graceful withheld; admitted handler completion
through drain; pending upload destruction by deadline during drain; streaming
completion after request cancellation/deadline; forced handler cancellation with
503 and clean finalization; aborted direct wrapper with actual cancelled JoinError,
no unjoined tasks and skipped finalizers while body/socket remain owned; full
client close before response; full close after headers with no second observation;
and write-half close with client EOF. Stream framing, EOF and resource/socket
destruction have separate observations and causal assertions. Report inspection
precedes blocked-body release and later teardown is reconciled. Negative controls
prove independent watchdog timeout/reap failure and retained exercise plus cleanup
failures. Normal Cargo/Jig discovery runs 14 tests for this target. The final
full matrices on both toolchains each pass 631 test executions and all five HTTP
smoke profiles pass on each toolchain. ADR-008, references, guarantees, integrations,
testing, status and validation reflect this evidence and the unverified macOS,
hosted, HTTP/2, WebSocket, capacity and universal-disconnect scope.


The Bead's acceptance criteria remain authoritative. Each case must acknowledge the intended state before drain, cancellation, abort or disconnect. Causal event ordering proves response completion does not imply body/connection termination and direct completion precedes successful cleanup. Cleanup must run after cooperative direct completion, including cancellation when the server exits normally; wrapper abort must retain cancellation outcome and UnsafeTaskExit skips with zero finalizer calls. The companion routing case must witness rejection at the admission boundary rather than pass on connection closure. Normal discovery and focused Cargo runs must both have an independent process bound and preserve exercise plus teardown failures.

## Idempotence and Recovery

Tests bind ephemeral loopback ports and own all resources. Re-run failed focused cases after repairs; never count watchdog termination as cleanup. No database, published API, migration, commit or deployment changes are intended. Keep `.epicd/` untracked contents untouched. The Jig plan captures baseline 9a49422308e662461524d62132cc81cbbc937c53.

## Interfaces and Dependencies

Use existing Axum 0.8.9, Hyper 1.11.1, hyper-util 0.1.20, Tokio 1.53.1 and http-body; verify actual Cargo.lock and primary sources. Test-only modules do not belong in generic batter-test-support. Python 3 and Unix subprocess/loopback permissions are required.
