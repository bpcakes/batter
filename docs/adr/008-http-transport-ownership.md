# ADR-008: Observe HTTP transport lifetimes separately

Status: accepted, 2026-09-10. Delivery evidence: `batter-u0m`; component comparison:
`batter-zu8`.

## Context

The HTTP example registers `axum::serve(...).with_graceful_shutdown(...)` as a
critical component. Lifecycle drain drives graceful shutdown. Request admission
uses readiness, while each admitted `OperationContext` inherits downward
cancellation from process forced cancellation. The middleware returns when it
constructs a Response. None of these boundaries alone establishes body
completion or socket closure.

Resolved Axum 0.8.9 spawns connection tasks. Its graceful serving future waits
for connection-completion receivers to drop, but aborting that future does not
join those connections. Its returned `io::Result` is not a connection-error
aggregate. See [versioned primary sources](../references.md#http11-transport-ownership-reviewed-2026-09-10).

## Decision

Keep the existing production API and response-construction boundary. Require
separate executable observations for handler entry/destruction, response headers,
body polling/completion/destruction, complete HTTP message framing, socket
EOF/error, direct server result and dependency cleanup.

The loopback suite in `crates/batter-axum/tests/http_lifetime.rs` uses the normal
owned server composition. An additional companion case delays graceful signal
delivery using test-owned synchronization after readiness withdraws. The second
request on an established connection must reach admission and return 503 without
entering the business handler. This closes the coverage gap where transport
closure could otherwise satisfy every admission test.

Cooperative handler/body release follows a native connection-task graceful
milestone and a bounded held-work checkpoint. The shared observation helper
requires a current-thread runtime and the resolved Axum 0.8.9 trace/call ordering;
producer and accept-loop events cannot substitute. See [testing](../testing.md)
and follow-up `batter-mhp` for the repaired synchronization and mutation evidence.

The measured HTTP/1.1 behaviors are:

| Trigger | Independently asserted outcome |
| --- | --- |
| Drain with idle keep-alive | Socket closes; server returns Ok before cleanup. |
| Drain with an admitted pending handler | Handler remains live and its context remains uncancelled at the drain checkpoint; release produces 200, complete framing and socket closure. |
| Additional request during ordinary drain | 503 or transport closure, with no second business-handler entry. |
| Incomplete upload consumed inside guarded handler | Response deadline destroys the handler and returns 503 `deadline_exceeded`. |
| Headers sent with a blocked streaming body | One 200 observation and cancelled escaped request context coexist with a pending body; release separately produces final framing, body destruction and socket closure. |
| Forced cancellation with a pending handler | Handler drops, context cancels and 503 `operation_cancelled` is observed; direct server completion permits successful cleanup without abortion. |
| Server-wrapper abortion with blocked body | Report retains named Aborted/JoinError and requested abort, with no unjoined direct task; dependency cleanup is skipped. The body remains live at report inspection and completes only after test release. |
| Full client disconnect before response or during streaming | With this resolved transport, the corresponding handler/body drops before any test release, within a one-second observation bound. Pre-response observation is dropped without status; streaming retains its sole earlier 200 observation. |

Disconnect uses native `shutdown(Shutdown::Both)` followed by closing the client
socket. It is not a write-half shutdown. A timeout waiting for destruction fails
the regression; recording either possible outcome is not success. The bounded
checkpoint does not establish universal disconnect propagation or an OS latency
guarantee. Body completion (poll returning None), complete chunk framing and
socket EOF/error remain distinct assertions, even when they occur close together.

Dependency upgrades intentionally rerun these regressions as compatibility gates.
If a new Axum/Hyper version changes the measured disconnect outcome, investigate
the upstream change and record new platform evidence before explicitly revising
this ADR and its acceptance contract. Do not silently loosen the one-second
checkpoint or accept either outcome just to make an upgrade pass.

Every real scenario runs in a child process with the existing Unix PID-bound
stdin launch and watchdog. The parent kills and reaps after eight seconds; an
independent ten-second emergency exit and parent-death pipe cover lost parent
control. A deliberately stalled Tokio runtime must be killed/reaped and rejected
as successful lifetime evidence. Exercise task and teardown failures are retained
together. The runtime remains alive through abort-report inspection and later
body/connection teardown; runtime destruction cannot supply the proof.

## Consequences and exclusions

Successful direct supervision can coexist with hidden descendants when a
component violates its ownership protocol. The generic comparison independently
pings such a child after successful direct reporting and cleanup. The library
cannot detect it or automatically suppress cleanup. A conforming component
acknowledges actual initialization and joins its children before returning.

The HTTP suite establishes this specific native composition on recorded
platforms, not HTTP/2, WebSockets, load capacity, universal disconnect behavior,
transitive joining after abort, or a new body-lifetime API. Applications requiring
stronger ownership must design that protocol explicitly. No automatic retry or
remote side-effect certainty follows from a response, disconnect or timeout.

The follow-up in [ADR-009](009-http-lifetime-observations.md) shares the process
protocol with instrumented observations. Both require scenario-specific completion
after exercise and teardown, and reject ordinary zero-test exits. The private
EventLog owns locking and returns snapshots so failed assertions preserve Drop
recording. HTTP fixture policy lives with these suites, outside generic process
mechanics and the public test-support crate.

## Amendment, 2026-11-11: forced connection close (`batter-9wig.1`)

Context: the wrapper-abort row above is reachable by any client. A pending
streaming body, a peer that stops reading a large fixed response and a request
head that never completes each hold Axum's graceful wait past the cancel
allowance. The supervisor then aborts the server wrapper and, under ADR-002,
skips every dependent finalizer. A real-socket probe reproduced this with an
ordinary non-streaming handler returning 64 MiB to a client that stopped
reading.

Decision: `low_level::register_http`, `register_http_in`,
`register_http_with_connect_info_in` and both `AssembledHttp` registration
methods wrap the listener in a private gate. `ShutdownSignal::abort_deadline`
exposes the stop time plus the drain and cancel allowances. The registered
component future, not a spawned task, waits for forced cancellation, then until
halfway to that deadline, and closes the gate: every accepted connection's reads
and writes fail and parked I/O is woken. Hyper ends those connections in their
own tasks, Axum's graceful wait returns, and the wrapper is joined. The first
half of the allowance is reserved because forced cancellation also cancels
admitted request contexts, and the 503 `operation_cancelled` those handlers
render must still be written whole. Owning the accept loop was rejected: it
would duplicate Axum's serving path and gain nothing the transport gate lacks.

The response-construction boundary, ADR-002's conservative skipping and the
public registration signatures are unchanged. The amended row is:

| Trigger | Independently asserted outcome |
| --- | --- |
| Connection still open halfway through the cancel allowance, served through a registration helper | Its I/O fails; body or handler is destroyed before the report; the server returns joined, `forced_cancellation` is true, no abort is requested and cleanup runs. |
| The same with a zero cancel allowance, or a server registered directly with `axum::serve` | The original wrapper-abort row: named abort, body still owned at report inspection, cleanup skipped. |

Consequences: the two HTTP lifetime suites keep registering `axum::serve`
directly, because they instrument the native graceful milestones; they remain
evidence for the ungated native composition, including the abort row. The
library path is covered by `tests/operational/forced_close.rs` and a TLS case.
Not covered: HTTP/2, upgraded connections (which leave Axum's accounting at the
upgrade and stay application-owned), tasks a handler detached, a connection task
that never yields, and a stop clock tightened after cancellation was observed.
A library-owned lifetime for streaming bodies was assessed in `batter-9wig.2` and
delivered as `AdmittedRequest::into_streaming_response` under `batter-9wig.6`.
The response-construction boundary itself is unchanged: the owner starts a
second lifetime from the same admission and does not extend the first. Its
contract and limits are in the [HTTP guarantees](../guarantees.md#http-boundary).
See [primary sources](../references.md#forced-connection-close-at-shutdown-2026-11-11).
