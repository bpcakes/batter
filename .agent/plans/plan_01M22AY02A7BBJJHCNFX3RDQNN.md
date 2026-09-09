# Configure HTTP observation severity per response

Owning Bead: `batter-faj.2`. Git baseline: `19aa9ff8e72995add6d75fd65d4c7790ebafe872`.
The checkout already contains the completed, uncommitted `batter-faj.1` split;
preserve that work. Beads owns delivery scope and acceptance.

## Progress

- [x] Inspect the current adapter and upstream response-extension semantics.
- [x] Add the typed severity override and a compiling/runnable example.
- [x] Prove defaults, override levels, redaction, rendering and destruction.
- [x] Update contracts/references and execute both toolchains and HTTP smoke.
- [x] Record validation evidence and prepare the final repository gates.

Final gate and closure status is maintained in Jig receipts and Beads. Run the
final commands below, require fresh/passed gates, then close through the CLIs.
Do not edit this plan after final gate capture merely to duplicate their status.

## Surprises & Discoveries

Axum 0.8.9 already supports application-to-middleware metadata via response
extensions. Request extensions and headers are distinct and must not configure
observation. The earlier split preserved the existing 5xx-to-WARN default.

The focused suite passes 33 adapter tests and two doctests. An initial test-only
tuple construction put StatusCode between response parts; Axum requires it first
when a separate body follows. Correcting the tuple order resolved compilation.
Temporarily replacing override capture with None compiled but failed the selected
level assertion (exit 101). The source was restored byte-for-byte; evidence is
`.agent/tmp/batter-faj.2/mutation-ignore-level.log`.

The initial full default-toolchain run passed every test but failed Clippy's
complexity limit on expanded tracing branches. Moving severity emission into
noncapturing functions selected by level keeps individual static callsites
without a lint exemption or application callback. The final 1.98.1 and 1.94.0
scripts both passed all 380 test/doctest executions, formatting, Clippy and docs.

## Decision Log

Use `HttpObservationLevel(tracing::Level)` as an explicit response extension,
read by both observers through the shared helper after response construction.
Keep status/outcome classification and exactly one event per installed observer.
No lifecycle state, callback, route inference, new middleware or dependency is
needed. A missing response cannot carry an override, so dropped futures keep
WARN. Select a native tracing callsite for each level while preserving the
existing span and dispatch protection. Application handlers or inner middleware
own policy and may insert/remove the extension before returning to observation.

## Outcomes & Retrospective

Implementation and independent verification are complete. Both supported
toolchains passed with zero failed/ignored tests; all three HTTP smoke modes
passed after rebuilding with 1.98.1. Cargo.lock is unchanged. Exact results and
limits are in docs/validation.md and `.agent/tmp/batter-faj.2/`. The remaining
repository gate commands and closure are recorded through Jig/Beads so their
receipts correspond to this final source/documentation snapshot. This change
adds one response metadata type; existing observer signatures/defaults remain
compatible and there is no new dependency, lifecycle coupling or dynamic policy
callback during destruction. macOS, hosted and extended transport evidence are
not claimed.

## Context and orientation

`crates/batter-axum/src/lib.rs` owns public HTTP APIs and private HttpObservation.
`observe_response` is shared by standalone observation and legacy request_scope;
all observation ownership stays inside with_current_dispatch. Admission stays
unchanged. Tests are in `tests/observation/`, sharing `tests/support/capture.rs`.
The runnable composition is `examples/http_service.rs`.

## Plan of work and concrete steps

Add rustdoc and an Axum Extension response example for HttpObservationLevel.
Read the override from the completed response without consuming it. Store the
selected level on the private guard; never execute application callbacks in Drop.
Demonstrate explicit readiness policy in the runnable example. Add focused
tests for each level and default, readiness/admission, rewriting/removing overrides,
ignored request metadata, and first-poll subscriber/destruction. Keep existing
failure assertions and limits intact. Update the relevant package guide, contract,
status, integration/usage guidance, testing and primary references.

## Validation and acceptance

Run `cargo test -p batter-axum --locked`, then `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`. Rebuild the HTTP example and run
`scripts/smoke_http.py` in default, SIGINT and deadline modes. Capture logs under
ignored `.agent/tmp/batter-faj.2/`. Check Cargo.lock remains unchanged. Update
docs/validation.md with actual Linux results and remaining macOS/transport limits.
Run `scripts/jig work check --plan-id plan_01M22AY02A7BBJJHCNFX3RDQNN`, inspect
work evidence/gates, and finish backend verification with `scripts/jig check test`.
Finish all source/docs/plan edits before the final gate refresh because Jig
input digests include documentation. Then close via the tracker/Jig CLIs.

## Idempotence and recovery

All changes are local and additive. Repair failures without weakening semantic
tests. Do not commit, publish, update downstream projects, or rewrite existing
tracker/Jig state. Preserve the completed first task and its historical plan.

## Interfaces and dependencies

New response metadata type HttpObservationLevel wraps native tracing::Level.
Existing observer and admission signatures and the Rust 1.94 minimum stay intact.
Response body streaming, universal disconnect behavior and panic recovery remain
outside this work. No new dependency is required.
