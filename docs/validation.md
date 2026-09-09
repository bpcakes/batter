# Validation evidence

## HTTP redaction and filtered-operation assertions: 2026-09-09

Bead `batter-faj.7` addresses the two accepted review gaps. The nested-observer
test now checks its full captured output for secrets before inspecting individual
completion events. The WARN-filtered process smoke rejects INFO operation
completions, while allowing WARN deadline completions and application INFO events.
Three additional Python control entries cover rejected INFO events with and
without the example formatter's timestamp, permitted WARN/application output,
and absent operation events. The timestamped negative control failed against
the first parser version and passed after its timestamp handling was corrected.

Executed on Linux x86_64 with Python 3.12.3:

```sh
python3 -m unittest discover -s scripts -p test_smoke_http.py -v
cargo test -p batter-axum --test observation composition_edges --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
python3 -m unittest discover -s scripts -p 'test_*.py' -v
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

The seven focused Python controls and three middleware-edge tests passed.
Rust 1.98.1 and 1.94.0 each passed the full verification script, including 391
test/doctest executions, formatting, Clippy and rustdoc. All five rebuilt-example
smoke profiles passed. The first full Python discovery attempt failed in existing
Jig subprocess controls because the sandbox denied signal-handler writes with
EPERM; the rerun outside that sandbox passed all 38 entries. The first Jig work
check ran its tests successfully but rejected all target receipts because this
validation document changed during the read-only run. A repeat must keep the
checkout unchanged; final gate/test outcomes are recorded in the task plan and
Jig receipts using these commands:

```sh
scripts/jig work check --plan-id plan_01M22SNAMJ0FC0BT7A3NX9M11X
scripts/jig check test
scripts/jig work evidence --plan-id plan_01M22SNAMJ0FC0BT7A3NX9M11X --json
scripts/jig work gates --plan-id plan_01M22SNAMJ0FC0BT7A3NX9M11X --json
```

Logs are under `.agent/tmp/batter-faj.7`; tracked work uses
`plan_01M22SNAMJ0FC0BT7A3NX9M11X`. Cargo.lock remains unchanged at SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Runtime APIs and dependencies are unchanged. macOS/hosted execution, live
concurrent-connection correlation and sink-specific exporter behavior remain
unverified by this follow-up.

## HTTP smoke oracles and test diagnostics: 2026-09-09

Bead `batter-faj.6` addresses the two accepted review findings. Every HTTP process
smoke now checks method, route, status, outcome and latency after the completion
message, independently of fields printed on its spans. The shared Rust text
assertion and live readiness assertions use the same event boundary. Four Python
negative-control entries cover successful events, missing/conflicting event fields
despite correct span fields, and a missing completion message. Existing Python CI
discovery includes them without a workflow change.

The live readiness fixture still awaits teardown before assertions. One assertion
now checks and reports the saved request, supervisor and server outcomes together.
A task-local fault-injection control drove real teardown with request, supervisor
and server failures: the old assertions hid the request error, while the revised
assertion included all three injected messages. The fixture was restored
byte-for-byte after each control, including a first injector compile error from an
unconstrained Result type. That injector annotation was corrected; both expected
failure controls then passed. The initial Python control also caught and rejected
an overly broad latency substring match before it was corrected.

Executed on Linux x86_64 with Python 3.12.3:

```sh
python3 -m unittest discover -s scripts -p test_smoke_http.py -v
python3 -m unittest discover -s scripts -p 'test_*.py' -v
cargo test -p batter-axum --all-targets --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
python3 .agent/tmp/batter-faj.6/check_diagnostics.py
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Python discovery passed 35 entries. The focused adapter run passed 44 entries.
Rust 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0 (`4a4ef493e`, 2026-03-02)
each passed 391 test/doctest executions: 169 foundation, 218 workspace and four
doctests. Formatting, compilation, Clippy and rustdoc passed on both toolchains.
All five rebuilt-example HTTP smokes passed with shutdown status zero. No ordinary
test was failed or ignored in the final runs; the diagnostic controls deliberately
required failure and checked the resulting messages.

Logs and the one-off diagnostic control are under ignored `.agent/tmp/batter-faj.6/`.
Tracked gate and final backend evidence belong to
[the owning plan](../.agent/plans/plan_01M22Q37MQG1B1Z3533Y7PQMZV.md).
No runtime implementation, public API or dependency changed. Cargo.lock remains
SHA-256 `3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Hosted/macOS execution, live concurrent-connection correlation and broader
transport lifetime coverage remain unverified by this follow-up.

## HTTP verification wiring and composition edges: 2026-09-09

Bead `batter-faj.5` addresses the completed review's CI and prerequisite findings
and its bounded observation test gaps. Linux CI now includes both WARN-filtered
HTTP process smoke profiles. macOS CI adds adapter integration/live example tests
and all five HTTP smoke modes. The required all-targets gate continues to run the
live readiness tests; `scripts/verify.sh` and the testing guide now state their
loopback socket and Unix subprocess prerequisites. No test was ignored or removed.

Three new integration test entries cover DEBUG/TRACE overrides filtered under an
INFO subscriber (with a WARN positive control), outer status/severity rewriting
after observation, and retained overrides through nested observers. The existing
complete-router scenario now checks newly added routes and unsupported methods
while Starting/Ready/Draining, including admission 503 before method fallback.
The unwind test additionally requires the original identity on resource destruction.
The live example adds a mixed-filter profile over all four readiness phases: INFO
events are absent and the Stopped WARN event retains its generated response ID.
Both live profiles check unique response IDs and per-response event counts.
Contracts and primary-source notes describe these existing behaviors; no runtime
implementation, public API or dependency changed in this follow-up.

Executed on Linux x86_64 with Python 3.12.3 and unchanged Cargo.lock SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`:

```sh
bash -n scripts/verify.sh
cargo test -p batter-axum --locked
cargo test -p batter-axum --all-targets --locked
cargo clippy -p batter-axum --all-targets --locked -- -D warnings -D clippy::mod_module_files
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Rust 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0 (`4a4ef493e`, 2026-03-02)
each passed 391 test/doctest executions: 169 foundation, 218 workspace (including
42 adapter integration tests and two live example tests) and four doctests. No
test failed or was ignored. Formatting, compilation, Clippy and rustdoc passed
on both toolchains. The example was rebuilt with 1.98.1; all five process smokes
passed with shutdown status zero. A Python/PyYAML check parsed the workflow and
confirmed both filtered profiles in each CI job. Shell syntax passed.

Focused checks and both matrices are retained under ignored `.agent/tmp/batter-faj.5/`
as `adapter-tests.log`, `adapter-all-targets.log`, `adapter-clippy.log`,
`verify-1.98.1.log`, `verify-1.94.0.log`, `http-build.log` and five `http-*.log`
smoke results. Required Jig evidence/gates and final backend results are recorded
with [the owning plan](../.agent/plans/plan_01M22ND6VP41F0KQ8FVQJ0HZFH.md).

The updated CI workflows were not run on hosted runners, and macOS runtime
execution of these changes remains unverified. Broader streaming, disconnect,
connection shutdown and exporter delivery remain outside this task's evidence.

## HTTP context ownership and handler unwinds: 2026-09-09

Beads `batter-faj.4` and `batter-4qc` address a local ownership error: the optional
HTTP INFO span was also the observer's correlation parent. With
`RUST_LOG=info,batter=warn`, an explicit disabled HTTP parent made the WARN event
rooted even when the application's request span remained enabled. Looking up a
fallback only during destruction could instead attach another request's identity.
Research against resolved primary sources and related consumer tasks preceded
implementation; decisions and filtering limits are in [references](references.md).

The observer now selects and retains its HTTP span or available application span
at first poll, reusing it for inner execution/destruction and explicit completion
parenting. The original HTTP span remains the only target of HTTP field recording.
Public APIs, response severity policy, dependencies and the foundation's dispatch
wrapper remain unchanged. The example exposes a private router constructor used
by both main and its readiness test, preserving the runtime composition.

Three correlation regression entries cover standalone/split/combined observation,
enabled/disabled HTTP spans, interleaved requests, completion/drop under another
span and subscriber, retained parent lifetime, untouched application fields and
an absent/filtered original parent. The handler-unwind case verifies a Tokio task
panic, cancelled admitted context and exactly one WARN dropped observation without
an HTTP status or panic payload in tracing. Rust's default panic hook is unchanged.
Two of these tests compiled and failed before the fix; the absent-parent test
already passed. A second compiled mutation, selecting `or_current` at event
emission, failed both the retained-parent and absent-parent assertions. The mutation
was removed before successful final verification.

A live loopback test executes the example's actual readiness router during
Starting, Ready, Draining and Stopped. It checks 503/200/503/503, respectively,
INFO/INFO/INFO/WARN event levels, outcome, route, generated request identity and
one HTTP event per response. A separately owned listener deliberately remains
available through Stopped; this proves response policy, not the binary's connection
shutdown window. Client I/O and teardown have bounds; lifecycle transitions use
acknowledgements and an explicit release channel rather than startup sleeps.
The initial sandboxed attempt failed at socket bind with EPERM. Its approved
rerun passed without changing the test.

Executed on Linux x86_64, kernel `7.0.11-76070011-generic`, Python 3.12.3, with
unchanged Cargo.lock SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`:

```sh
cargo test -p batter-axum --locked
cargo test -p batter-axum --example http_service --locked
cargo clippy -p batter-axum --all-targets --locked -- -D warnings -D clippy::mod_module_files
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 -m py_compile scripts/smoke_http.py
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Rust 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0 (`4a4ef493e`, 2026-03-02)
each passed 387 test/doctest executions: 169 foundation, 214 workspace (including
39 adapter integration tests and the live example test) and four doctests. No
tests failed or were ignored. Both scripts passed formatting, compilation, Clippy
and rustdoc. The example was rebuilt with 1.98.1; all five process smoke commands
passed with signal exit zero. The new `--warn-filter` profile verifies that each
5xx WARN event retains its application request ID and its own HTTP fields with
the HTTP INFO span disabled, while INFO HTTP events are absent.

Ignored logs under `.agent/tmp/batter-faj.4/` retain `context-before.log`,
`late-parent-mutation.log`, `adapter-tests.log`, `adapter-clippy.log`,
`readiness-http.log` (the socket denial), `verify-1.98.1-tail.log`,
`verify-1.94.0.log`, `http-build.log` and five `http-*.log` smoke results. The
1.98.1 tail starts after the initial 39 successful executions; its initial output
and the focused approved live-test success remain in the tool transcript.
Required Jig gate outcomes and the final backend test receipt are recorded with
[the owning plan](../.agent/plans/plan_01M22JCBCPYZWQRDZSBPKQ6CZZ.md).
The first final backend run passed all 387 Rust executions and exited zero, but
Jig rejected its receipt because an independently added `.reviewignore` changed
the worktree fingerprint during execution. That file was preserved; the rejected
receipt and output remain in `jig-final-test-first.log`. This was an evidence
freshness failure, not a Rust test failure.

macOS and hosted CI execution of this change remain unverified. These checks do
not establish arbitrary per-layer/exporter delivery, panic recovery, aborting
panics, damaged shared-state recovery, streaming/body panics after headers,
disconnect handling or transitive connection shutdown. The corresponding broader
transport and metadata/exporter tasks remain open.

Latest evidence: 2026-09-09. Earlier sections retain their historical scope.

## HTTP completion fields independent of spans: 2026-09-09

Bead `batter-faj.3` corrects the dependency of HTTP completion fields on an enabled
INFO span. The observer retains normalized method and the cloned matched route
template and emits method, route, optional numeric status, outcome and latency
on the completion event itself at every severity. Existing span fields remain
for nested context. This does not change response construction lifetime,
admission, response severity policy or the number of completion events. Filtering
of application correlation spans and event delivery remains application-owned.

Three new regressions use an event visitor that never reads span fields. They
cover every severity across standalone/split/combined observers, disabled INFO
parents for WARN/ERROR, success/client/server outcomes, normalized custom methods,
matched and unmatched routes, admission rejection, and dropped futures under a
different ambient subscriber. Paused time checks exactly 25 ms on completed
events. Dropped futures retain WARN, omit status, and reach the first-poll
subscriber. All three tests compiled and failed on the original implementation
because the event contained only a message; the fix made them pass. The initial
test build needed an explicit response return type on the unreachable handler;
that compile error is retained separately and is not regression evidence.
The testing guide now describes coverage and commands rather than duplicating
aggregate inventories; dated executed counts remain here.

Executed on Linux x86_64, kernel `7.0.11-76070011-generic`, Python 3.12.3, with
unchanged Cargo.lock SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`:

```sh
cargo test -p batter-axum --locked
cargo clippy -p batter-axum --all-targets --locked -- -D warnings -D clippy::mod_module_files
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 .agent/tmp/batter-faj.3/smoke_warn.py
```

Rust 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0 (`4a4ef493e`, 2026-03-02)
each passed 383 test/doctest executions: 169 foundation, 210 workspace (including
36 adapter tests) and four doctests. No tests failed or were ignored. Formatting,
compilation, Clippy and rustdoc passed on both toolchains. The example was rebuilt
with 1.98.1 and all three standard HTTP smoke modes passed. An additional live
check with `RUST_LOG=warn` requested `/fail`, asserted one 500 completion with all
HTTP fields and no INFO span context, checked query redaction and exited through
SIGTERM with status zero. The first network attempt was blocked by sandbox socket
permissions before starting a server; the network checks then ran with approval
outside that sandbox.

Logs and the additional smoke script are under ignored `.agent/tmp/batter-faj.3/`:
`regression-before.log`, `test-initial-compile.log`, `adapter-tests.log`,
`adapter-clippy.log`, `event-fields.log`, `verify-1.98.1.log`, `verify-1.94.0.log`,
`http-build.log`, `http-default.log`, `http-sigint.log`, `http-deadline.log`,
`http-warn.log` and `http-sandbox-denial.log`. The final full scripts include the
later status-class and exact-latency assertions. Required repository gate results
and final backend test results belong to
[the owning plan](../.agent/plans/plan_01M22FQT8J3TFN9T5K1XVZQKRB.md), with
`jig-work-check.log`, `jig-final-test.log`, `jig-evidence.json` and `jig-gates.json`
in that log directory.

macOS and hosted CI execution of this change remain unverified. These checks do
not establish streaming-body, disconnect or panic recovery behavior, or delivery
by an arbitrary subscriber/exporter. The standard network smoke still does not
force requests into the example's Starting/Draining readiness window.

Latest evidence: 2026-09-09. Earlier sections retain their historical scope.

## Explicit HTTP observation severity: 2026-09-09

Bead `batter-faj.2` adds `HttpObservationLevel(tracing::Level)` as application-owned
response metadata. Standalone observation and the combined compatibility wrapper
read it through their shared helper. Unannotated responses retain WARN for 5xx
and INFO otherwise; futures dropped without returning a response retain WARN.
The level does not change status, outcome, fields, correlation, response headers
or body. Request extensions and client level headers do not select severity.
The runnable example explicitly marks Starting/Draining readiness responses INFO;
other application failures and stopped-process probes retain their defaults.

Five new test entries exercise all five tracing levels across standalone/split/
combined observation, default status classes, ignored request-side hints, explicit
readiness policy versus unrelated unguarded 503s, failure-renderer overrides,
and inner middleware replacing/removing an override. Existing rendering tests
now also assert an ERROR override on 429 after admission rejection, timeout and
forced cancellation, retaining the original response/correlation assertions.
Abort tests assert WARN despite an annotated response constructed but never
returned, with nested resource destruction under the first-poll subscriber.
Unpolled and body-lifetime tests remain intact. Focused verification passed
33 adapter tests and two doctests.

A temporary mutation that ignored the response extension compiled and failed
the event-level assertion with exit 101. The source was restored byte-for-byte.
An initial test compilation rejected incorrect Axum response tuple ordering;
that was corrected. The initial full 1.98.1 run passed its tests but failed
Clippy's complexity limit in the expanded tracing branches. Selecting separate
noncapturing emitters resolved the lint without an exemption. Both full scripts
were then executed successfully on the final Rust source.

Executed on Linux x86_64 with unchanged Cargo.lock SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`:

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
```

Rust 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0 (`4a4ef493e`, 2026-03-02)
each passed 380 test/doctest executions: 169 foundation, 207 workspace and four
doctests, with zero failures or ignored tests. Formatting, compilation, Clippy
and rustdoc also passed on both. The HTTP example was rebuilt with 1.98.1;
all three smoke modes passed their existing response, correlation, redaction,
one-completion-per-request and signal assertions. Smoke does not force requests
into the example's startup/drain window; readiness severity is exercised in the
router tests, not established by these network smoke cases.

Logs are under ignored `.agent/tmp/batter-faj.2/`: `verify-1.98.1.log`,
`verify-1.94.0.log`, `verify-1.98.1-initial.log`, `mutation-ignore-level.log`,
`http-build.log`, `http-default.log`, `http-sigint.log`, and `http-deadline.log`.
Final repository gate receipts belong to
[the owning plan](../.agent/plans/plan_01M22AY02A7BBJJHCNFX3RDQNN.md):
`scripts/jig work check --plan-id plan_01M22AY02A7BBJJHCNFX3RDQNN` and the final
backend `scripts/jig check test`, with `jig-work-check.log` and
`jig-final-test.log` in that log directory. Jig/Beads closure records retain
their outcomes without changing the source snapshot after verification.

macOS and hosted CI execution of this change remain unverified. No new body
streaming, real disconnect, handler panic recovery or log-delivery guarantee is
established. DEBUG/TRACE observations remain subject to subscriber filtering.

## Independent HTTP observation: 2026-09-08

Bead `batter-faj.1` adds stateless `observe_http` and policy-driven
`request_admission`, retaining `request_scope` as the combined compatibility
entry point. Both paths share the private observation/admission implementations;
`RequestPolicy` still couples lifecycle readiness and deadlines. The runnable
HTTP example assembles guarded routes, probes and fallback before observation,
with server identity outermost. Its identity future now also protects full
instrumented-future destruction with the public dispatch helper.

The new 12-test observation executable checks full-router event counts and
sanitized fields through startup, readiness and drain; application errors and
short-circuit/status-changing middleware; probe/fallback coverage; an added
application route; custom rendering after timeout/forced cancellation; retained
original correlation; context cancellation; standalone observation without an
execution policy; and subscriber retention during abort and nested destruction.
Directly discarded unpolled entry points emit no completion or application work.
Dropping a response body after construction produces no second HTTP event. This
last check uses an ordinary unconsumed body and does not establish streaming or
real disconnect behavior.

Placement tests make two Axum limits executable: routes appended after
`Router::layer` bypass observation, and a wrapper outside routing records
`<unmatched>` before matched metadata exists. Nesting outer observation around
legacy `request_scope` deliberately produces two HTTP observations; the supported
split composition emits one. There is no automatic deduplication. All 16 existing
adapter tests remain, with their assertions intact. The existing scoped-dispatch
capture helper is shared with the new executable; no global subscriber is installed.

Two targeted mutations failed event-count assertions with exit 101: removing
standalone observation, and having `request_admission` call the combined wrapper.
The middleware source was restored byte-for-byte after each mutation. Neither
failure was a compile error or a missing prerequisite.

Executed on Linux x86_64, kernel `7.0.11-76070011-generic`, Python 3.12.3:

```sh
cargo test -p batter-axum --locked
cargo clippy -p batter-axum --all-targets --locked -- -D warnings -D clippy::mod_module_files
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
```

All commands exited 0. Each full Rust 1.98.1 / 1.94.0 matrix passed 169 foundation
entries in isolation, 202 workspace entries and three doctests: 374 successful
entries including repeated foundation execution, zero failed and zero ignored.
The workspace counts 28 Axum tests, 169 foundation tests and five generic support
tests; the added doctest compiles the split router composition. Formatting,
Clippy, compilation and rustdoc also passed on both toolchains.

The normal executable was rebuilt with Rust 1.98.1. All three HTTP smoke modes
passed, now requiring exactly one HTTP completion with matching server-generated
request ID, route and status for each tested request, including probes and
unmatched fallback. Polling readiness may generate additional requests with their
own IDs; the assertions do not confuse them with the explicit test requests.
Untrusted IDs and raw unmatched path/query sentinels must be absent from output.
The application envelope, deadline and both native signal checks remain intact.

Logs are in ignored `.agent/tmp/batter-faj.1/`: `verify-1.98.1.log`,
`verify-1.94.0.log`, `http-build.log`, `http-default.log`, `http-sigint.log`,
`http-deadline.log`, `mutation-missing-observation.log` and
`mutation-duplicate-observation.log`. The owning
[execution plan](../.agent/plans/plan_01M21E97EVPQTVPJ6DDC79SR40.md) records Jig
receipts and final backend verification. `scripts/jig work check --plan-id
plan_01M21E97EVPQTVPJ6DDC79SR40` passed Clippy, formatting, tests, contract and
file-budget checks. Work evidence/gates reported fresh and passed. The final
`scripts/jig check test` also exited 0. Logs are `jig-work-check.log` and
`jig-final-test.log` in the same directory.

Cargo.lock is unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
There are no dependency or minimum-version changes. macOS and hosted CI execution
of this change remain unverified. No downstream repository was modified, and no
commit, push or publication was performed.

## Scenario-derived synchronization and overflow controls: 2026-09-08

Bead `batter-fvz` addresses the three-reviewer follow-up. The timing finding was
a private harness policy duplication: a five-second panic synchronization guard
ignored the deliberate two-second post-startup observation. It now derives the
maximum allowance from the scenario's wait policy. An exact-instant regression
includes panic after the latest accepted startup; no near-deadline launch sleep
or configurable timing protocol was added.

Event waits now observe process status before sampling capture and join the
reader on exit, so unread final output is not rejected as missing evidence.
Exited-child controls cover present and absent events. Overflow retains the first
detected byte/event cause and reports the captured event count. A live event-bound
control floods output after exhausting metadata, requires ordinary child exit
to prove continued draining, and still rejects evidence for event overflow after
the byte cap. The live deadline-wiring control also rejects a first kill request
delayed by a full observation allowance past the selected deadline.

The focused Linux suite passed all 48 entries. Three targeted mutations each
failed with exit 101 and were restored byte-for-byte: returning startup alone
from the maximum wait calculation; stopping capture on overflow; and delaying
the actual wait threshold by observation plus one second while recording the
original deadline. The last fails with `watchdog kill was late`. Clippy initially
reported the expanded fixture function at 101/100 lines; extracting the existing
invalid-output scenarios fixed this without a lint exemption.

Research answers are in [references](references.md#synchronization-review-research-2026-09-08).
The macOS hosted job is deliberately focused. The exported scenario test protects
the removed ambient launch path. The Python probe has one intentional startup
phase budget; its hard alarm can bypass `finally`, leaving containment to the
independent deadlines and adopting reaper. The ten-second emergency control
continues to test the actual fallback in ordinary matrices. No finite margin
guarantees progress on a suspended/starved OS. These are explicit limits, not
reasons to introduce another deadline configuration or signal-handler protocol.

Both Rust 1.98.1 and 1.94.0 verification matrices exited 0 on Linux x86_64
(kernel `7.0.11-76070011-generic`, Python 3.12.3) and the authorized macOS arm64
host (macOS 26.6.2 build `25G83`, Python 3.14.7). Each host ran:

```sh
RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
```

HTTP was rebuilt using 1.98.1; all three smoke modes passed on both hosts.
Linux passes 154 foundation / 175 workspace entries plus two doctests; macOS
passes 152 foundation / 173 workspace entries plus two doctests. The focused
suite contains 25 process controls, 19 pure evidence/policy tests, three reader
tests and one inert dispatch entry; two process controls are Linux-only.
Formatting, Clippy, compilation and rustdoc passed through the full matrices.
Static package inspection and diff checks also passed.

macOS executed `/tmp/batter-review.ttFRHr`, reusing only the previous target
cache through a symlink; Cargo rebuilt the changed test source. All 67 source/build
hashes match on both hosts. Snapshot archive SHA-256:
`9cce84cdd3006e20ec2074c099365acc83ed90a7b9c6dabfc923ed5e012e841d`.
Cargo.lock remains unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Documentation and tracker records were finalized after source validation.

Current follow-up logs in ignored `.agent/tmp/batter-fvz-review/` are
`verify-linux-1.98.1.log`, `verify-linux-1.94.0.log`, `mutation-startup-only.log`,
`mutation-stop-draining.log`, `mutation-late-kill.log`, `http-build.log`,
`http-default.log`, `http-sigint.log`, `http-deadline.log`, `macos-manifest.log`,
and the copied `macos/` logs. Older differently named logs in that directory
belong to earlier work. Jig receipts and final backend-test evidence belong to
[this execution plan](../.agent/plans/plan_01M20N7KXZDVWH9WM6AA1V4303.md).
The updated hosted macOS job remains unexecuted: the successful remote runs
still refer to baseline `e5f2f04`, and no commit or push was performed. Library
APIs, dependencies and lifecycle behavior are unchanged.

## Startup evidence after a child panic: 2026-09-08

Bead `batter-fvz` fixes a reproduced polling-order failure and the stale
Linux-only evidence summary in `docs/status.md`. A complete drain record
captured on time now establishes the observation deadline even when the next
parent poll sees later panic diagnostics. Capture overflow remains an error
before event lookup. Missing startup still fails promptly on a known panic;
a genuinely late event remains a deadline failure. Final validation still
rejects all captured child panics. This separates timing evidence from final
outcome validation without changing the library or dependencies.

The fresh 41-entry baseline passed. Regressions added before the fix failed
four tests (exit 101): the on-time event plus later panic, a late event plus
panic, the pure late-poll deadline calculation, and the live deliberate-panic
fixture. The live control waited for captured panic output before its first
startup resolution and reproduced the original `child panicked` unwrap failure.
It now reaches final validation and verifies that the panic is rejected there,
with the unchanged drain-plus-observation deadline and observed watchdog kill.
No arbitrary startup delay or near-deadline sleep was added.

Two additional mutations were rejected with exit 101 and restored byte-for-byte:
removing final panic rejection fails
`evidence::tests::later_panic_keeps_captured_startup_but_invalidates_final_evidence`;
removing the startup capture bound fails
`evidence::tests::byte_overflow_invalidates_even_previously_observed_startup`.
The latter now includes both an on-time drain and panic diagnostics before
overflow. Missing-startup panic remains covered by an immediate-failure test.

Both supported toolchains passed on Linux x86_64 (kernel
`7.0.11-76070011-generic`, Python 3.12.3) and the authorized macOS arm64 host
(macOS 26.6.2 build `25G83`, Darwin 25.6.0, Python 3.14.7): Rust 1.98.1
(`48a229cea`, Cargo `797e8a9bc`) and Rust 1.94.0 (`4a4ef493e`, Cargo `85eff7c80`).
These commands each exited 0 on both hosts, with HTTP rebuilt using pinned
Rust 1.98.1:

```sh
RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
```

Linux passes 150 foundation / 171 workspace entries and two doctests, including
44 focused entries. macOS passes 148 foundation / 169 workspace entries and two
doctests, including 42 focused entries. The focused executable has 22 process
controls, 18 pure evidence/policy tests, three reader-thread tests and one inert
dispatch entry; two process probes are Linux-only. Formatting, compilation,
Clippy and rustdoc also pass through the verification scripts.

macOS ran an isolated source snapshot at `/tmp/batter-panic.jg9qJ2`, reusing the
previous temporary Cargo target directory via a `target` symlink. Cargo rebuilt
the changed targets from this source before execution. Archive SHA-256:
`7e897c7214b16915da9fa6d922cbea1588afe231fc80f79af572bbade7404ad2`.
All 67 source/build hashes match on both hosts. Documentation and tracker
records were finalized afterward. Cargo.lock is unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

Ignored `.agent/tmp/batter-fvz-panic/` retains the baseline, before-fix and focused
logs, both mutation logs/backup, host verification and HTTP logs, source manifest
and archive. Remote logs/environment are copied into `macos/`. Repository gate
and final backend-test evidence belongs to
[the execution plan](../.agent/plans/plan_01M20K6ESFXMBG1239ATKTGFF6.md).
The updated hosted macOS job remains unexecuted; SSH testing does not validate
GitHub Actions execution. Abrupt-owner-death adoption/reaping probes remain
Linux-only; macOS covers the shared parent-pipe EOF path. Existing OS-scheduling,
process-control and application-finalization limitations remain explicit.

## Deadline wiring and reader failure controls: 2026-09-08

The final three-reviewer pass identified a remaining test gap in Bead
`batter-fvz`: pure deadline calculations were tested, but a fixed deadline
could replace the computed argument to `wait()` without failing the fast-startup
process controls. The implementation already used the correct deadline.

A new real-process control now runs both blocked-runtime scenarios and asserts
that the deadline actually passed to `wait()` equals the captured drain instant
plus the three-second observation allowance. It also requires observed SIGKILL
and a first kill request at or after that deadline. This checks the driver and
policy connection without a near-deadline startup sleep.

Capture now accepts a private `Read + Send + 'static` input; process runs still
use their ordinary pipe reader. Three scripted-reader controls exercise the
same OS thread, read loop and join path: retry Interrupted and finish on EOF;
return the original BrokenPipe kind/message after partial output; and convert a
reader panic into the fixed harness I/O error. Each confirms retained evidence
and reader destruction before finish returns. Neither read errors nor panics
are accepted as successful complete capture. No library API, dependency or
application behavior changed.

The fresh baseline passed all 37 focused entries. The expanded suite passes 41
on Linux and 39 on macOS: 21 process controls, 16 pure evidence/policy tests,
three reader-thread tests and one inert dispatch entry, with two Linux-only
probe controls excluded on macOS. Three mutations each failed (exit 101), then
were restored byte-for-byte before the full matrices:

| Mutation | Rejecting test / evidence |
| --- | --- |
| Replace `wait(limit)` with `wait(EXIT_LIMIT)` | `watchdog::tests::blocked_runtime_wait_uses_the_captured_drain_deadline`; all child milestones were present, but the selected deadline differed from drain plus observation (5.01-second run). |
| Replace read-error propagation with EOF | `capture::tests::reader_io_error_is_preserved_after_partial_output_and_join`; finish incorrectly returned Ok. |
| Replace reader-panic propagation with success | `capture::tests::reader_panic_becomes_an_io_error_after_partial_output_and_join`; finish incorrectly returned Ok. |

Both Rust 1.98.1 (`48a229cea`, Cargo `797e8a9bc`) and Rust 1.94.0 (`4a4ef493e`,
Cargo `85eff7c80`) passed the following full matrices on Linux x86_64 (kernel
`7.0.11-76070011-generic`, Python 3.12.3) and macOS arm64 (26.6.2 build `25G83`,
Darwin 25.6.0, Python 3.14.7). Every command below exited 0; HTTP was freshly
built with pinned Rust 1.98.1 on each host.

```sh
RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
```

Linux passes 147 foundation / 168 workspace entries and two doctests; macOS
passes 145 foundation / 166 workspace entries and two doctests. The verification
scripts also pass formatting, compilation, Clippy and rustdoc. The updated
hosted macOS CI job remains unexecuted; these are actual SSH host results.

macOS used the isolated `/tmp/batter-wiring.0G3Ftp` directory. The transferred
source archive has SHA-256
`200cbd184f11e91112d58b42ef6192d679d9bdad19899660ab96595cc9b0d721`;
all 67 source/build file hashes match on both hosts. Documentation and tracker
evidence were finalized afterward. Cargo.lock remains unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Logs, restored-mutation backups and the source manifest/archive are retained in
ignored `.agent/tmp/batter-fvz-wiring/`, with remote environment and command logs
in `macos/`. Final Jig gate and backend-test evidence belongs to
[the execution plan](../.agent/plans/plan_01M20HPY9R5QVQ616B894HCAGK.md).
The prior platform, OS-scheduling and process-finalization limitations remain.

## Captured evidence and deterministic deadlines: 2026-09-08

Bead `batter-fvz` now separates diagnostic capture, event evidence and deadline
policy. The repeated timing findings exposed a private harness design weakness:
startup and final validation parsed different evidence, while polling time and
post-capture elapsed time stood in for event and kill-request times. They did
not establish a defect in Batter's lifecycle architecture. Research into Rust
sleep/process semantics and macOS runners preceded implementation; primary
sources are recorded in [references](references.md#subprocess-evidence-clocks-and-platform-research-2026-09-08).

A behavior-preserving extraction first passed the original 22-entry suite.
The subsequent fix gives complete protocol records one bounded owner with
capture timestamps. Recording and inspection sample time under the same mutex,
so a deadline decision cannot use stale evidence followed by a newer clock.
An on-time record remains valid when polling is late; a late record cannot
extend startup. Observation starts from captured evidence. Validation uses the
first actual kill-request time and observed SIGKILL, never capture-join delay.
Partial protocol records and byte/event overflow fail closed. This timestamp
measures parent capture, not an unobservable child write time.

The four-second sleep regression was replaced with deterministic boundary tests.
Sixteen pure tests cover exact/fragmented/duplicate records, incomplete final
records, panic/overflow, delayed polling, early/late/missing startup, retained
observation budgets, early kill requests and non-SIGKILL statuses. The real
subprocess controls still exercise blocked Tokio runtimes, missing startup,
emergency exit and kill/reap behavior. Linux has 37 focused entries (20 process
controls, 16 pure tests and one inert fixture dispatch); macOS has 35 because two
reaping/probe controls require Linux.

Three mutations each failed with exit 101, then were restored before full
verification: checking the poll clock before buffered evidence; resetting the
observation window from polling time; and approving a kill using elapsed time
after capture joining. The first two fail
`timing::tests::delayed_startup_keeps_its_full_observation_even_when_polled_late`;
the third fails
`watchdog::tests::delayed_capture_cannot_make_an_early_kill_look_timely`.

Final source passed on both hosts:

| Host | Environment | Verification entries per toolchain |
| --- | --- | --- |
| Linux | x86_64, kernel `7.0.11-76070011-generic`, Python 3.12.3 | 143 foundation, 164 workspace, two doctests; 37 focused |
| macOS via authorized SSH | arm64, macOS 26.6.2 build `25G83`, Darwin 25.6.0, Python 3.14.7 | 141 foundation, 162 workspace, two doctests; 35 focused |

Both hosts used Rust 1.98.1 (`48a229cea`, Cargo `797e8a9bc`) and Rust 1.94.0
(`4a4ef493e`, Cargo `85eff7c80`). These commands all passed with exit 0 on each
host, including formatting, compilation, Clippy, rustdoc and tests:

```sh
RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
```

HTTP was rebuilt with the pinned 1.98.1 toolchain on each host. macOS used an
isolated `/tmp/batter-evidence.06xIfN` directory, with no existing checkout
modified. The transferred source archive has SHA-256
`18012ec8a13218378321b7d9d4f2bdc2c4bc71cd3d5feb33e2e608bf448d47ee`;
a manifest confirmed identical hashes for all 66 source/build files on both
hosts. Documentation and tracker evidence were finalized afterward. Cargo.lock
is unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

Ignored logs are under `.agent/tmp/batter-fvz-evidence/`: `baseline.log`,
`extraction.log`, `focused.log`, the three mutation logs, `verify-linux-*.log`,
`http-linux-*.log`, `source-manifest.json` and `snapshot.tar`. The `macos/`
subdirectory retains environment, both verification and three HTTP smoke logs.
Final repository gates and completion evidence belong to
[the execution plan](../.agent/plans/plan_01M20F5AD3SH9MD9GM4YMNTKQZ.md).

The CI definition now compiles all workspace targets and runs focused macOS
tests on both supported toolchains. That updated hosted job has not executed;
the SSH results above are actual host evidence. Linux-only probes do not claim
macOS reaping coverage. Other Unix targets, OS suspension and process-control
failures remain outside the executed evidence. Library behavior, public APIs
and dependencies are unchanged. Process termination does not prove application
finalization.

## Separate startup and observation windows: 2026-09-08

Bead `batter-fvz` was reopened for the comprehensive-review timing finding.
The blocked current-thread scenarios now allow five seconds from spawn to
observe `drain-requested`, followed by the fixture's two-second observation
plus one second of margin. The combined eight-second allowance remains below
the child's independent ten-second emergency exit. Ordinary scenarios retain
their existing deadlines. Validation records and checks the selected deadline;
an extended observation cannot be approved using the old five-second floor.

Three new controls cover delayed launch, absent startup, and a SIGKILL between
the ordinary deadline and an extended deadline. The delayed-launch test first
failed against the original watchdog at 5.005940244 seconds with
`missing event blocked-observation-elapsed`. It holds the launch record for
four seconds, then requires all original blocked-runtime milestones and rejects
completion/cleanup evidence. With the fix, all 22 subprocess entries pass in
10.01 seconds. A separate mutation restored only the old validation comparison
`self.elapsed >= LIMIT`: the extended-deadline control failed (exit 101) in
5.00 seconds because that comparison accepted the premature kill. The watchdog
was restored byte-for-byte before full verification.

Logs and the mutation backup are retained under ignored
`.agent/tmp/batter-fvz-startup/`: `before-fix.log`, `focused.log`,
`old-deadline-mutation.log`, `verify-1.98.1.log`, `verify-1.94.0.log`, and
`http-*.log`. The focused command was
`cargo test -p batter --test non_yielding --locked`; the before-fix run selected
`-- --exact watchdog::tests::delayed_startup_preserves_the_blocked_runtime_observation_window`,
and the mutation selected
`-- --exact watchdog::tests::watchdog_rejects_a_kill_before_an_extended_observation_deadline`.

Executed on Linux `7.0.11-76070011-generic`, x86_64, Python 3.12.3:
Rust 1.98.1 (`48a229cea`, Cargo `797e8a9bc`) and Rust 1.94.0 (`4a4ef493e`,
Cargo `85eff7c80`). Cargo.lock is unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
These commands passed (exit 0):

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/check_package.py
git diff --check
```

Both full matrices pass 128 foundation entries, 149 workspace entries and two
doctests, including all 22 subprocess entries, plus formatting, compilation,
Clippy and rustdoc. The three rebuilt HTTP smoke modes pass. Jig gate receipts
and final completion status belong to
[the execution plan](../.agent/plans/plan_01M20DGZY1HF5V5TRAX9KQ0WT1.md).

This is a private test-harness fix; library behavior, APIs and dependencies are
unchanged. macOS execution remains unverified. Scheduling suspension and OS
process-control failures remain outside these test timing bounds.

## Prompt unwind cleanup regression: 2026-09-08

Bead `batter-fvz` was reopened for the unwind test's missing timing assertion.
The control now requires owner-thread unwinding and capture-reader joining to
finish within five seconds of child startup. The startup-relative measurement
prevents a late unwind from passing by waiting for the ten-second emergency
exit. The Linux `/proc/<pid>` absence check still establishes reaping.

An executed mutation replaced only `FixtureChild::Drop`'s `kill_and_reap()`
call with `self.child.wait()`. The command was:

```sh
cargo test -p batter --test non_yielding --locked -- --exact watchdog::tests::parent_unwind_kills_reaps_and_joins_capture_reader
```

Before the assertion, that mutation passed (exit 0) in 10.00 seconds. With the
assertion, the same mutation failed (exit 101), reporting
`unwind cleanup took 10.002227365s; expected completion before 5s`.
The original watchdog source was restored byte-for-byte after each mutation;
no destructor behavior change remains. Logs are retained under ignored
`.agent/tmp/batter-fvz-unwind/` as `original-wait-mutation.log` and
`fixed-wait-mutation.log`.

Execution uses the same Linux x86_64 platform, Python 3.12.3 and Rust/Cargo
versions documented in the optimization follow-up below. Cargo.lock is
unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
After restoring the destructor, these commands passed (exit 0):

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
scripts/jig check test
python3 scripts/check_package.py
git diff --check
```

Each matrix passes 125 foundation entries, 146 workspace entries and two
doctests, including all 19 subprocess entries, plus formatting, compilation,
Clippy and rustdoc. The three HTTP smoke modes and final Jig backend check pass.
The matrix logs are `verify-1.98.1.log` and `verify-1.94.0.log`; HTTP and Jig logs
are `http-*.log` and `jig-test.log` in the mutation log directory above.

macOS execution remains unverified. This change adds a private test assertion
and supporting documentation; library behavior and dependencies are unchanged.

## Optimized parent-death probe checks: 2026-09-08

Bead `batter-fvz` was reopened for the confirmed Python optimization finding.
The probe now uses explicit failure branches for deadlines, EOF, output bounds,
required evidence, owner/child status and forbidden events. The child command
accepts an argument vector so a Rust negative control can supply independent
protocol fixtures. Both the real orphan-reaping test and this control set
`PYTHONOPTIMIZE=1`. The control checks that a child exiting 7 and a child emitting
cleanup evidence before exiting 74 each cause probe exit 1, a specific failure
diagnostic, and no success marker. Their own alarm bounds the control children.

Before replacing the assertions, this command failed as intended (exit 101):

```sh
cargo test -p batter --test non_yielding --locked -- --exact watchdog::tests::optimized_parent_death_probe_rejects_invalid_child_evidence
```

The original optimized probe accepted the wrong exit code, returned 0, and
printed `batter-fixture:parent-death-reaped`. After the explicit-check fix, both
negative cases and the real parent-death case pass. This is an executed
regression check against the original failure, not just a source assertion.

Executed locally on Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu,
Python 3.12.3, Rust 1.98.1 (48a229cea 2026-09-01) / Cargo 1.98.1
(797e8a9bc 2026-08-05), and Rust 1.94.0 (4a4ef493e 2026-03-02) /
Cargo 1.94.0 (85eff7c80 2026-01-15). Cargo.lock remains unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

These commands passed (exit 0):

```sh
cargo test -p batter --test non_yielding --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/check_package.py
git diff --check
```

The focused harness passes 19 entries in 10.01 seconds. Both full matrices pass
125 isolated foundation entries, 146 workspace entries and two doctests with no
ignored tests, plus formatting, compilation, Clippy and rustdoc with warnings
denied. All three rebuilt HTTP smoke modes pass. Static package inspection
counts 146 authored entries and reports no failures. Full-matrix and HTTP logs
are retained under ignored `.agent/tmp/batter-fvz-python-optimize/`.
The final backend check, `scripts/jig check test`, also passed (exit 0); its log
is `jig-test.log` in that directory, with a receipt in `.agent/state/`.

This follow-up changes private test infrastructure and its documentation only.
No public API, dependency, or library behavior changed. macOS execution remains
unverified; no hosted CI, PostgreSQL, or production execution is claimed.
The earlier validation sections remain historical evidence for their snapshots.

## Non-yielding harness review fixes: 2026-09-08

Bead `batter-fvz` was reopened after comprehensive review. Follow-up execution
record: [.agent/plans/plan_01M20896B9AP88W2T4XZD2AGW6.md](../.agent/plans/plan_01M20896B9AP88W2T4XZD2AGW6.md).
The Git baseline remains `e5f2f04b2dbb349d08085caf177f662fcbc89812`.
The first implementation's evidence below is historical; its eight-entry
harness has been superseded by the 18-entry harness described here.

Research into GitHub cancellation, runner v2.337.0, Rust process/pipe semantics,
and Linux subreapers is recorded in [references](references.md#subprocess-review-follow-up-2026-09-08).
It did not establish unconditional runner process-group termination. The fix
adds a child-owned emergency deadline and a parent-owned stdin pipe that both
authorizes a PID-specific launch and ends the child on parent death. Capture
files are replaced by continuously drained, bounded memory. Kill requests and
actual wait status are separate facts. The platform follow-up also removes
non-Unix example signal fallbacks and records the Unix-only policy in guides,
package rustdoc and ADR-007. Library behavior, public APIs, manifests, Cargo.lock
and the resolved Tokio 1.53.1 dependency remain unchanged.

Executed locally on Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu,
with Python 3.12.3 and both supported toolchains: rustc 1.98.1
(48a229cea 2026-09-01), Cargo 1.98.1 (797e8a9bc 2026-08-05), rustc 1.94.0
(4a4ef493e 2026-03-02), Cargo 1.94.0 (85eff7c80 2026-01-15).
Cargo.lock SHA-256:
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

These commands passed after the final Unix-only platform cutover (exit 0):

```sh
cargo test -p batter --test non_yielding --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/check_package.py
git diff --check
```

The focused suite passed all 18 entries in 10.01 seconds. Each full matrix
passed 124 isolated foundation entries, 145 workspace entries and two doctests,
with no ignored/filtered entries, plus formatting, compilation, Clippy and
rustdoc with warnings denied. All three HTTP smoke modes passed and exited 0.
Static package inspection found 145 authored entries and no link/package
failures; this is distinct from executed Rust evidence. The changed CI YAML
also parsed successfully locally.

The final platform cutover removed the Windows watchdog branch and CI target,
and all three non-Unix example signal fallbacks. Linux and macOS remain in
scope and use the same native Unix implementation. SIGINT/SIGTERM registration
still precedes readiness acknowledgement. A source/workflow audit found no
remaining Windows implementation branches, non-Unix signal fallbacks or Windows
CI targets. Final matrix logs are `verify-unix-1.98.1.log` and
`verify-unix-1.94.0.log`; rebuilt HTTP smoke logs are `http-unix-*.log` in the
directory below. The lockfile hash is unchanged.

The Linux regression killed only the fixture owner's PID after observing
unjoined work, skipped cleanup and runtime-drop entry. Its isolated subreaper
adopted and waited for the child's parent-disconnect exit (74), with no task
destruction or finalizer invocation. Unix controls verified the ten-second
emergency exit (75) with blocked work and incomplete launch, mismatched launch
PID rejection, harmless inherited scenario state, preserved natural statuses
after a later kill, early-kill rejection, forbidden events, output overflow,
spawn failure and parent-unwind cleanup. Post-run inspection found no fixture
processes or legacy capture directories.

Five temporary harness mutations each produced an executed test failure (101):
removing parent-EOF termination, accepting the wrong launch PID, accepting any
unsuccessful Unix status as a kill, ignoring forbidden events, and ignoring
capture overflow. Each used
`cargo test -p batter --test non_yielding --locked -- --exact watchdog::tests::<test_name> --nocapture`
with the corresponding regression in `watchdog_tests.rs`. All mutations were
restored byte for byte before both full matrices. The original report/cleanup
mutation evidence below remains applicable; its semantic assertions remain.
An initial compile check rejected moving a capture JoinHandle through
`catch_unwind`; the unwind control now joins a deliberately panicking owner
thread, avoiding an `AssertUnwindSafe` assertion or reuse of unwound state.

Jig's five configured targets passed, and its required `verify` evidence gate
was fresh and passed. Completion commands are:

```sh
scripts/jig work check --plan-id plan_01M20896B9AP88W2T4XZD2AGW6
scripts/jig work evidence --plan-id plan_01M20896B9AP88W2T4XZD2AGW6 --json
scripts/jig work gates --plan-id plan_01M20896B9AP88W2T4XZD2AGW6 --json
scripts/jig check test
```

Logs are retained under ignored `.agent/tmp/batter-fvz-review/`; completion
receipts, including the final backend test, belong to the linked plan. The macOS
focused CI job remains configured but has not been executed here; neither macOS
compilation nor hosted success is claimed. Windows is unsupported and not
planned; its CI job and exit-code branch have been removed. Process
termination proves neither application finalization nor Tokio preemption;
after real parent death, its adopting OS reaper owns waiting. No PostgreSQL,
external-service, hosted-CI or production evidence is implied. Nothing was
committed, published or deployed.

## Non-yielding subprocess tests: 2026-09-08

Bead `batter-fvz`, Git baseline `e5f2f04b2dbb349d08085caf177f662fcbc89812`.
Added a private foundation integration-test fixture and OS-process watchdog;
production source, manifests and dependencies are unchanged. Execution is local
Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu, with rustc 1.98.1
(48a229cea 2026-09-01), Cargo 1.98.1 (797e8a9bc 2026-08-05), rustc 1.94.0
(4a4ef493e 2026-03-02), and Cargo 1.94.0 (85eff7c80 2026-01-15).
Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`;
the resolved Tokio version is 1.53.1.

Executed successfully on the final test source (exit 0):

```sh
cargo test -p batter --test non_yielding --locked -- --nocapture
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/check_package.py
git diff --check
scripts/jig work check --plan-id plan_01M205HP71C12K7EQ44H0JFW7G
scripts/jig work evidence --plan-id plan_01M205HP71C12K7EQ44H0JFW7G --json
scripts/jig work gates --plan-id plan_01M205HP71C12K7EQ44H0JFW7G --json
scripts/jig check test
```

The focused executable passed all eight entries in 5.01 seconds: seven parent
tests plus an inert child-dispatch entry. Each full Rust matrix passed isolated
core compilation and 114 core entries, 135 workspace entries, two doctests,
all example/binary compilation targets, Clippy and rustdoc with warnings denied.
No entries were ignored or filtered in either full matrix. The three HTTP smoke
modes passed probes, work/deadline and envelope/telemetry assertions, then exited
zero after SIGTERM or SIGINT. Static inspection found 135 test entries and no
package/link failures; it is separate from the executed Rust evidence.
Jig's five configured targets passed (Clippy, formatting, tests, repository
contract and file budget), and its required `verify` evidence gate was fresh
and passed. The final backend `scripts/jig check test` passed the same locked
core/workspace/doctest matrix. No gate or semantic assertion was relaxed.

The cooperative child observed forced cancellation, joined its direct task,
ran the dependency finalizer and destroyed its runtime normally. The two-worker
child retained its blocked task as abort-requested and unjoined, produced no
fabricated termination record, skipped the finalizer factory, and remained
live during runtime destruction until killed. The current-thread child polled
its timer before task entry and externally acknowledged two seconds after drain,
beyond its timer and configured total allowance, without timer/report/cleanup
completion. Watchdog timeouts killed and reaped their children after the
five-second limit. Failure controls rejected missing milestones and child
panics, including a panic after expected stall milestones. Parent-unwind cleanup
removed capture files and left no `/proc/<pid>` entry (including no zombie).
A post-run process/capture scan found no fixture processes or temporary directories.

Two temporary production mutations challenged the report assertions using
`cargo test -p batter --test non_yielding --locked -- --exact non_yielding_child_reports_unjoined_work_and_cannot_drop_runtime --nocapture`.
Clearing the returned `unjoined` list failed the direct-join assertion; replacing
unsafe-exit skipping with cleanup invocation failed the live-task check and
cleanup-record assertion. Both executions failed as expected (exit 101), and
the parent killed/reaped the child after five seconds. Production source was
restored byte for byte; no weakened assertion or mutation remains.

The first fixture attempt stalled its two-worker driver after a drain request
from the blocking task, consistent with Tokio's documented non-stealable LIFO
wake slot. The final fixture requests drain from a separate OS thread after
task-entry acknowledgement. An initial compile error used the wrong cleanup
variant and was corrected to the existing `Succeeded` outcome. The default
panic hook prints the deliberately caught parent panic with `--nocapture`;
the test passes without replacing that hook or a global tracing subscriber.

Logs are retained locally under ignored `.agent/tmp/batter-fvz/`. This evidence
does not establish task preemption, application cleanup after OS kill, hidden
descendant termination, hosted CI, non-Linux behavior, live PostgreSQL or
production operation. No external service prerequisite was skipped. Nothing
was committed, published or deployed.

## Beads backlog migration: 2026-09-08

Audited delivery requirements against Git baseline `9654b2e`, including the
roadmap, integration/testing/operations docs, Effect reconciliation, capability
status, security policy and completed execution plans. Four planning review
rounds, a separate fresh-context fixture check and a final source-removal audit
resolved dependency sequencing and cleanup-ownership ambiguities. No runtime
code, dependency graph, upstream integration or publication behavior changed.

The tracker now contains 34 records: 29 tasks and five epics, comprising 28 open,
three closed and three deferred outcomes. The existing extraction issue was
reused; completed local-validation/finite-ownership milestones were migrated as
closed rather than rerun or reopened. All 12 former roadmap identifiers survive
in Beads provenance. Markdown task inventories were removed; contracts,
implemented capability facts and historical execution evidence remain.

Migration checks executed locally:

- Compared all 34 stored descriptions, acceptance fields, priorities, statuses
  and types through `br show` / `br list` with the reviewed issue descriptions.
- Compared each `br dep list <id> --json` result with the intended graph:
  66 edges, including parent grouping and blocking prerequisites.
- `br dep cycles --json`: no cycles.
- `br lint --status all --json`: no missing issue-template sections.
- `br ready --type task --json`: ten actionable delivery tasks; deferred items
  and tasks with unfinished prerequisites were excluded.
- `br --no-db list --all --deferred --json`: the JSONL-only view matched the
  database's descriptions, statuses and priorities. `br sync --flush-only`
  refreshed the tracked export, and `bv --robot-triage` saw all 34 records.
- `python3 scripts/check_package.py`: no TOML, internal-link or static source
  inspection failures. `git diff --check`: no whitespace errors.

An attempted `bv --beads-file` override was rejected by the installed CLI; no
such command was retained in documentation. Viewer source/count checks use its
reported `source_path` and the authoritative `br` result. Epic template lint
initially requested `Success Criteria` headings; those headings were corrected
and the clean result above is the final check. These are migration checks, not
new Rust, hosted-CI or live-database execution evidence. Current work is available
through [Beads navigation](roadmap.md).

## Original authoring evidence (historical)

The following table describes the original authoring environment only. Subsequent
local execution and dependency-upgrade evidence is recorded below.

The authoring environment did not contain `rustc`, `cargo`, or `rustfmt`.
The container could not resolve the Rust toolchain download host, and an attempt
to obtain the distribution through the download facility did not succeed.
No external compiler/CI execution was substituted or claimed.

| Check | Evidence status |
| --- | --- |
| Source files and 67 test cases authored | Present; inventory counted locally. |
| TOML manifest/toolchain/config parsing | Checked locally with Python tomllib. |
| Internal Markdown file links | Checked by scripts/check_package.py. |
| Rust delimiter lexical balance | Checked by scripts/check_package.py; NOT a Rust parser/type checker. |
| Shell/Python script syntax and CI YAML structure | Checked locally; not evidence of hosted CI execution. |
| ZIP extraction/CRC/source checksums | Checked during packaging. |
| Cargo dependency resolution / Cargo.lock | NOT RUN; no lockfile invented. |
| rustfmt / compiler / borrow checker | NOT RUN. Initial formatting must be normalized by bootstrap. |
| Unit/integration/doctests / Clippy / rustdoc | NOT RUN. No passing-test claim. |
| HTTP example / SIGTERM smoke / traffic tests | NOT RUN. |
| SQLx example against PostgreSQL | NOT RUN. |
| Runledger/Runlimit/harness compatibility | NOT IMPLEMENTED / NOT VERIFIED. |
| Intended MSRV 1.88.0 / current stable matrix | NOT VERIFIED. |
| Security audit / load / race / platform coverage | NOT PERFORMED. |

The machine-readable [package checks](package-checks.json) record the local static
inspection. These checks catch packaging mistakes, not Rust type/lifetime/runtime
errors. Source integrity hashes attest archive contents, not software correctness.

## Original first-run procedure

```sh
rustup show
rustc --version --verbose
cargo --version
bash scripts/verify.sh --bootstrap
# Review and commit the real Cargo.lock and formatter changes.
bash scripts/verify.sh
cargo build --example http_service --features axum --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
```

Repeat the verification script with a current stable toolchain. The included CI
workflow describes the matrix but has not run here. Resolve any transitive MSRV
conflicts deliberately; do not casually raise the declared minimum or claim
reproducible builds while leaving the dependency graph unlocked.

## Recording subsequent evidence

Append the toolchain/target, platform, full command, resolved Cargo.lock SHA-256,
exit status, test counts, skipped prerequisites, and failure details. Change
source-status claims only after the corresponding command succeeds. Database
and upstream-integration claims need their own actual executions; successful
SQLx example compilation alone does not establish transaction behavior.

The initial validation milestone is retained as closed Bead `batter-okz`.
Its original Rust 1.88 baseline precedes the SQLx-driven minimum-version increase;
see [backlog access](roadmap.md) for tracker commands.

## Dependency and toolchain refresh: 2026-09-07

Platform: Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu.
All runs below were local, not hosted CI. A Cargo-generated lockfile already
existed at the start of this session. Bootstrap formatted the authored sources;
`cargo +1.94.0 update` subsequently refreshed the dependency graph.

| Graph / toolchain | rustc | Cargo | Result |
| --- | --- | --- | --- |
| Original SQLx 0.8.6 graph / 1.88.0 | 1.88.0 (6b00bc388 2025-06-23) | 1.88.0 (873a06493 2025-05-10) | Full verification passed after the two repairs below. |
| Upgraded graph / minimum 1.94.0 | 1.94.0 (4a4ef493e 2026-03-02) | 1.94.0 (85eff7c80 2026-01-15) | Full verification passed. |
| Upgraded graph / pinned 1.98.1 | 1.98.1 (48a229cea 2026-09-01) | 1.98.1 (797e8a9bc 2026-08-05) | Full verification and HTTP SIGTERM smoke passed. |

The Rust release server identified stable as 1.98.1 on 2026-09-07. `rustup update
stable --no-self-update` installed that release, and `rustc +stable --version`
and `cargo +stable --version` match the pinned 1.98.1 versions above. The matrix
was run through the pinned toolchain; an identical stable-alias run was not repeated.
Default rustfmt: 1.9.0-stable (48a229ceae 2026-09-01).
Default Clippy: 0.1.98 (48a229ceae 2026-09-01).

Commands and outcomes (exit 0 unless explicitly described otherwise):

```sh
RUSTUP_TOOLCHAIN=1.88.0 bash scripts/verify.sh --bootstrap
cargo +1.94.0 update
cargo +1.94.0 update --verbose
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
bash scripts/verify.sh  # rust-toolchain.toml selects 1.98.1
cargo build --example http_service --features axum --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
```

The first 1.88.0 bootstrap run exited 101 at Clippy: `tests/cleanup.rs` wrapped
`pending::<Result<(), BoxError>>` in a redundant closure. The second exited 101
at rustdoc: `src/http.rs` used an unquoted `Extension<OperationContext>` that
rustdoc interpreted as an unclosed HTML tag. Removing the closure and adding
code formatting fixed these diagnostics. The third bootstrap run completed
successfully. No runtime logic or failure assertions were weakened.

Each successful verification script ran exactly these checks:

```sh
cargo fmt --all -- --check
cargo check --lib --no-default-features --locked
cargo test --workspace --no-default-features --lib --tests --locked
cargo test --workspace --all-features --all-targets --locked
cargo test --workspace --all-features --doc --locked
cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
RUSTDOCFLAGS=" -D warnings" cargo doc --workspace --all-features --no-deps --locked
```

Per toolchain: 59 tests passed without default features; 67 tests passed with
all features; one doctest passed. Zero failures, ignored tests, or filtered tests.
The eight HTTP tests are feature-gated and therefore absent from the core run.
All three examples compiled. Clippy and rustdoc completed with warnings denied.
The HTTP process smoke received successful responses from `/live`, `/ready`, and
`/work`, then sent SIGTERM and observed exit status 0.

Lockfile SHA-256:

- Original graph: `5c36cce6076c625bf9981a44220d4063d1adb8ee8e8c4995b08aca59216ac14a`.
- Upgraded graph: `79865e6518881639a9b8776dbd73d58d924ff80f385faf0d5fe483b093e87c26`.

All nine direct dependency versions in the lockfile match the crates.io latest
stable releases queried on this date; [references](references.md#dependency-refresh-2026-09-07)
records the old/new requirements and upstream constraints. A second verbose
Cargo update made zero changes. The only older compatible-line patches are
`matchit` and `generic-array`, held by exact upstream requirements.

Not executed: live PostgreSQL, Runledger/Runlimit/harness integrations, SIGINT,
streaming/disconnect/load/race suites, other platforms, or hosted CI. No database
was provisioned; no configured test silently skipped missing services. SQLx
example compilation does not establish database behavior or commit certainty.
No publishing or deployment occurred. This directory has no `.git` repository,
so Cargo.lock and formatting changes could not be committed here.

## Ownership, HTTP telemetry, and budget hardening: 2026-09-08

Platform: Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu. Both toolchain
versions are unchanged from the preceding table. These are local checks, not
hosted CI or production validation. No dependencies were added or updated.

Cargo.lock SHA-256 remains
`79865e6518881639a9b8776dbd73d58d924ff80f385faf0d5fe483b093e87c26`.

Executed successfully on the final Rust source (exit 0):

```sh
cargo fmt --all
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build --example http_service --features axum --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
cargo run --locked --example process_owned
cargo run --locked --example operation_budget
python3 scripts/check_package.py
```

Each complete verification run passed core compilation, **91 core tests**, **105
all-feature tests**, **one doctest**, all five example compilation targets,
Clippy, and rustdoc with warnings denied. No failures, ignored tests, or filtered
tests occurred. Twelve HTTP tests and two HTTP telemetry tests are excluded from
the core-only run. The matrix was rerun after the final ownership and
submission-context changes; scoped agent checks are not substituted for it.

The HTTP subprocess checks require successful probes, normal work, a sanitized
application 500, a middleware-deadline 503, consistent custom error envelopes and
nonempty generated correlation IDs, exclusion of untrusted correlation input,
default INFO completion/status/latency logs, and clean SIGTERM/SIGINT exit.
The deadline process checks the 503 independently: it does not require a separate
handler to beat a 1 ms budget. Startup acknowledgement replaces the old extra
sleep for signal-listener initialization. These remain short loopback smoke tests.

The finite-work example completed admitted work and dependency cleanup after
losing its receipt and final driver owner. The budget example completed both
work and finalization. New regressions cover completion/drain classification,
startup acknowledgement/failure, cancelled request and shutdown waiters,
coordinator/cleanup drop, retained coordinator panic, bounded descendant admission,
typed task failures versus normal business denial, two 64-iteration multi-thread
races, scoped subscriber continuity, finalization reserves, and reproducible jitter.
Review repaired a normal-stop/drain classification edge and propagated scoped
subscribers across owned spawn boundaries; assertions were not weakened.

Static package inspection found 105 authored tests and no broken internal links
or lexical/TOML failures. Its fixed snapshot date, `docs/package-checks.json`, and
`SHA256SUMS` belong to the original archive; they do not attest the modified source.
The actual compiler/runtime evidence is the matrix above.

Not executed: live PostgreSQL, upstream shared-library integration, hosted CI,
cross-platform execution, sustained load, full keep-alive/slow-body/streaming/
disconnect coverage, or non-yielding child-process tests. No database or external
resource was provisioned. The checkout now has Git metadata but remains
uncommitted; no commit, push, publication, or deployment was performed.

## Root-cause fixes for cancellation and shutdown: 2026-09-08

The four review findings were traced independently and reconciled before source
changes. They require internal lifetime/state corrections, not a replacement
runtime or new downstream cleanup protocol.

| Finding | Root cause and classification | Implemented correction |
| --- | --- | --- |
| Completed tasks reported as abort targets, suppressing cleanup | Local state-model error: JoinSet membership means unobserved, not unfinished; expired timers and cooperative polling expose the difference. | Harvest ready results at bounded phase boundaries; retain AbortHandles and request abort only for unfinished tasks. |
| Never-polled driver drop leaves readiness waiting | Ownership-boundary defect: the future takes ownership at construction, but its emergency guard was installed only on first poll. | Capture the guard eagerly in `run_until`; keep factories/readiness lazy and ordinary `.await` usage unchanged. |
| Scoped drop diagnostics are lost/misrouted; HTTP abort panics | Cross-cutting lifetime integration gap: subscriber propagation covered polling but not destruction, including nested span fields. | One private, allocation-free dispatcher wrapper owns the complete inner future through poll and Drop, across operation, HTTP, cleanup, and task boundaries. |
| First budget-skipped hook has no warning | Local control-flow duplication: the already-popped hook bypassed the shared skip path and retained captures while dependencies dropped. | Check budget before popping; one path reports, logs, and drops skipped hooks in LIFO order. |

Before their respective source corrections, targeted regression runs reproduced
the false aborts, unpolled readiness hang, missing cleanup warning, wrong skipped
capture-drop order, lost scoped operation completion, and HTTP cross-registry
panic. Tests that protect borrowed/non-Send compatibility already passed on the
baseline. Additional tests cover completed errors/panics, mixed finished/running
work, native !Unpin capture destruction, critical/finite task abort, cleanup-driver
abort, and cleanup-hook timeout. These later cases are coverage, not a claim that
every added test was individually run against the old source.

Platform and toolchain versions are unchanged from the preceding evidence.
Cargo generated the updated lockfile after promoting existing transitive
`pin-project-lite` 0.2.17 to a direct dependency. No resolved version changed.
Removing only that dependency edge from the lockfile in a read-only comparison
reproduces its previous SHA-256. Current Cargo.lock SHA-256:
`7bcb7ba0d656d0f996ed7c91cc49409c2b75c92ba63749dcc81324ca2eddbca5`.

Executed on the integrated final Rust source, all exit 0:

```sh
cargo fmt --all
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build --example http_service --features axum --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
cargo run --locked --example process_owned
cargo run --locked --example operation_budget
python3 scripts/check_package.py
```

Each toolchain passed **107 core tests**, **122 all-feature tests**, **one doctest**,
all five example compilation targets, formatting, Clippy, and rustdoc with warnings
denied. Zero failures, ignored tests, or filtered tests. This adds 17 tests to the
previous 105-test suite. The three HTTP process checks passed; both finite-work
and finalization-budget examples completed successfully. Static inspection found
122 authored tests and no lexical/TOML or internal-link failures. The historical
archive checksum manifest and package-check JSON were not rewritten.

The original isolated global-subscriber/scoped-subscriber reproduction also
passed after the fix: operation and HTTP abort return cancelled, not panic; the
scoped subscriber receives dropped events and the global fallback stays empty.
Repository tests use independent per-thread ambient/scoped registries to exercise
the same mismatch without global test interference. Independent code review
checked whole-future destruction, pin projection, task ownership, and unchanged
borrowed/non-Send support.

No consumer wrapper or new public runtime type is required. Genuine aborts,
panics, and unjoined work still conservatively skip dependent finalizers. Runtime
death, non-yielding work, hidden descendants, live PostgreSQL/upstream integration,
full transport/load coverage, hosted CI, and other platforms remain outside this
evidence. No commit, push, publication, deployment, or external provisioning occurred.

## Clippy complexity and length limits: 2026-09-08

Root `clippy.toml` sets cognitive complexity to 20 and function length to 250.
Both packages inherit the explicitly enabled warning-level workspace lints;
verification's existing `-D warnings` makes violations fail the check.

The initial Clippy run exited 101, reporting complexity 34 in cleanup's
`close_inner`, 29 in the supervisor's `drive_until`, and 27 in task-result
`record`. No function-length violation was reported. Private helpers now handle
cleanup result classification/abort observation, task startup/result reporting,
and waiting for shutdown. No lint suppression or threshold increase was used.

Existing failure assertions remain intact. A new component-failure regression
asserts that the shutdown future's captured guard survives through dependency
cleanup and drops when the driver finishes. Existing tests also cover cleanup
errors/panics/timeouts, conservative finalizer skipping, readiness, cancellation,
and tracing during destruction.

Executed locally, all exit 0:

```sh
cargo fmt --all
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build --example http_service --features axum --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/check_package.py
git diff --check
```

On Rust 1.98.1 and 1.94.0, each complete verification run passed 108 core tests,
123 all-feature tests, and one doctest, with zero failures or ignored tests.
Formatting, Clippy, rustdoc, and all five example targets passed. All three HTTP
smoke scenarios passed. Exact compiler/Cargo versions and platform remain those
recorded above; Cargo.lock is unchanged, SHA-256
`7bcb7ba0d656d0f996ed7c91cc49409c2b75c92ba63749dcc81324ca2eddbca5`.
Live PostgreSQL, hosted CI, other platforms, and the remaining operational
hardening work were not exercised. No commit or publication was performed.

## Jig adoption verification: 2026-09-08

Baseline: initial commit `3e64cb2`, followed by the owner's uncommitted Jig
adoption. Local platform and compiler/Cargo versions remain those recorded above:
Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu; Rust 1.98.1 and 1.94.0.
Jig runtime 0.3.0 accepts contract version 7. Cargo.lock is unchanged, SHA-256
`7bcb7ba0d656d0f996ed7c91cc49409c2b75c92ba63749dcc81324ca2eddbca5`.

The first `scripts/jig check` exited 1. Clippy rejected the shared example
`mod.rs` layout and the 107-line HTTP telemetry test under the current 100-line
limit. The default test gate also reproduced an empty-log failure in
`budget_exhaustion_reports_and_logs_each_skipped_hook_once_in_dependency_order`;
a separate repetition of `cargo test --locked --test cleanup` reproduced it on
run 9. Initial doctor failed its SQLx CLI probe, although the configured SQLx
check returned success. Neither result established any database behavior.

At the owner's request, Jig's inferred SQLx adapter, migration/metadata actions,
profile target, required command, generated CI jobs/environment, and associated
agent instructions were removed. The optional SQLx dependency and lifecycle
example remain. Both Jig test aliases now execute locked core, all-feature
all-target, and doctest commands. Shared example support moved to `support.rs`;
all five examples are explicitly declared to exclude that support module from
Cargo's executable discovery. HTTP completion assertions moved into a private
helper. No lint threshold, failure assertion, or library runtime logic changed.

The two cleanup observation tests and their fixture moved intact into
`tests/cleanup_observation.rs`, isolating their scoped subscribers from other
cleanup tests that register the same tracing callsites without subscribers.
The callsite-interference diagnosis and its limits are in [references](references.md).
After isolation, 100 fresh invocations of the following command passed, each
running 12 cleanup tests and two observation tests:

```sh
cargo test --locked --test cleanup --test cleanup_observation
```

Executed successfully on the final Rust source, all exit 0:

```sh
scripts/jig --json doctor
scripts/jig check
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build --example http_service --features axum --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
scripts/jig check agent-map
scripts/jig check agent-guides
scripts/jig check test
python3 scripts/check_package.py
git diff --check
```

Jig's five default targets passed: Clippy, formatting, tests, contract, and
file-budget. Each Rust verification matrix entry passed 108 core tests, 123
all-feature tests, one doctest, five example targets, and warning-denied Clippy
and rustdoc. No failures, ignored tests, or filtered tests occurred in these
final runs. All three loopback HTTP smoke scenarios passed.

Static package inspection initially rejected relative links in Jig's transient
adoption backup. It now excludes `.agent/.cache`, `.agent/runtime`, and
`.agent/tmp`, while retaining durable agent documents. A temporary fixture with
broken links in all three transient directories and `.agent/plans` verified
that only the durable-plan link is reported. Final repository inspection finds
123 authored tests and no lexical/TOML/internal-link failures. Historical archive
checksums and package-check JSON were not rewritten.

These results are local only. Hosted CI, live PostgreSQL, upstream application
integrations, other platforms, and the remaining operational hardening work
remain unverified. Repetition supports test isolation; it does not establish a
general tracing race or production concurrency guarantee. No further commit,
push, publication, deployment, or external provisioning occurred in this work.

## Jig footprint and installation policy audit: 2026-09-08

The repository selects the official Jig v0.3.0 release. The command below ran
in a disposable clone to review the generated changes before transferring the
release metadata and launcher/installer to this checkout:

```sh
scripts/jig update /tmp/batter-jig-fresh-e2rj43mj \
  --template https://github.com/bpcakes/jig-sh.git \
  --vcs-ref v0.3.0 --no-input --force
```

Jig resolved the tag to `8629700b92cd9ab8b09f8ff86de4fc1573469c83`, matching the
upstream tag's peeled commit. The generated launcher and installer were already
byte-identical to the release templates. Repository customizations were retained.
This is the accepted interim policy: `update --recopy` retains the revision;
ordinary `update` advances upstream. Persistent version-only enforcement remains
a future upstream feature, not a prerequisite for this change.

Removed the generated duplicate Rust and agent-map workflows, the optional
checkout helper, Swift/TypeScript plugin requests, frontend/SQLite ignore rules,
and unused proxy/package-manager settings. One Jig workflow checks installation,
contract, guides, and file budgets alongside the original Rust matrix. The
deprecated machine-local adoption receipt was removed from the change set;
append-only work records remain. Workspace bootstrap now uses `cargo fetch --locked`.
The launcher and installer retain their generated implementations.

Source packaging now includes `scripts/jig` with executable permissions, Git
attributes, and durable agent records. A temporary fixture executed the actual
packager and asserted those members were present while sentinel files in all
three transient Jig directories, the deprecated receipt, environment files,
build output, and local validation output were absent from ZIP and inventory.
Historical repository archive checksums were not regenerated.

The following passed locally on the unchanged Rust source and dependency graph:

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build --example http_service --features axum --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
scripts/jig doctor
scripts/jig check contract
scripts/jig check agent-map
scripts/jig check agent-guides
scripts/jig work check --plan-id plan_01M1ZXWMP2P73CEFVJENNG55TN
scripts/jig work evidence --plan-id plan_01M1ZXWMP2P73CEFVJENNG55TN
scripts/jig work gates --plan-id plan_01M1ZXWMP2P73CEFVJENNG55TN
scripts/jig check test
python3 scripts/check_package.py
git diff --check HEAD
```

Each Rust matrix entry passed 108 core tests, 123 all-feature tests, and one
doctest; Clippy, formatting, documentation, and all three HTTP scenarios passed.
Cargo.lock still has SHA-256
`7bcb7ba0d656d0f996ed7c91cc49409c2b75c92ba63749dcc81324ca2eddbca5`.
The moved cleanup fixture and observation tests were compared byte-for-byte
against their original bodies. Both workflow YAML files and embedded Bash
syntax parsed successfully. A local JSON-RPC client launched the exact command
from `.mcp.json`, initialized server 0.3.0, discovered its four repository tools
plus 13 agent/work tools, and inspected exactly the seven configured targets.

Fresh installation of the selected release was checked with no repository Jig
cache or runtime override. Installation and doctor exited 0 without an initialized
vault; the runtime build disabled default features. The main checkout then reused
that binary with its original source stamp and passed doctor and MCP checks.
Local and hosted evidence remain separate: no hosted CI execution, live
PostgreSQL test, publication, deployment, or commit occurred.

## Jig review fixes: 2026-09-08

Addressed the three accepted review findings. The CI file-budget helper now
compares manual runs with `origin/master`; pull-request, push, and merge-group
events keep their exact bases and provenance. A missing exact base remains a
blocking error. The repository policy workflow caches only
`.git/jig-tools/*-runtime` with actions/cache v4.3.0, pinned to its verified SHA.
Its key includes OS, architecture, runtime profile, configuration/source pin,
contract, toolchain, launcher/installer, and workflow contents. No broad fallback
key restores an unrelated runtime.

Markdown plans now use the text merge driver. The override follows the Jig
managed block, so a template refresh that restores its union rule still leaves
conflicting plan edits visible. Append-only JSONL streams retain union merging.

Added seven standard-library Python regression tests and wired them into the
Jig policy workflow. They invoke the actual event helper and selected Jig runtime
in isolated Git repositories containing `origin/master` and no local `master`.
Each event accepts a small source file and rejects growth past the configured
budget. Missing event bases block, and an all-zero push-before identity succeeds.
Assertions inspect structured receipt findings rather than the abbreviated CLI
display. Merge tests exercise actual Git merges, including a regenerated union
rule before the plan override. The archive test executes the real packager and
checks ZIP members, inventory exclusions, and launcher executable permissions.

The cache test blocks Cargo with an executable sentinel: a cold cache fails and
records the attempted install; restoring the selected runtime and its source
stamp then passes without invoking Cargo. This establishes local restored-cache
behavior, not a hosted actions/cache hit. Workflow YAML, embedded shell syntax,
cache ordering, artifact paths, and key inputs were also inspected locally.

Executed successfully:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py' -v
bash -n scripts/check_file_budget.sh
scripts/jig doctor
scripts/jig check test
scripts/jig work check --plan-id plan_01M1ZZF1CVV1XECCP6FKH71QF9
scripts/jig work evidence --plan-id plan_01M1ZZF1CVV1XECCP6FKH71QF9
scripts/jig work gates --plan-id plan_01M1ZZF1CVV1XECCP6FKH71QF9
python3 scripts/check_package.py
git diff --check
```

The Python suite passes seven tests. Jig's locked test command passes 108 core
tests, 123 all-feature tests, and one doctest. Rust source and Cargo.lock are
unchanged by this follow-up, so the earlier two-toolchain and HTTP evidence
continues to apply. No hosted CI/cache operation, PostgreSQL provisioning,
commit, push, or publication occurred.

## Effect v4 analysis reconciliation: 2026-09-08

Baseline: `5c77593c6700de9b2e8d3cbc1d2acf4bbbb0b71d`. This change updates
documentation and append-only Jig work records. Runtime source, tests, manifests,
and Cargo.lock are unchanged. Lock SHA-256 remains
`7bcb7ba0d656d0f996ed7c91cc49409c2b75c92ba63749dcc81324ca2eddbca5`.
Local platform: Linux 7.0.11-76070011-generic, x86_64 GNU/Linux;
rustc 1.98.1 (48a229cea 2026-09-01), cargo 1.98.1 (797e8a9bc 2026-08-05).

Independent source reviews checked the runtime and HTTP/context/test boundaries.
Primary upstream release, migration, and boundary references were rechecked in
[references](references.md). Proposed capabilities remain labelled unimplemented;
HTTP handler panic propagation is source-inspected with no dedicated regression.

Executed successfully:

```sh
scripts/jig doctor
scripts/jig work check --plan-id plan_01M200DQFXPVDNMTBK1E40YPD3
scripts/jig work evidence --plan-id plan_01M200DQFXPVDNMTBK1E40YPD3 --json
scripts/jig work gates --plan-id plan_01M200DQFXPVDNMTBK1E40YPD3 --json
python3 scripts/check_package.py
git diff --check
```

The configured profile passed Clippy, formatting, locked tests, repository
contract, and file budget. Tests passed 108 core cases, 123 all-feature cases,
and one doctest; all five example targets compiled. No tests failed or were
ignored. The work gate reported passed with fresh target receipts. Static
package inspection found no lexical/TOML/internal-file-link failures.

The two-toolchain verification script and HTTP process smoke were not repeated
for these documentation changes; their earlier evidence remains historical.
No new PostgreSQL, durable integration, hosted CI, production, or performance
evidence was produced. No commit or publication was requested or performed.

## Virtual workspace and Axum extraction: 2026-09-08

Baseline: `5c77593c6700de9b2e8d3cbc1d2acf4bbbb0b71d`, with the preceding
documentation changes already present in the working tree. The workspace now
contains the `batter`, `batter-axum`, and `batter-test-support` libraries plus
the unpublished `batter-example-postgres-lifecycle` executable package. The
external PostgreSQL harness and downstream repositories were not modified.

Platform: Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu. Toolchains:
rustc 1.98.1 (48a229cea 2026-09-01), Cargo 1.98.1 (797e8a9bc 2026-08-05);
rustc 1.94.0 (4a4ef493e 2026-03-02), Cargo 1.94.0 (85eff7c80 2026-01-15).
Every member retains Rust 1.94 and `publish = false`; this relocation makes
no claim of compatibility with an older compiler.

Cargo regenerated local workspace package entries during an offline check.
All external package versions, sources, and checksums match the preceding lock.
Current Cargo.lock SHA-256:
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

Executed successfully on the final Rust source (exit 0):

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
cargo run -p batter --example process_owned --locked
cargo run -p batter --example operation_budget --locked
cargo build -p batter-example-postgres-lifecycle --locked
cargo tree -p batter --edges normal --locked
python3 -m unittest discover -s scripts -p 'test_*.py' -v
python3 scripts/check_package.py
scripts/jig check agent-guides
scripts/jig check agent-map
scripts/jig work check --plan-id plan_01M201112SDGVHTPKG68SW2165
scripts/jig work evidence --plan-id plan_01M201112SDGVHTPKG68SW2165 --json
scripts/jig work gates --plan-id plan_01M201112SDGVHTPKG68SW2165 --json
scripts/jig check test
```

Each full Rust verification run passed core-only compilation and **106 core
tests**, then **127 workspace tests** (106 core, 16 Axum, five generic support),
**two doctests**, every example/binary compilation target, Clippy and rustdoc
with warnings denied. No failures, ignored tests, or filtered tests occurred.
All 123 pre-refactor named tests remain present. Four additional tests cover
the public dispatch wrapper's capture/poll/abort/unpolled destruction behavior,
borrowed non-Send work, and HTTP budget validation after extraction. The core
and adapter still use the same private pin/drop implementation. HTTP readiness,
request budgets, failure rendering, and scoped destruction assertions were
preserved; the adapter's combined RequestPolicy was not redesigned.

The three HTTP process checks passed probes, work/deadline behavior, sanitized
error envelopes, request IDs, ordinary INFO observations and clean SIGTERM or
SIGINT termination. The process-owned and operation-budget examples exited
successfully. Cargo's normal dependency tree for `batter` contains neither
Axum nor SQLx. The SQLx executable builds as an independent package. The seven
Python tooling tests passed, including source-archive coverage for nested
members and exclusion of nested build/environment artifacts. All four package
guides and the agent map passed their checks.

The unchanged 848-line lifecycle source exceeds the existing 800-line budget
by 48 lines. Its exact rename was recorded in the Git index so Jig can compare
it to the original path; the policy thresholds were not changed. An initial
intent-to-add attempt was rejected as lacking stable index authority. A
subsequent check recognized unchanged legacy debt but rejected concurrent
documentation edits as worktree drift. With edits stopped, the complete Jig
profile passed Clippy, formatting, tests, contract and file-budget checks; its
required evidence gate reported passed with fresh receipts. The final
`scripts/jig check test` also passed. No semantic tests or policy limits were
relaxed.

Local logs are retained under ignored `.agent/tmp/workspace-refactor/`.
Live PostgreSQL, external library integrations, hosted CI, production, and
performance tests were not run. No commit, publication, or deployment occurred.

## Seeded cancellation and admission exploration: 2026-09-08

Owning task: `batter-953`. This extends Git baseline
`5db16f6f18fd918d3d3558a68843120bf9a13b79` with test-local scheduling exploration,
process containment and oracle challenges. No production source, public API,
dependency or lockfile change was needed. The library's current capacity behavior
was correct; the audit exposed a missing rejecting test.

Executed on Linux x86_64 with Python **3.12.3**, default Rust **1.98.1** and MSRV
**1.94.0**. Lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
The final source includes the readiness observation refinement: an acknowledged
component remains alive until Draining is checked, so a later Stopped state cannot
hide an invalid readiness revival. The initial matrix also passed; the matrix
and corpus repeats were rerun after this test refinement.

| Executed check | Outcome |
| --- | --- |
| `bash scripts/verify.sh` | PASS, exit 0 on 1.98.1; final run 37.349 s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS, exit 0; final run 39.043 s. |
| Full scheduling corpus, two workers, three fresh processes | PASS, 32 seeds / 64 workload lifecycles / 4,096 finite completions per invocation, all eight families 32 times each; commands took 6.276–6.331 s. |
| Full scheduling corpus, four workers, three fresh processes | PASS, identical required counts; commands took 6.277–6.278 s. |
| Capacity mutation challenge, each toolchain | PASS: six original controlled replays passed and six mutant replays failed the intended shared-capacity assertion with Rust exit 101. Neither compile failure nor watchdog timeout counted as rejection evidence. |
| Blocked-runtime negative control, direct recorded run | Expected FAIL, runner exit 1 / child SIGKILL (`-9`), reaped at 3.001 s; timer-armed and runtime-blocked records retained, no profile success. |
| Unjoined finite-work negative control, direct recorded run | Expected FAIL, runner exit 1 / child SIGKILL (`-9`), reaped at 3.012 s; unjoined report, pending receipt and skipped-cleanup assertions completed before runtime destruction blocked. |
| Early-exit, capture-overflow and controlled-schedule Cargo tests | PASS in both verification matrices. Exit zero without an oracle and overflow with an early success marker were rejected. |
| `cargo build -p batter-axum --example http_service --locked` | PASS, rebuilt with 1.98.1 after the MSRV matrix. |
| `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` | PASS, exit 0; SIGTERM mode. |
| Same smoke command with `--signal SIGINT` | PASS, exit 0. |
| Same smoke command with `--deadline` | PASS, exit 0. |

Each complete Rust verification ran 162 foundation entries in the isolated core
pass, 183 workspace entries in the all-feature pass, and two doctests, plus core
compilation, Clippy and rustdoc with warnings denied. The eight scheduling entries
include two full subprocess profiles, one test of two capacity orderings, the
inert child entry and four process controls. There were no ignored tests or silent prerequisite
skips. The six explicit fresh-process repetitions account for **24,576** successful
finite workload completions; those counts exclude additional matrix runs and do
not stand in for the behavioral assertions.

The new test-owned ledger checks accepted, started and returned numeric task IDs,
including lost receipts, then reconciles the complete successful result set with
the report. Separate held gates check queued work and eight simultaneous roots
and descendants. Failure scenarios retain four concurrent task causes and two
cleanup causes by identity and name, including the exact shared typed cause;
scoped capture rejects synthetic error contents. Completion, cancellation,
readiness, scope expiry, task failure, forced closure, ownership drop and real
panic/abort cases use explicit orderings plus seeded yields. Existing deterministic
paused-time delayed-result regressions remain unchanged and ran in both matrices.

Reproduction commands, from the repository root:

```sh
cargo test -p batter --test scheduling --locked
# Resolve the executable using the JSON command in docs/testing.md, then:
python3 scripts/stress_scheduling.py --binary "$scheduling_binary" --workers 2
python3 scripts/stress_scheduling.py --binary "$scheduling_binary" --workers 4
python3 scripts/stress_scheduling.py --binary "$scheduling_binary" --workers 2 --seed 17 --schedule capacity-after-drain
python3 scripts/stress_scheduling.py --binary "$scheduling_binary" --workers 2 --seed 17 --schedule stuck --watchdog 3
python3 scripts/stress_scheduling.py --binary "$scheduling_binary" --workers 2 --seed 17 --schedule unjoined --watchdog 3
python3 scripts/check_scheduling_mutation.py --output-dir validation/local/scheduling-mutation-1.98.1
RUSTUP_TOOLCHAIN=1.94.0 python3 scripts/check_scheduling_mutation.py --output-dir validation/local/scheduling-mutation-1.94.0
```

The two negative-control commands intentionally return 1. Choose a new mutation
output directory on repetition; existing evidence is not overwritten. The final
full-matrix and repetition command ledger/logs are under
`validation/local/batter-953-final/`, with `controls.json` retaining negative-control
command/status/timing records. Mutation output directories retain the exact patch,
commands, build logs and outcome records. These ignored local artifacts are not
hosted-CI evidence. The structured Jig `verify` profile passed all five targets (Clippy, formatting,
tests, contract and file budget), and `work evidence` / `work gates` reported fresh
required evidence. Receipts are associated with `plan_01M20R9776KRBS3TBW7QGPDM38`.
The final backend command `scripts/jig check test` passed (exit 0, 35.2 s); its
receipt is retained alongside the work-profile evidence. Jig receipts are refreshed
after final documentation/tracker updates so the closing work snapshot has fresh
required gate evidence.

During implementation, initial lifetime/move compilation errors and two Clippy
complexity failures were repaired by correcting captures and separating report
assertions; no contract assertion was weakened. Inspection also distinguished
factory return from wrapper permit release: transient Full is handled as an inert
submission rejection within the family timeout. It is not a failed test retried
until green. No production race defect was found, so no production fix is claimed.

Limits: seeds reproduce scenario choices, not Tokio/OS scheduling. Fixed action
replay demonstrates the rejecting capacity oracle, not arbitrary scheduler replay.
Five-second family and 120-second profile timers rely on Tokio polling; the external
watchdog provides a separate termination deadline and bounded output, subject to
OS scheduling/reaping. A killed fixture does not demonstrate application cleanup.
macOS and hosted execution of the scheduling suite remain unverified; the macOS CI
command now includes it. These results do not establish exhaustive concurrency
correctness, detached-task termination, real HTTP load/streaming behavior or any
PostgreSQL/upstream integration.


## Scheduling subprocess ownership corrections: 2026-09-08

Owning task: `batter-953`, reopened after review. The scheduling workload and
library behavior above are unchanged; this follow-up corrects the test subprocess
protocol. The root cause was fragmented ownership of launch, process exit, pipe
completion and evidence reporting. Exceptions bypassed the evidence object, and
the mutation checker parsed console text to recover control data. The newline
omission was a local manifestation of that reporting boundary.

Before the fix, an inherited fixture flag changed inert discovery from exit 0 to
exit 101 on stdin EOF; a pipe held open could block before the emergency timer
was armed. A descendant retaining stdout caused the runner to raise after
5.106 seconds without its captured checkpoint or result summary. The overflow
fixture produced one summary marker but zero standalone summary lines. A timeout
in the build helper skipped its log write. Follow-up inspection and a live probe
also found that ignored SIGCHLD could make CPython report a child exiting 7 as
status 0 and accepted success. These were harness defects, not evidence of a
production admission/cancellation defect.

The corrected tools use explicit child arguments and a bounded PID-bound record,
a native startup deadline armed before input, one shared bounded process owner,
and structured outcomes consumed directly by the mutation checker. Direct-child
exit and pipe EOF remain separate facts. Partial output and failure metadata are
saved before classification. A non-default SIGCHLD disposition is rejected without
changing the caller's signal handler. The maximum external watchdog is now
140 seconds, followed by at most five seconds of cleanup observation, before the
149-second emergency backstop. This resolves the previous overlapping limits
without extending the task's external-deadline ceiling.

Executed on Linux x86_64 (kernel `7.0.11-76070011-generic`, glibc 2.39), Python
**3.12.3**, Rust **1.98.1** and **1.94.0**. Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
No production source, public API or dependency change accompanies the correction.
The complete command ledger, logs, environment and source hashes are in
`validation/local/batter-953-containment/` (ignored local evidence).

| Executed check | Outcome |
| --- | --- |
| `python3 scripts/test_scheduling_process.py --binary "$scheduling_binary"` | PASS, 18 controls, 7.528 s. |
| Same control command with `PYTHONOPTIMIZE=1` | PASS, all 18 controls, 8.279 s; optimization did not bypass rejection checks. |
| `python3 -m unittest discover -s scripts -p 'test_*.py' -v` | PASS, 20 tests, 73.207 s: 13 scheduling process controls plus seven Jig integration regressions. The five real Rust launch controls are supplied by Cargo/the explicit binary command. |
| Two-worker corpus, three fresh processes | PASS, each 32 seeds, 64 workload lifecycles, 4,096 completed tasks and all eight families 32 times; 6.277–6.327 s. |
| Four-worker corpus, three fresh processes | PASS, same counts; 6.226–6.276 s. |
| `bash scripts/verify.sh` | PASS on 1.98.1, 42.948 s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS, 41.694 s. |
| `python3 scripts/check_scheduling_mutation.py --output-dir validation/local/batter-953-containment/mutation-1.98.1` | PASS, 10.637 s: six originals passed and six mutants failed the expected capacity assertion with status 101, complete output and no watchdog/overflow/I/O failure. |
| Same mutation command with `RUSTUP_TOOLCHAIN=1.94.0` and output directory `mutation-1.94.0` | PASS, identical rejecting evidence, 10.485 s. |
| `cargo build -p batter-axum --example http_service --locked` | PASS, rebuilt with 1.98.1 after the MSRV matrix. |
| `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` | PASS, SIGTERM mode. |
| Same HTTP smoke command with `--signal SIGINT` and with `--deadline` | PASS in both modes. |

Each Rust verification executed 163 foundation entries in the isolated pass,
184 workspace entries in the all-feature pass, and two doctests: 349 successful
entries total, with zero failures or ignored tests. Both matrices also passed
formatting, isolated core compilation, Clippy and rustdoc with warnings denied.
The six explicit corpus repetitions account for 24,576 finite task completions;
these counts supplement the contract assertions rather than replacing them.

The process controls cover real timeout/output retention, EOF before process
exit, inherited and escaped pipe writers, overflow framing, spawn failure,
partial build/replay evidence, ignored child status, inert ambient flags,
missing/malformed launch records, parent EOF and the watchdog ceiling. Read/close
I/O failures and inability to observe reaping use narrow fault injection around
real children; no claim is made that an OS-unreapable process was created. The
escaped-pipe control explicitly reports incomplete EOF and separately terminates
its test-owned escaped fixture. Killing a group never proves arbitrary detached
work stopped or application cleanup ran.

An initial Jig contract edit accidentally included a profile entry when selecting
the two test actions, broadening the file-budget action's inputs. Contract checks
rejected it. The edit was corrected without weakening policy; native regeneration
in a disposable Git copy produced a contract exactly equal to the working one,
and `scripts/jig check contract` passed. Both Rust test targets now include their
four Python dependencies so Python-only changes invalidate their test evidence.
The required Jig verify profile passed all five targets (Clippy, formatting, tests,
contract and file budget). The final explicit `scripts/jig check test` passed
(exit 0, 40.234 s). Work receipts belong to
`plan_01M20XAG6XNY8GDQXMGJ4NJG7M`; command logs are retained with the other local
evidence.

Limits: macOS and hosted execution of these corrections remain unverified. Local
Python execution was 3.12.3; the documented 3.9 minimum follows the tool's language
features, not an executed lower-version matrix. A Python deadline cannot preempt
OS process creation or guarantee scheduling/reaping latency. Escaped pipe writers
produce explicit incomplete-output evidence; abrupt parent death has no general
application-finalization guarantee. Seeds still reproduce scenario generation,
not Tokio scheduling, and these tests do not establish exhaustive concurrency
correctness or live database/HTTP-load behavior.

## Scheduling group identity and timing margins: 2026-09-08

Owning task: `batter-953`, reopened for the next independent review's three
findings. The observation loop now defers reaping while output pipes remain open,
retaining the leader's numeric identity through the last group-signal decision.
Cached reaped status independently forbids signalling. Python control budgets
now allow three seconds for startup/work; the outer Rust watchdog assertion uses
three seconds of observation, five of cleanup and four of startup overhead.
Exit-status, checkpoint, overflow, failure classification and cleanup assertions
are preserved. No production source, public API or dependency changed.

Before the fix, `python3 -m unittest discover -s scripts -p
test_scheduling_process.py -k reaped_child -v` failed because `killpg` was called
after an explicit wait. The same command with `-k descendant_pipe` failed because
the leader already had cached exit status 0 at the group signal. After the fix,
both controls pass. A real one-second pre-checkpoint delay was killed at 0.303 s
under the former 0.3-second budget, retaining no checkpoint; the new delayed-start
control completes successfully. These probes do not claim an actual unrelated
process was killed or that macOS startup latency was measured.

Executed on Linux x86_64, Python **3.12.3**, Rust **1.98.1** and **1.94.0**.
Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
The exact command/environment ledger, logs and final source hashes are retained
under ignored `validation/local/batter-953-reap-order/`.

| Executed check | Outcome |
| --- | --- |
| `python3 scripts/test_scheduling_process.py --binary "$scheduling_binary"` | PASS, 20 controls, 27.635 s. |
| Same controls with `PYTHONOPTIMIZE=1` | PASS, 20 controls, 28.517 s. |
| Full corpus, three fresh processes per worker count | PASS, each 32 seeds, 64 workload lifecycles, 4,096 finite completions and all eight families 32 times; 6.223–6.274 s per run. |
| `bash scripts/verify.sh` | PASS, 1.98.1, 79.776 s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS, 79.846 s. |
| Capacity mutation challenge on each toolchain | PASS, 10.494 / 10.485 s; each six originals passed and six mutants failed the expected capacity assertion with status 101. No timeout or build failure counted as oracle evidence. |
| `cargo build -p batter-axum --example http_service --locked` | PASS, rebuilt with 1.98.1. |
| HTTP smoke in default, `--signal SIGINT` and `--deadline` modes | PASS, all three. |

Each full Rust verification executed 163 isolated foundation entries, 184
workspace entries and two doctests, with zero failures or ignored tests; both
also passed formatting, Clippy and rustdoc. The standalone Python suite now has
15 process controls; Cargo supplies the five additional Rust launch controls.
The six corpus repetitions account for 24,576 finite completions. Work receipts
belong to `plan_01M210XR2A7PWP4E540FHQK4GW`.
The required Jig profile passed Clippy, formatting, tests, contract and file-budget
checks; closing checks and their receipts are retained with the same work record.

Limits: macOS/hosted execution and Python 3.9 execution remain unverified. Finite
startup margins do not guarantee progress during OS suspension or starvation.
Pipe EOF, child reaping and application finalization remain separate facts; an
escaped writer still produces explicit incomplete-output evidence. The earlier
dated sections describe their respective pre-correction worktrees.

## Scheduling escalation oracle correction: 2026-09-08

Owning task: `batter-953`. Independent review identified a test assumption that
cooperative work always finishes inside a live 25 ms shutdown phase. The actual
contract allows a delayed worker to miss that deadline. The live oracle now
reconciles completion, observed abort and a recorded abort request racing with
completion or panic. It retains exact task counts, names, error categories and
conservative cleanup decisions. Production source, public APIs, dependencies,
phase allowances and seeded scenario choices are unchanged.

Two current-thread paused-clock regressions retain precise assertions. A real
thread sleep of 100 ms does not consume the paused 25 ms phase allowance; drain
and forced-cancellation modes must complete successfully with their exact phase
flags. A Tokio sleep of 100 ms crosses the shutdown deadlines and must yield the
named abort, retained JoinError, incomplete task count and skipped cleanup. Before
the oracle correction, the delayed-completion regression failed the former
`result.is_ok()` assertion. After correction both regressions pass. This is
negative evidence against the old test oracle, not a production defect.

The implementation also checked pinned Tokio 1.53.1 cancellation semantics:
normal completion or panic in the final poll can race with an abort request.
Any recorded request still requires skipped cleanup. The exact abort/completion
race is not claimed as a deterministically replayed interleaving. Existing
delayed-coordinator tests continue to prove already-finished work is not aborted.

Executed on Linux x86_64, Python **3.12.3**, Rust **1.98.1** and **1.94.0**.
Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Final command logs, environment and source hashes are retained under ignored
`validation/local/batter-953-escalation-oracle/`. The preliminary run is retained
separately under `batter-953-escalation-oracle-preliminary/`; it preceded the
additional abort/completion reconciliation and is not final-source evidence.

| Executed check | Outcome |
| --- | --- |
| `cargo test -p batter --test scheduling cooperative_escalation --locked -- --nocapture` | PASS, two paused-clock regressions, 0.314 s. |
| `python3 scripts/test_scheduling_process.py --binary "$scheduling_binary"` | PASS, 20 controls, 27.712 s. |
| Same controls with `PYTHONOPTIMIZE=1` | PASS, 20 controls, 28.487 s. |
| Full corpus, three fresh processes per worker count | PASS, each 32 seeds, 64 workload lifecycles and 4,096 finite completions; 6.276–6.583 s per run. |
| Scheduling target with two-CPU affinity and two competing CPU workers | PASS, all 11 Cargo entries, including the 20 Python controls, 31.637 s. The exact local Linux driver is retained as `contended-driver.py` with the command ledger. |
| `bash scripts/verify.sh` | PASS, Rust 1.98.1, 79.475 s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS, 80.926 s. |
| Capacity mutation challenge on both toolchains | PASS, 10.639 / 10.536 s; each six originals passed and six mutants failed the intended capacity assertion with status 101. |
| `cargo build -p batter-axum --example http_service --locked` | PASS, rebuilt with 1.98.1. |
| HTTP smoke in default, `--signal SIGINT` and `--deadline` modes | PASS, all three. |

Each Rust matrix executed 165 isolated foundation entries, 186 workspace entries
and two doctests: 353 successful entries, zero failures and zero ignored tests.
Both also passed formatting, Clippy and rustdoc. The six explicit corpora account
for 24,576 finite completions. The final source-hash audit matched the recorded
files. No failed build, timeout or preliminary run counted as mutation evidence.
Work receipts belong to `plan_01M2134C2JK2J3YBB8X4JFK4KR`.

Limits: macOS/hosted execution and Python 3.9 execution remain unverified. The
paused-clock tests prove controlled current-thread outcomes; the live corpora
and CPU-contention run do not establish exhaustive concurrency correctness or
an OS latency guarantee. Seeds reproduce generated choices, not Tokio scheduling.

The first Jig work check passed tests, Clippy, formatting and contract validation,
but file-budget scope capture rejected Git intent-to-add entries. Staging the
new Rust test files provided stable index contents; the standalone file-budget
check then passed. This changed index metadata, not source or budget policy.
Closing work-profile checks and the explicit backend check are recorded with the
same work receipts. The test files are staged; no commit or publication was made.

## Scheduling fixture lifetime correction: 2026-09-08

Owning task: `batter-953`. The remaining review findings exposed two harness
lifetime errors. The unjoined fixture reused the stuck fixture's three-second
watchdog despite needing to complete shutdown and pending-receipt checks before
its intentional hang. The escaped pipe writer became an orphan whose later
cleanup used a saved numeric process-group ID without retained child ownership.
No unrelated process was observed being signalled; that finding concerned lost
identity protection. Production source, public APIs and dependencies are unchanged.

The unjoined deadline now derives from four seconds of startup slack, the complete
five-second case allowance and three seconds of hang observation. A new replay
delays runtime creation by two seconds. With the former deadline, its Cargo parent
failed for missing report evidence after a watchdog kill at 3.001 seconds. The
fixed parent requires the startup checkpoint, reconciled unjoined report, pending
receipt, skipped cleanup, SIGKILL and absent success marker. The separate pipe
writer is now a direct child in its own session sharing a test-created pipe with
the observed child; its owner retains the handle through bounded kill/reap.
Controls cover incomplete EOF while that writer runs, cleanup after observation
raises and refusal to signal an already-reaped child. No PID-file cleanup remains.

Executed on Linux x86_64, Python **3.12.3**, Rust **1.98.1** and **1.94.0**.
Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Exact command ledgers, logs, the old-deadline failure, environment, matrix and
contention drivers, and source hashes are retained under ignored
`validation/local/batter-953-fixture-lifetimes/`.

| Executed check | Outcome |
| --- | --- |
| `cargo test -p batter --test scheduling delayed_unjoined_start_preserves_report_evidence --locked -- --exact --nocapture` | PASS, 12.147 s; the same regression rejected the old deadline. |
| `cargo test -p batter --test scheduling cooperative_escalation --locked -- --nocapture` | PASS, both paused-clock regressions, 0.321 s. |
| `python3 scripts/test_scheduling_process.py --binary "$scheduling_binary"` | PASS, 21 controls, 27.634 s. |
| Same controls with `PYTHONOPTIMIZE=1` | PASS, 21 controls, 28.470 s. |
| Full corpus, three fresh processes per worker count | PASS, each 32 seeds, 64 workload lifecycles, 4,096 finite completions and all eight families 32 times; 6.226–6.280 s per run. |
| Scheduling target with two-CPU affinity and two competing CPU workers | PASS, all 12 Cargo entries, including the 21 Python controls, 39.358 s. |
| `bash scripts/verify.sh` | PASS, Rust 1.98.1, 80.829 s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS, 81.926 s. |
| Capacity mutation challenge on both toolchains | PASS, 10.736 / 10.439 s; each six originals passed and six mutants failed the intended capacity assertion. |
| Rebuilt HTTP example and smoke in default, `--signal SIGINT` and `--deadline` modes | PASS, all three. |

Each Rust matrix executed 166 isolated foundation entries, 187 workspace entries
and two doctests: 355 successful entries, zero failures and zero ignored tests.
Both also passed formatting, Clippy and rustdoc. All 20 recorded test/tool source
hashes matched during the matrix, at completion and in the current worktree.
Work-gate and completion receipts use `plan_01M216ACYSQ3P9R1GX02JPVAX0`;
the final backend command is `scripts/jig check test`.

Limits: macOS/hosted and Python 3.9 execution remain unverified. Fixed startup
allowances and the CPU-contention run do not establish OS latency guarantees.
The outside-group writer control proves retained test ownership and incomplete
EOF; it does not prove arbitrary detached descendants terminate. Stress remains
non-exhaustive, and watchdog kills do not establish application finalization.

## Descendant closure and partial capture correction: 2026-09-08

Owning task: `batter-953`. Two further confirmed review findings concerned the
scheduling harness. The closure scenario held an admitted ancestor until after
observing closure but unconditionally required successful completion, even when
its one-second cancellation allowance expired inside the five-second case budget.
A partial `Capture` constructor failure left its selector without an owner able
to close it; selector reference cycles deferred descriptor release until garbage
collection. Both corrections preserve production APIs and the dependency graph.

The closure scenario now reconciles each admitted receipt with named report
outcomes and completed counts, preserves the original closing-task error, and
checks actual dependent cleanup against recorded abort requests. Post-closure
admission still must return `Closed`. Two paused-clock regressions exercise both
force- and task-failure closure with prompt observation and with a 1.25-second
delay. The latter rejected the previous implementation when cancellation dropped
the held ancestor's release receiver; the fixed test requires its named abort,
no unjoined work and skipped dependent cleanup. The prompt case still requires
no abort request. The ordinary corpus keeps its generated choices unchanged.

Capture construction now explicitly closes the selector on any setup exception,
including interruption, before propagating it to the process owner. The regression
uses real selectors and children, failing the second nonblocking setup or second
registration with either `OSError(EIO)` or `KeyboardInterrupt`. All four subcases
rejected the previous code because the retained selector's descriptor stayed open.
All now require descriptor closure before garbage collection, child SIGKILL,
reaping and preservation of the error category. Focused commands were:

```sh
python3 -m unittest discover -s scripts -p test_scheduling_process.py -k partial_capture -v
cargo test -p batter --test scheduling descendant_closure_ --locked -- --nocapture
```

Executed on Linux x86_64, Python **3.12.3**, Rust **1.98.1** and **1.94.0**.
Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Exact commands, before/after failures and successes, full logs, environment,
contention/matrix drivers and source hashes are retained under ignored
`validation/local/batter-953-closure-capture/`.

| Executed check | Outcome |
| --- | --- |
| Focused Python partial-construction control | PASS, all four subcases, 0.086 s; all four failed before the fix. |
| `cargo test -p batter --test scheduling descendant_closure_ --locked -- --nocapture` | PASS, both regressions and both closure causes, 0.115 s. |
| Existing paused-clock escalation regressions | PASS, both, 0.315 s. |
| `python3 scripts/test_scheduling_process.py --binary "$scheduling_binary"` | PASS, 22 controls, 27.726 s. |
| Same controls with `PYTHONOPTIMIZE=1` | PASS, 22 controls, 28.526 s. |
| Full corpus, three fresh processes per worker count | PASS, each 32 seeds, 64 workload lifecycles, 4,096 finite completions and all eight families 32 times; 6.225–6.325 s per run. |
| Scheduling target with two-CPU affinity and two competing CPU workers | PASS, all 14 Cargo entries including the 22 Python controls, 39.288 s. |
| `bash scripts/verify.sh` | PASS, Rust 1.98.1, 81.575 s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS, 83.090 s. |
| Capacity mutation challenge on both toolchains | PASS, 10.689 / 11.489 s; each six originals passed and six mutants failed the intended capacity assertion. |
| Rebuilt HTTP example and smoke in default, `--signal SIGINT` and `--deadline` modes | PASS, all three. |

Each Rust matrix executed 168 isolated foundation entries, 189 workspace entries
and two doctests: 359 successful entries, zero failures and zero ignored tests.
Both also passed formatting, Clippy and rustdoc. All 24 mutation replay records
retained expected exit status, complete EOF and reaping, with no watchdog,
overflow or observation errors. All 20 recorded test/tool source hashes matched
before and after the matrix and in the current worktree. Work-gate and completion
receipts use `plan_01M218ETNJTVXHS4TS8S885QX5`; the final backend command is
`scripts/jig check test`.

Limits: macOS/hosted and Python 3.9 execution remain unverified. Paused time checks
logical ordering, not live scheduler latency. The live oracle still rejects
unexpected errors, panics, unjoined work and inconsistent cleanup. Corpus success
is non-exhaustive, and watchdog kills do not establish application finalization.

Final repository checks also passed: all five Jig work targets (Clippy, formatting,
tests, contract and file budget), followed by `scripts/jig check test` with all
359 entries passing. The source hashes still matched after that final command.

## Scoped SIGINT ownership correction: 2026-09-08

Owning task: `batter-953`. The cleanup-interruption defect was a structural
boundary error in the private Python process owner. Catching `KeyboardInterrupt`
around observation left settlement and descriptor release exposed, and another
catch around one wait would still leave arbitrary Python instructions exposed to
signal exceptions. Python's signal guidance was researched before changing the
implementation; the source references are recorded in [references](references.md).
The stale standalone counts were a separate documentation omission caused by
repeating the same number in multiple instructions.

The private helper now owns a scoped, non-raising SIGINT handler from before child
acquisition through resource release. The handler records a stop request; normal
observation checks it, while settlement keeps its original absolute deadline.
After release, the default handler is restored and interrupted outcomes remain
failed evidence. A non-main thread or non-default SIGINT owner is rejected before
launch, alongside the existing SIGCHLD ownership check. All existing call sites
are standalone main-thread tools. Descriptor release is protected by nested
finally blocks, and simultaneous selector/stream close errors both survive.
Production Rust source, APIs and dependencies are unchanged.

The new cleanup regression uses real, directly owned children and a real retained
selector. An outside-group writer keeps a pipe open while one or three real
SIGINTs are delivered after cleanup's selector polls. Both subcases rejected the
old code with `SIGINT escaped cleanup without an outcome`. They now require the
original cleanup deadline, retained checkpoint, interrupted error category,
SIGKILL/reaping of the observed child, incomplete EOF and closed descriptors.
Other controls deliver SIGINT during setup, observation and close, verify handler
restoration after callback failure, and reject incompatible handlers/threads
without starting a child. The two stale numeric instructions now refer to process
controls without duplicating their count; executable discovery and the coverage
summary retain the actual current totals.

Focused commands:

```sh
python3 -m unittest discover -s scripts -p test_scheduling_process.py -k sigint -v
python3 -m unittest discover -s scripts -p test_scheduling_process.py -k close_failure -v
```

Executed on Linux x86_64, Python **3.12.3**, Rust **1.98.1** and **1.94.0**.
Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Exact command ledgers, before/after failure evidence, full logs, environment,
contention/matrix drivers and source hashes are retained under ignored
`validation/local/batter-953-sigint-ownership/`.

| Executed check | Outcome |
| --- | --- |
| Focused SIGINT controls | PASS, four controls covering real signals and ownership/restoration, 1.017 s. Both cleanup subcases failed before the fix. |
| Simultaneous selector/stream close-error control | PASS, both error categories and descriptor release retained, 0.165 s. |
| Existing paused-clock escalation controls | PASS, both, 0.316 s. |
| `python3 scripts/test_scheduling_process.py --binary "$scheduling_binary"` | PASS, 26 controls, 28.629 s. |
| Same controls with `PYTHONOPTIMIZE=1` | PASS, 26 controls, 31.291 s. |
| Full corpus, three fresh processes per worker count | PASS, each 32 seeds, 64 workload lifecycles, 4,096 finite completions and all eight families 32 times; 6.276–7.229 s per run. |
| Scheduling target with two-CPU affinity and two competing CPU workers | PASS, all 14 Cargo entries including the 26 Python controls, 40.247 s. |
| `bash scripts/verify.sh` | PASS, Rust 1.98.1, 81.528 s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS, 80.477 s. |
| Capacity mutation challenge on both toolchains | PASS, 11.638 / 11.034 s; each six originals passed and six mutants failed the intended capacity assertion. |
| Rebuilt HTTP example and smoke in default, `--signal SIGINT` and `--deadline` modes | PASS, all three. |

Each Rust matrix executed 168 isolated foundation entries, 189 workspace entries
and two doctests: 359 successful entries, zero failures and zero ignored tests.
Both also passed formatting, Clippy and rustdoc. All 24 mutation replay records
retained their expected statuses, complete EOF and reaping, with no watchdog,
overflow or observation errors. All 20 recorded test/tool source hashes matched
before and after the matrix and in the current worktree. Work-gate and completion
receipts use `plan_01M21AB7YKJ8HDRC8SVB1S1ETQ`; the final backend command is
`scripts/jig check test`.

Limits: macOS/hosted and Python 3.9 execution remain unverified. Signal handling
belongs to these synchronous private tools; it is not a new library handler or
support for arbitrary custom signal owners, interpreter threads, asynchronous
exceptions or process death. Process creation, OS scheduling and non-yielding
work can still exceed cooperative deadlines. Stress remains non-exhaustive, and
watchdog kills do not establish application finalization.


## Inherited SIGINT policy and unjoined elapsed checks: 2026-09-08

The review found a local signal-policy design error: the private process owner
classified every non-Python-default SIGINT disposition as competing ownership.
A non-interactive background Cargo run inherited SIG_IGN and failed
`two_worker_scheduling_corpus` before child creation with
`unsupported-sigint-owner`, despite the foreground scheduling target passing.
The missing elapsed assertion on the unjoined fixtures was a separate test
omission: eventual SIGKILL could satisfy their oracle even after fallback to the
140-second full-profile watchdog.

Primary Python 3.12 documentation, CPython 3.12.3 initialization and GNU Bash
signal documentation were checked before implementation; see
[references](references.md#inherited-sigint-policy-in-scheduling-tools-2026-09-08).
The correction distinguishes inherited policy from a competing callback. Ignored
SIGINT stays ignored in both owner and child. Python-default and SIG_DFL retain
the scoped non-raising recorder, and every path restores the exact prior
disposition after resource release. The main-thread/default-SIGCHLD prerequisites,
custom/unknown-handler rejection, output evidence, child identity and cleanup
deadlines remain enforced.

Three new standalone controls exercise background signal inheritance/delivery,
all compatible dispositions over success/spawn failure/callback failure/timeout,
and real SIGINT under SIG_DFL. Before the helper changed, these produced ten
failed subcases; afterward all seven focused SIGINT controls passed. A fourth
new control runs the real Rust replay through a background shell. Existing tests
that intentionally send active SIGINT explicitly establish their signal policy
and restore the launcher's disposition afterward. Both unjoined parent tests now
require elapsed time from twelve to less than twenty-one seconds; a synthetic
140-second result must fail the same elapsed oracle without sleeping for it.

The final matrix ran on Linux x86_64 (kernel `7.0.11-76070011-generic`, glibc
2.39), Python 3.12.3 and Rust 1.98.1/1.94.0. All 23 recorded commands succeeded.
Evidence is retained under ignored
`validation/local/batter-953-inherited-sigint/`: exact argument vectors,
environment, wall-clock durations, before/after logs, the matrix driver, source
hashes, corpus output and mutation records. Cargo.lock remained unchanged at
SHA-256 `3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

| Executed check | Observed result |
| --- | --- |
| Focused SIGINT controls (`python3 -m unittest discover -s scripts -p test_scheduling_process.py -k sigint -v`) | Seven passed; 1.367 seconds. |
| Synthetic unjoined elapsed oracle (`cargo test -p batter --test scheduling unjoined_elapsed_oracle --locked`) | Expected late-duration rejection passed; 0.114 seconds. |
| Complete scheduling target (`cargo test -p batter --test scheduling --locked`) in foreground and through a `/bin/sh` background command with explicit child-status propagation | Fifteen tests passed in each context. Background command took 29.134 seconds; the foreground test target reported 29.01 seconds. |
| Existing controlled escalation regressions | Both passed; 0.315 seconds. |
| Python controls with the built Rust fixture, normal/background/`PYTHONOPTIMIZE=1` | Thirty passed in each mode; command durations 29.029 / 29.029 / 30.292 seconds. |
| Three fresh full corpora for each of two/four workers | All six passed in 6.276–6.582 seconds each; every run recorded 64 workload cycles, 4,096 completions and all eight families at 32 cases each. |
| Scheduling target restricted to two CPUs with two competing CPU workers | Fifteen passed; 41.773 seconds, including both unjoined elapsed assertions. |
| `bash scripts/verify.sh` on Rust 1.98.1 | 361 test executions passed, zero failed/ignored; formatting, Clippy and rustdoc passed; 88.960 seconds. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | 361 test executions passed, zero failed/ignored; formatting, Clippy and rustdoc passed; 86.801 seconds. |
| Capacity mutation challenge on each toolchain | Each passed six originals and rejected six mutants with status 101 at the intended shared-capacity assertion; 12.994 / 12.091 seconds. |
| Rebuilt HTTP example and default, SIGINT, deadline smoke modes | All three passed with the expected telemetry/response checks and exit zero. |

All 20 implementation/test/script hashes matched before and after the matrix and
when audited afterward. All 24 mutation replay records showed reaped children,
complete EOF and no watchdog, overflow or I/O errors. Mutant logs contained the
intended capacity assertion and no completed-profile marker. Corpus outcomes
were also audited for counts, markers, successful status and complete observation.

The closing Jig gate/backend logs and evidence are retained beside the matrix
logs; plan `plan_01M21CBBH20FMCC3Q6FP0PQDBP` connects `work check`, the final
`scripts/jig check test`, `work evidence`, `work gates` and `work finish` receipts.
These are local results. macOS/hosted scheduling and Python 3.9 execution remain
unverified, and no PostgreSQL service was provisioned. The corpus is not exhaustive;
process creation and OS scheduling remain outside hard preemption guarantees.
No production API, dependency graph or publication change accompanies this fix.

### Test inventory correction (2026-09-08)

The review follow-up for `batter-953` corrected stale discovery counts in
`docs/testing.md`. On Linux with Rust 1.98.1,
`cargo test --workspace --all-features --locked -- --list` succeeded and listed
190 test entries: 169 foundation, 16 Axum adapter and five test-support entries,
plus two foundation doctests. The scheduling executable listed fifteen entries.
Python 3.12.3 `unittest.TestLoader().loadTestsFromTestCase(...).countTestCases()`
reported 24 `ProcessTests` and six `FixtureLaunchTests` in
`scripts/test_scheduling_process.py`, matching the documented 30 controls.
The other-Unix total of 188 is derived by excluding the two Linux-only probes;
this correction adds no macOS execution evidence. These were discovery checks,
not another runtime test pass; the implementation and preceding matrix evidence
are unchanged. The implemented-status row now links to the test inventory
instead of duplicating its Python count.
