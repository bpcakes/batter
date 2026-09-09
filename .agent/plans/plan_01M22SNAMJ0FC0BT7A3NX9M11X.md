# HTTP redaction and filtered-operation assertions

Owning Bead: batter-faj.7.

## Progress

- [x] Assert redaction over the full nested-observer log capture.
- [x] Reject INFO operation completions in WARN-filtered smokes, allowing WARN failures.
- [x] Add focused negative controls and update testing/status/validation evidence.
- [x] Run both Rust matrices, all Python controls, five live smokes and final Jig gates/tests.

## Decisions and discoveries

Reuse a private test helper for both whole-capture and per-event redaction checks.
Parse the configured example formatter's level, including its optional timestamp.
The timestamped negative control failed against the first parser and passed after
correction. The first full Python run encountered sandbox EPERM in existing Jig
signal-handler writes; the host rerun passed. The first Jig profile was invalidated
by an overlapping validation-doc edit; the stable-checkout rerun passed all targets.
Failed attempts remain in the task-local logs.

## Outcomes and validation

Seven focused Python controls and three middleware-edge tests passed. Full Python
discovery passed 38 entries. Rust 1.98.1 and 1.94.0 each passed 391 test/doctest
executions plus formatting, Clippy and rustdoc. All five rebuilt-example HTTP
smokes passed. The final scripts/jig check test passed all 391 executions.

The verify profile passed all five targets in run_01M22TT843RRGX567B1H3HRWPH;
work evidence and work gates both reported passed/fresh after the final test.
Exact commands and limitations are in docs/validation.md. Logs are under
.agent/tmp/batter-faj.7. No runtime API, implementation or dependency changed;
existing Git staging remains untouched. macOS/hosted, live concurrent-connection
and sink-specific exporter behavior remain unverified by this follow-up.
