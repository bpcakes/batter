# HTTP test oracle and diagnostic corrections

Owning Bead: batter-faj.6. Correct the two accepted review findings without
changing middleware behavior, public APIs or dependencies.

## Progress

- [x] Read review evidence, repository guidance and existing test utilities.
- [x] Require event-local fields in process and related formatted-log assertions.
- [x] Retain all request/supervisor/server diagnostics after completed teardown.
- [x] Run regression controls, both Rust matrices, five HTTP smokes and Jig gates.
- [x] Record validation and close tracked work; leave existing staging untouched.

## Surprises & Discoveries

The shared observation assertion and live readiness assertion also search whole
formatted lines. Strengthen those alongside the smoke oracle. The readiness
fixture already saves all three results; only the assertion discards evidence.
The initial Python negative-control run found that a latency substring accepted
other_latency_ms. Requiring a whitespace-delimited latency_ms key fixed it; all
four Python control entries then passed without weakening the fixture.
The task-local diagnostic injector first failed compilation because its synthetic
server error lacked a unit success type; correcting that annotation allowed the
old/new expected-failure controls to run. Both restored the fixture byte-for-byte.
The revised assertion retained all three injected failures; the old one hid the
request error. Logs retain the failed injector attempt and both control results.

## Decision Log

Use the completion message as the event boundary in tests of the configured text
formatter. Direct event visitors remain the independent structured oracle.
Check all saved readiness results in one assertion after teardown; report every
result on failure. No new generic error-combination helper or dependency is needed.

## Outcomes & Retrospective

Both review findings are addressed. All HTTP text oracles require event-local
fields, with four new Python control entries included in existing CI discovery.
Readiness teardown now reports all saved outcomes together. A task-local live
failure-injection comparison proved that the old assertion hides the primary
request error and the new one retains request, supervisor and server failures.
The fixture was restored byte-for-byte before final checks.

Python discovery passed 35 entries; focused adapter tests passed 44 entries.
Rust 1.98.1 and 1.94.0 each passed 391 test/doctest executions plus formatting,
Clippy and rustdoc. All five rebuilt-example smokes passed. The tracked verify
profile passed all five targets (run_01M22QJY75EVR6AD2VZ4R041N7), and evidence/gates
reported passed/fresh. The final scripts/jig check test passed 391 executions.
Exact commands, diagnostic-control limitations and platform limits are recorded
in docs/validation.md; logs are under .agent/tmp/batter-faj.6. No runtime API,
implementation or dependency changed. Existing staging was preserved; the fixes
remain uncommitted. Hosted/macOS and concurrent-connection execution remain
outside this follow-up.

## Context and plan

scripts/smoke_http.py validates the example process; add focused negative controls
in scripts/test_smoke_http.py, automatically discovered by the existing Python CI
command. crates/batter-axum/tests/observation/support.rs and the example readiness
tests own the corresponding Rust text assertions. Complete teardown before any
combined assertion. Update docs/testing.md, docs/status.md and docs/validation.md.

## Validation and recovery

Run focused Python controls and adapter all-targets tests; run bash scripts/verify.sh
under 1.98.1 and 1.94.0, rebuild the example and execute all five smoke profiles.
Finish with fresh Jig work check/evidence/gates and scripts/jig check test. Keep
failed attempts as evidence, repair failures without relaxing tests, and record
hosted/macOS execution as unverified. Preserve prior files and Git staging.
