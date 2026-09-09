# Retain HTTP observation context independently of filtering

Owning Bead: batter-faj.4. The handler-panic decision and tests also complete batter-4qc. Resolve correlation at the response boundary's first poll, so an optional HTTP span cannot erase an enabled application parent or allow later ambient context to replace it.

## Progress

- [x] Research resolved tracing/Axum/Tower semantics and related delivery tasks before implementation.
- [x] Add mixed-filter completion, cancellation, context-isolation and panic regressions; prove the context tests reject the current implementation.
- [x] Retain a selected context span separately from the HTTP field span and use it for execution and event parenting.
- [x] Exercise the actual example router over sockets in each readiness phase and add mixed-target filtering to smoke validation.
- [x] Update contracts, references, status and validation; run both toolchain matrices, HTTP smokes and Jig gates; close owning work.

## Surprises & Discoveries

- An explicit disabled tracing parent means a root event. Looking up a contextual parent only at Drop would allow unrelated ambient spans to supply identity.
- tracing 0.1.44 recommends Span::or_current for propagating a parent when a child span is disabled. Instrumented futures enter their retained span during both polling and destruction; Batter separately retains the dispatcher around all destruction.
- The consumer integration keeps trusted request identity in an outer application span and still uses the pinned combined entry point. Both split and legacy behavior need coverage.
- Per-layer filtering may hide spans in a sink while another sink retains them; subscriber/exporter presentation remains application-owned.
- The new tests also reject a compiled mutation that chooses `or_current` at event emission: one request loses identity and an originally parentless request adopts an unrelated parent. First-poll selection is necessary, not just a formatting choice.
- The live readiness test initially failed at socket bind because of sandbox permissions. Its approved rerun passed without changing assertions or listener behavior.
- The first final `scripts/jig check test` passed all 387 Rust executions, but Jig rejected its receipt because an independently added `.reviewignore` changed the worktree fingerprint while the target ran. Preserve that file and refresh evidence without changing tests.

## Decision Log

- Keep HTTP fields and retained execution/correlation context distinct. Select the context once at first poll as the enabled HTTP span or current application span; reuse it through polling, destruction and explicit event parenting. Never record HTTP fields into the fallback application span.
- Keep native tracing::Level, all five explicit response severities and existing filtering policy. No severity floor, level re-export or new telemetry abstraction is necessary.
- Preserve existing handler-panic propagation. An unwind before a response is a dropped response future, not a synthesized HTTP 500. Do not install a catcher or global panic hook; verify no panic payload enters Batter telemetry.
- Extract a private example router constructor used by main and example tests. Drive its readiness policy over a separately owned live listener with explicit state transitions; do not add timing switches to the runnable application.
- Keep body streaming/disconnect/shutdown transport work in batter-u0m; the new readiness transport test proves only the stated response policy.
- The smoke script exposes the fixed `--warn-filter` profile (`info,batter=warn`), so expected filtered events have an explicit test oracle.

## Outcomes & Retrospective

Implementation and semantic validation passed: Rust 1.98.1 and 1.94.0 each
executed 387 passing tests/doctests, and five rebuilt HTTP process smoke modes
passed. Both pre-fix and late-parent-mutation failures provide regression evidence.
No dependency or public API changed. Research preserved application-owned identity,
severity and exporter policy; the fix removes implicit ambient-context dependence
at the HTTP response boundary.

The initial tracked verify profile passed, followed by a valid final backend test
rerun after `.reviewignore` appeared during the first attempt. That new path made
every target input digest stale, requiring a fresh profile. Attempts to refresh
only file-budget through the legacy tool and standalone target were rejected by
Jig's CLI (unsupported native tool and a work-plan identity mismatch); neither
executed a failing semantic check. Logs retain those outcomes. The refreshed profile passed all five targets,
and both final evidence/gates reports are passed and fresh. The final backend
command also passed all 387 executions with a valid receipt. Evidence is retained
in `.agent/tmp/batter-faj.4/jig-work-refresh.log`, `jig-final-test.log`,
`jig-final-evidence.json` and `jig-final-gates.json`. Owning Beads are closed as part
of this handoff. All changes remain uncommitted; the independently added
`.reviewignore` is preserved.

## Context and orientation

crates/batter-axum/src/lib.rs contains the shared observer and admission functions. Its HttpObservation guard originally owned a filterable span and emitted with that explicit parent; it now retains an independent context span. tests/observation contains direct-field, lifetime, severity and composition checks; tests/support/capture.rs captures formatted event scopes under a scoped dispatcher. examples/http_service.rs composes request identity, observation, probes and admission. The consumer and related batter-in2/batter-8jr tasks reinforce that identity and exporter policy remain application-owned.

## Plan of work

First add tests for per-target filters with enabled application spans. Move futures between independent request spans and subscribers after first poll, and test absence of an original application span. Assert both HTTP completion and nested destruction stay with the original identity, and application status fields are not overwritten. Add unwind tests with Tokio JoinError and retained OperationContext. Then retain the context with Span::or_current while leaving the original HTTP span as the sole target of HTTP span record calls. Extract the existing example router into a private constructor and test its exact handlers over an owned loopback listener during Starting, Ready, Draining and Stopped. Extend normal smoke with the fixed `--warn-filter` profile (`info,batter=warn`) and verify error correlation independently of INFO success events.

## Concrete steps

Run focused cargo test -p batter-axum --test observation --locked tests before and after the context fix. Retain failure logs under .agent/tmp/batter-faj.4. Run cargo test -p batter-axum --example http_service --locked for live readiness policy and cargo clippy for changed adapter/example code. Run bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh, rebuild the example and run all existing HTTP smoke modes plus mixed-filter error checks. Finish with scripts/jig work check, work evidence, work gates and scripts/jig check test. Correct failures without weakening semantics.

## Validation and acceptance

The enabled event retains the first-poll application identity even if the HTTP INFO span is disabled. Other requests cannot supply identity during later polls or destruction. A request with no original parent cannot inherit a later unrelated parent. HTTP fields remain on the event and its own span, with no writes into application fields. Unwinding handlers preserve their panic outcome, cancel admitted contexts and emit exactly one sanitized dropped HTTP event. Real HTTP tests execute the example's unchanged readiness handler and distinguish INFO Starting/Draining from default WARN Stopped responses. Record exact test counts, versions, log paths and remaining platform/transport limits.

## Idempotence and recovery

Preserve all existing uncommitted work. Log failed experiments separately from successful verification. No commits, publishing or downstream changes. Sandbox failures require narrowly approved reruns rather than changing tests. Beads owns scope; this plan records execution only.

## Interfaces and dependencies

Public API and Cargo dependencies remain unchanged. The router constructor and retained observation context are private. Use native tracing, Tokio, Axum and existing dispatch protection; no parallel observer, ambient identity service or panic recovery abstraction is added.
