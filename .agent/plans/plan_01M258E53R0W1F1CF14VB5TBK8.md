# Prove component and HTTP transport ownership

This living ExecPlan follows `.agent/PLANS.md` and owns execution for Beads
`batter-zu8` and `batter-u0m`. Beads retains acceptance and delivery status.

## Purpose

Make the limits of joining a registered task executable: compare an internally
joined component with a wrapper that leaves a child running, then measure the
same distinction with real Axum HTTP/1.1 connections. No production API extension
is intended. A body finishing, a socket closing, and a server wrapper returning
are separately observed events.

## Progress

- [x] 2026-09-10: inspected both Beads, clean worktree, crate guides and existing watchdog.
- [x] 2026-09-10: six component comparisons pass on macOS / Rust 1.98.1.
- [x] 2026-09-10: ten HTTP scenarios, deliberate-stall control and reused controls pass (51 tests).
- [x] 2026-09-10: inspected pinned registry sources; wrote ADR-008 and updated contracts/testing/status/references.
- [x] 2026-09-10: both compiler matrices passed (650 executions each); all five HTTP smokes passed on each toolchain.
- [x] 2026-09-10: final Jig verify gate fresh/passed; api:test receipt `receipt_01M259EZ2QQ71MW2B1W52J966Y` passed 650 executions.
- [x] 2026-09-10: audited every acceptance criterion, updated validation/contracts and closed/exported both owning Beads.

## Surprises & Discoveries

Existing non-yielding tests have a PID-bound stdin launch protocol, parent-death
monitor, independent emergency deadline, bounded capture, kill and reap. Reuse
these controls rather than build another process runner. Existing lifecycle tests
already prove direct panic/abort policy; new comparisons must connect that policy
to independently observed descendants.

On this macOS arm64 checkout, a pending HTTP body survives serving-wrapper abort
and completes after explicit release. Full client SHUT_RDWR drops pending handler
and streaming body before release. Force cancellation returns 503 with successful
direct join/cleanup. Clippy found an existing 136-byte private startup error result
on macOS; a narrowly documented result_large_err allowance preserves the retained
report before the monitor's existing Arc allocation. New test helpers were split
to meet the existing cognitive-complexity threshold without changing assertions.

## Decision Log

Use test-owned channels and resource Drop acknowledgements for causal ordering.
Keep normal HTTP composition driven directly by lifecycle drain; only a companion
admission case withholds graceful notification to witness a rejected request.
Do not change public APIs or infer connection termination from wrapper abortion.
No durable formats or dependency version changes are needed.

Reuse Unix modules by explicit test-only paths from the HTTP integration target,
including their existing runner controls. Existing Cargo/Jig discovery and source
input globs cover them without command or configuration changes. Add http-body as
a direct dev dependency (the lock already resolves 1.1.0) and enable Tokio io-util;
Cargo updates only the adapter's dependency edge in Cargo.lock. No new versions.

## Outcomes & Retrospective

Implementation, focused macOS execution, both compiler matrices, all ten executable
smokes and final Jig gates are complete. Both `work evidence` and `work gates`
reported passed with fresh receipts and no missing/stale/failed requirements.
No new Linux or hosted execution is claimed. No public API or production runtime
behavior changed. The only foundation source edit documents an existing macOS
large-error layout lint exception; all semantic assertions remain enabled.

The zu8 audit maps initialization and joining to the conforming comparison,
post-cleanup request/reply to the nonconforming comparison, all four direct exits
to `check_exit`/`check_cleanup`, and non-yielding containment plus preserved finite,
abandonment and scheduling regressions to the full matrices.

The u0m audit maps separate milestones to State/ControlledBody/Client and direct
report assertions; both admission forms to `keep_alive`; active drain, upload
deadline and forced cancellation to `handler`; body/context independence and
report-before-release abortion to `streaming`/`aborted_report`; full disconnect
and pre-release destruction to `disconnect`; normal direct completion before
cleanup to `clean_report`. `check_observation` checks actual status/outcome/count
at header-time for streams and after teardown for every case. The spawned
exercise result and separately awaited teardown result are retained together.
The reused watchdog and deliberate stall prove independent bounds/reaping;
ordinary Cargo and Jig runs discover all cases. ADR-008, source references,
contracts, guides, status and validation cover the remaining named artifacts.
Both toolchains and all five smoke modes passed; platform exclusions are explicit.

## Context and orientation

`crates/batter/src/lifecycle` owns registered direct tasks and reports, not hidden
Tokio children. `crates/batter/tests/non_yielding` contains Unix process controls.
`crates/batter-axum/examples/http_service.rs` registers `axum::serve` as a critical
task and awaits graceful shutdown on drain. Middleware bounds response creation,
not streaming body transmission. New tests belong in the owning crate's tests.

## Plan of work

First add a component comparison suite: gate child initialization before startup
acknowledgement; gate child termination during drain; prove cleanup runs only
after the conforming wrapper joins. For the nonconforming wrapper, retain an
independent child handle and request/reply channel and prove it still executes
after the direct report and cleanup. Cover early success, returned typed errors,
panic and forced abort with retained task and cleanup outcomes. Existing isolated
non-yielding cases remain the evidence for tasks whose poll never returns.

Then create `crates/batter-axum/tests/http_lifetime.rs` and small fixture modules.
Use loopback HTTP/1.1 clients, retained connection state, controlled handler/body
entry and destruction, and a per-case event log. Cover idle keep-alive, active
handler drain, ordinary additional-request rejection/closure, a synchronized
admission rejection, incomplete upload deadline, pending streaming completion,
forced cancellation, wrapper abortion, and full client disconnect before/after
headers. Assert framing completion separately from EOF and body destruction.
Inspect reports before releasing blocked bodies. Aggregate exercise and teardown
failures. Wrap every real case in the existing independent subprocess controls;
a deliberate runtime stall must fail success validation and be reaped.

Finally update `docs/guarantees.md`, `docs/integrations.md`, `docs/status.md`,
`docs/testing.md`, `docs/references.md`, `docs/validation.md`, and a numbered ADR
linked from `docs/adr/README.md`. Verify source versions using Cargo.lock and
primary Axum/Hyper documentation. All new claims must name tested scope.

## Concrete steps and acceptance

Run from `/Users/aa/Documents/batter`:

    cargo test -p batter --test component_ownership --locked
    cargo test -p batter-axum --test http_lifetime --locked
    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
    cargo build -p batter-axum --example http_service --locked
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service

Repeat the smoke with `--signal SIGINT`, `--deadline`, `--warn-filter`, and
`--warn-filter --deadline`. Inspect `scripts/jig work evidence` and
`scripts/jig work gates`, run `scripts/jig work check` for required gates and a
fresh successful api:test receipt. Normal Cargo discovery must run all new cases.
Record exact platform/toolchains, lock hash, failures and final outcomes. Do not
claim Linux execution from macOS or vice versa, nor hosted CI from local runs.

## Idempotence and recovery

Tests use ephemeral loopback ports and child processes, no persistent services.
Process guards kill and reap on failure. Preserve original semantic assertions
when debugging. Retry failed commands after fixing their cause and record the
failure. Do not commit or publish. Finish by inspecting the diff and exporting
Beads with `br sync --flush-only`.

Verification logs for this run are under `/tmp/batter-ownership-Z5h9dT`.
Jig plan ID is `plan_01M258E53R0W1F1CF14VB5TBK8`; baseline is
`495e46fdbfd2009edb56d2e00d838407465a0c72`. This plan file is the only live
execution document; the temporary input used to create it was removed.

## Interfaces and dependencies

Use native Supervisor, ShutdownSignal, ShutdownReport, OperationContext, Axum
Router and serve, Tokio channels/tasks/TcpStream, and test-only observation.
Keep shared process controls independent of batter and its adapters. Existing
workspace test commands discover new integration targets automatically; update
Jig evidence inputs if a shared runner is moved outside their existing globs.

Revision 2026-09-10: recorded implemented fixtures, measured transport behavior,
the macOS lint finding and focused outcomes; full acceptance remains open until
the complete verification and delivery audit finish.

Final revision 2026-09-10: recorded full acceptance audit, two passing 650-execution
compiler matrices, ten passing process smokes and fresh final Jig receipts. Guide
edits invalidated the initial Jig pass, so all five gates were refreshed after the
final guide and validation edits. Test commands/configuration, relevant environment
and toolchains were unchanged during that refresh. Logs include `jig-final.log`.

The tracker closure subsequently invalidated Jig's input digests for every target,
even though source, dependency and test inputs did not change. The post-closure
refresh is `scripts/jig work check --plan-id plan_01M258E53R0W1F1CF14VB5TBK8 --json`,
logged in `jig-closed.log`; its authoritative receipt IDs and final freshness are
in `.agent/state/receipts.jsonl` and `scripts/jig work evidence`. Run it after all
tracker/documentation edits, inspect fresh successful gates, then use
`scripts/jig work finish` without additional content edits. This avoids another
self-invalidating documentation update. The Beads are closed, and implementation
acceptance is already proven; this last step refreshes repository bookkeeping.
