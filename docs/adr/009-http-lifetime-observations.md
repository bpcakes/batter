# ADR-009: Measure HTTP connection lifetimes without extending request ownership

Status: accepted, 2026-09-10. Owning Bead: `batter-u0m`.

## Context

A returned Axum `Response`, a finished HTTP message, a destroyed body, a closed
socket, and a joined serving wrapper are different observations. Router-only
tests cannot establish their relationship. `register_http` supervises the native
`axum::serve(...).with_graceful_shutdown(...)` wrapper. Axum itself spawns
connection tasks; Batter does not receive their join handles or individual errors.

## Decision

Preserve the response-construction boundary. Add real HTTP/1.1 loopback tests in
[`http_lifetime_observations.rs`](../../crates/batter-axum/tests/http_lifetime_observations.rs), using the
same native serve/graceful composition and unchanged admission/observation APIs.
A test-owned listener forwards native socket operations and records handle drop.
The fixture records actual wrapper returns separately from body, client and
socket failures; it does not convert these failures into invented server errors.

Ordinary cases deliver lifecycle drain directly to graceful shutdown. One
companion control holds back that delivery, using only test-owned synchronization,
so a second request on an established connection must reach guarded routing.
This verifies the admission rejection independently of transport closure.

## Measured behavior

The local resolved graph is Axum 0.8.9, Hyper 1.11.1, hyper-util 0.1.20 and Tokio
1.53.1. See [primary source inspection](../references.md#http11-lifetimes-reviewed-2026-09-10)
and [execution evidence](../validation.md#http11-connection-lifetimes-2026-09-10).

| Scenario | Required observations |
| --- | --- |
| Idle established keep-alive connection | A complete first response, drain, client EOF and server socket destruction; direct server success precedes dependency cleanup. |
| Second request during ordinary drain | Configured 503 or transport closure/reset; no second business entry. The companion control requires two admission-boundary entries, one business entry, and `service_unavailable` before graceful delivery. |
| Admitted pending handler | Entry before drain, native connection graceful acknowledgement followed by pending wire/live handler and active context at a bounded checkpoint, then explicit release, complete 200, context cancellation, EOF, server return and cleanup. |
| Incomplete upload inside the handler | An acknowledged pending body read with four of 100 declared bytes sent; drain starts while the read is pending, and the 150 ms response budget destroys the upload body and handler, returns complete 503 `deadline_exceeded`, and emits one `server_error` observation. |
| Blocked response body, cooperative drain | 200 headers and first chunk, one `completed` HTTP observation, cancelled escaped context, and a pending body beyond the request deadline. Native connection graceful acknowledgement precedes a further pending-wire/live-body checkpoint; explicit body release later produces the terminal chunk, body destruction and EOF; the server completes before cleanup. |
| Forced process cancellation | Pending handler entry with a 60 s request deadline; drain exhausts its 100 ms grace. Cancellation destroys the handler, cancels its context, returns complete 503 `operation_cancelled`, and emits one `server_error`. The server subsequently returns successfully, so cleanup runs even though `forced_cancellation` is true. |
| Forced wrapper abortion | Headers and pending body poll precede drain. After 100 ms grace and 100 ms cancellation allowance, report names `http` in `abort_requested`, retains `TaskOutcome::Aborted` and its cancelled `JoinError`, has no unjoined direct task, and skips the dependency with `UnsafeTaskExit`. At report inspection, body and server socket are still owned and a 50 ms read checkpoint remains pending. Only later test release completes framing and permits body/socket destruction and client EOF; cleanup invocation count remains zero. |
| Full client disconnect before response | After handler entry, client calls native `Shutdown::Both` and drops its socket. Handler/context and server socket destruction are observed before any handler release; observation is one `dropped` event with no status. |
| Full client disconnect while streaming | After 200 headers, first chunk and pending body poll, client closes both directions and drops the socket. Body and server socket destruction are observed before test release; there is no terminal chunk/body completion and no second HTTP observation or replacement status. |
| Client write-half closure before response | Client calls `AsyncWriteExt::shutdown` and retains the read side. With the resolved native default (`half_close = false`), the server drops the pending handler and closes the connection; the client observes EOF without a response. This is a distinct action from a full local close. |

Disconnect regressions require positive resource/socket-drop acknowledgements
within one shared second after close/EOF, before test-owned release or lifecycle drain. They fail if
the resource survives to that checkpoint; they do not accept either outcome
merely because it was logged. The blocked-body abort case instead requires the
resource still to be owned at its explicit report/read checkpoint and reconciles
its later destruction separately. A bounded pending read does not establish
indefinite survival, and a local socket drop does not establish remote delivery.

## Verification ownership and consequences

Both HTTP targets now share `tests/support/http_process.rs` and the native Unix
process machinery in `test-support/process/`. Exact argv plus PID-bound stdin
select the fixture; ambient environment does not. The parent kills/reaps after
eight seconds, while parent-pipe EOF and a ten-second emergency thread provide
independent containment. Normal success requires completion naming the exact
scenario after exercise/teardown and runtime destruction. Controls reject missing
or wrong completion, zero selected tests and ambient `stall` during discovery.
The deliberate stall requires SIGKILL/reaping and cannot prove cleanup; another
control retains both exercise and finalizer failures. Event storage returns owned
snapshots so assertion unwinding cannot poison subsequent Drop evidence.
Normal Cargo discovery, the workspace matrix and Jig gates execute these controls.

The two suites retain distinct fixture responsibilities: ADR-008 covers raw wire,
report and bounded startup/teardown diagnostics; this suite adds instrumented
native IO, upload polls, post-response deadline and write-half-close observations.
Shared mechanics and one contract/status summary prevent launch/evidence policy
from drifting while preserving both sets of semantic assertions.

Successful cleanup is evidence about the actual direct server exit and this
fixture's observed native connection milestones. It is not a transitive task-join
promise. The serving wrapper is not an aggregate connection-error report. The
suite makes no HTTP/2, WebSocket, TLS, load-capacity, universal/immediate disconnect,
or runtime-death guarantee. The library still needs a separately designed owner
for streaming lifetimes and detached application tasks. No production API or
runnable HTTP example behavior changes in this decision.

## Complete phase budgets, 2026-09-10

Follow-up Bead `batter-88d` repairs a composition omission: replacing the original
20 s process runner with the shared 8 s watchdog did not budget this fixture's
sequential waits. A 3 s exercise wait plus 4 s report wait plus 3 s body wait could
lose combined diagnostics to SIGKILL. Individual bounded waits did not establish
a bound for their owner.

Both HTTP drivers now use the same 1 s startup, 2 s exercise and 3.5 s teardown
allowances, checked with 1 s reserve below the 8 s parent limit. Observation
shutdown allowances fit teardown through `ShutdownBudget::total_allowance()`;
ordinary drain is 2 s and the existing 100 ms forced drain/blocked-body cancellation
remain. Other cancellation and abort observation are 300 ms each; cleanup has
500 ms total and 100 ms abort observation. Production service policy is unchanged.

The fixture returns its running owner before readiness waiting. Exercise timeout
is inside the spawned task, whose join completes before teardown starts. Report
waiting and later reconciliation share one absolute teardown deadline; the report
stays outside reconciliation so a later timeout/panic cannot erase it. New real
process controls combine two yielding timeouts with retained report/Drop evidence,
and withhold readiness while proving owned teardown. Both require ordinary failure
exit rather than watchdog termination. Non-yielding work still needs the outer
watchdog; no runtime-death cleanup guarantee follows.


## Terminal report ownership, 2026-09-10

Follow-up Bead `batter-rv8` completes the phase split: terminal observer waits
remained inside exercise after the driver acquired teardown ownership. Their
shorter enclosing deadline could reject a shutdown that fit teardown. Both
fixtures now validate terminal reports and dependent ordering after exercise.
Only the blocked-body abort scenario exposes a named intermediate report wait,
which remains essential to the before-release ownership oracle.

Each suite has a delayed-success case with a real 2.2 s finalizer. Its dedicated
3.1 s total shutdown allowance fits teardown; exercise-complete precedes cleanup
and terminal report observation. A failed reconciliation event wait also retains
the actual report and destructor evidence. These controls complement combined
failure tests, which alone could not detect success-path budget overlap.

## Cancellation diagnostics and terminal assertions, 2026-09-10

Follow-up Bead `batter-538` removes private three-second event/wire timers. Those
timers competed with exercise, disconnect and remaining teardown time, making the
failure shape and missing-wait evidence depend on which timer won. Private wait
guards now retain names/markers, partial wire bytes and event snapshots on Drop;
the driver keeps capture and reports outside the joined phase futures. Fast and
delayed missing-reconciliation controls both require teardown's timeout. Separate
exercise and disconnect controls verify shorter cancellation paths.

Both suites repeat keep-alive handler counts after the terminal report. Late-entry
injection after exercise verifies the final oracle independently of the EOF check.
The delayed-report policy uses 400 ms drain, 100 ms cancellation, 100 ms abort
observation, 2.7 s cleanup and 100 ms cleanup reap: 3.4 s within 3.5 s teardown.
This reserves more drain scheduling room without altering ordinary fixture policy.
