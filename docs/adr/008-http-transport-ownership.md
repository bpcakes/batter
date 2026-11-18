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
halfway from `ShutdownSignal::forced_cancellation_at`, when that cancellation
began, to that deadline, and closes the gate: every accepted connection's reads
and writes fail and the task that last polled each connection's I/O is woken,
whether or not that poll was pending, because Hyper can wait on a body alone
after ready polls. A component that reveals an earlier stop moves the deadline
earlier; `ShutdownSignal::abort_deadline_before` reports that, and the midpoint
is recomputed from the same cancellation instant. A deadline that has already
passed leaves the gate open, as a zero cancel allowance does. Hyper ends those connections in their
own tasks and Axum's graceful wait returns. Axum counts a connection finished
before its task destroys the connection future, which after an I/O error still
holds the in-flight handler future and the response body, so each connection's
service, response futures and response bodies hold a completion token and the
registered task also waits for every token to be released before the wrapper
is joined; a destructor still running at the abort deadline leaves the wrapper
to be aborted and cleanup skipped. The first
half of the allowance is reserved because forced cancellation also cancels
admitted request contexts, and the 503 `operation_cancelled` those handlers
render must still be written whole. Owning the accept loop was rejected: it
would duplicate Axum's serving path and gain nothing the transport gate lacks.

The response-construction boundary, ADR-002's conservative skipping and the
public registration signatures are unchanged. The amended row is:

| Trigger | Independently asserted outcome |
| --- | --- |
| Connection still open halfway through the cancel allowance, served through a registration helper | Its I/O fails; body or handler is destroyed before the server returns joined; `forced_cancellation` is true, no abort is requested and cleanup runs after that destruction. |
| The same, with a body or handler destructor still running at the abort deadline | The wrapper-abort row: named abort, cleanup skipped. |
| The same with a zero cancel allowance (the gate never closes), a connection task that never yields, or a server registered directly with `axum::serve` | The original wrapper-abort row: named abort, body still owned at report inspection, cleanup skipped. |

Consequences: the two HTTP lifetime suites keep registering `axum::serve`
directly, because they instrument the native graceful milestones; they remain
evidence for the ungated native composition, including the abort row. The
library path is covered by `tests/operational/forced_close.rs` and a TLS case.
Not covered: HTTP/2, upgraded connections (which leave Axum's accounting at the
upgrade and stay application-owned), tasks a handler detached, and a
connection task that never yields.
A library-owned lifetime for streaming bodies was assessed in `batter-9wig.2` and
delivered as `AdmittedRequest::into_streaming_response` under `batter-9wig.6`.
The response-construction boundary itself is unchanged: the owner starts a
second lifetime from the same admission and does not extend the first. Its
contract and limits are in the [HTTP guarantees](../guarantees.md#http-boundary).
See [primary sources](../references.md#forced-connection-close-at-shutdown-2026-11-11).

## Amendment, 2026-11-18: owned-body outcomes describe the body (`batter-9wig.11`)

The owned body first reported what the peer received. To do that it mirrored
Hyper 1.11.1's private HTTP/1 encoder and refusal rules, settled the
advertised length again at request admission, and associated each body with
the message that carried it through a thread-local the registered server set
around Hyper's calls. Review rounds under `batter-9wig.6`, `.8`, `.9`, `.10`
and `.14` each confirmed another case where the prediction or the association was
wrong: a nested in-process response, an HTTP/2 request, a spawned admission, a
body whose size changed between two questions, a body its handler kept while
returning another response. Under ADR-010 this is a library design gap, not a
series of implementation mistakes: the adapter reconstructed a decision that
belongs to the transport and that a body cannot observe, and `batter-axum`
declares no Hyper version that could pin it.

Decision: an owned body's outcome follows only the length the response its
builder returned declares, read once at the hand-over, and what its source
then does. The mirror, the admission settlement, the message association and
the registered server's framing record are removed. The rule is identical
in-process and on any server. The outcome is not, since it also depends on
whether the body's holder stops polling or polls on, so an in-process result
is not evidence of a served one. The outcome is documented as a description
of the body, not a delivery receipt.

Accepted costs: a head Hyper refuses is no longer detected, so the body under
it, which Hyper did not poll in the four cases exercised, is judged like any
other, by the length its
builder's response declared, the bytes it had yielded and what its source
reports, where it was `failed` at WARN: in the cases exercised, none polled
before, `dropped` if pending with no length declared, `completed` if already
at its end or pending having declared zero bytes, and still `failed` at its
end short of its declared length. A length added after the hand-over is not
read, so where it is the only declaration a source ending short of it is
`completed` rather than `failed`, and a source that yields those bytes and
stays pending is `dropped` rather than `completed` when its holder destroys
it there, as Hyper's HTTP/1 connection does. The public API
still lets a handler declare the length outside the builder. That is recorded
as a residual under ADR-010: the invariant cannot be enforced locally without
sealing the response, and the misuse changes only an observation label.
Alternatives rejected: keeping the mirror behind a pinned Hyper version, which
leaves the association defects; and keeping only the admission settlement,
which keeps the mechanism the last confirmed defect came from.

A second assessment followed the same day, from review rounds under
`batter-9wig.14`. So that a start refused after the response still reports to
the request's subscriber, the request's dispatcher had been kept in the
shared observation record. Three consecutive rounds each confirmed a defect in
that: the record outlived its observer and reached a later request with
copied extensions; a check that the observer was alive did not establish
which request it had seen; and the record kept its subscriber alive for as
long as any copy of the request's extensions existed. This is the same kind
of gap: shared, copyable request metadata was asked to prove which request an
event belongs to. Decision: the shared record holds no dispatcher. The
event's dispatcher is the one that owns the request span, which the span
itself names, so the two cannot disagree; where there is no such span, it
is the one the request was admitted under, recorded beside the correlation
the event carries and held only weakly, because the admission record is a
copyable request extension too: the first form of this decision kept it
strongly there, and the next round found that a copied extension map then
kept the subscriber alive. Once the request's observer has finished, none of
the library's request records holds a subscriber strongly, and an ended body
holds none. A body started while a
copied record's observer is still alive still enters that observer's span;
copying one live request's Batter records into another is outside the
supported redispatch and stays a documented limit. For a related reason the
outcome documentation states the complete rule once and gives examples only
as exercised cases with every input named: three rounds had each found an
example that omitted a fact its outcome depends on.

A third assessment, same day and task. To keep foreign waker code out of the
body's ending, the body was given a waker of its own between the polling
task and everything it waits on. The next two review rounds found three
defects in it each: muting it stranded a pending poll of a shared body, its
replacement at a poll was not unwind-safe, and it shifted idle accounting.
It also could not cover other tasks waiting on the same scope. Decision: no
such waker. After a poll or a question of the body unwinds, its scope is
cancelled with that unwind caught, not from a guard's destructor inside it.
A body destroyed while its holder unwinds for a reason of its own still
cancels its scope from its own destructor, during that unwind.

A fourth assessment set the boundary those rounds had been circling. The
next review showed that a task waker which panics when woken defeats Tokio
itself: its notification of the remaining waiters stops, and tokio-util
0.7.19 poisons a parent cancellation token. A library cannot promise more
than the primitives it stands on. Decision: the owned body's unwind
guarantees cover the code it calls on the application's behalf, namely the
response body and what it yields, and the tracing subscriber. Task wakers
are runtime infrastructure assumed not to unwind when woken, cloned or
destroyed; the library makes no claim for a waker that does, and the code
and tests that had tried to were removed.

A fifth assessment applied the same reasoning one layer up. Reviews had for
many rounds found paths on which a tracing subscriber that panics, on a span
entry or exit, on an event or in its destructor, left something of the
body's ending undone, and each repair exposed the next: a subscriber that
panics on two consecutive events aborts the process, because the body emits
its event from a destructor, and `tracing-subscriber` leaves its registry's
current span behind when a layer callback panics. Decision: a panicking
tracing subscriber is outside the contract as well. The unwind guarantees
cover the application's code for the body, namely the builder, the source's
polls and questions, and the destructors of the source and of what it
yielded. The orderings that serve those guarantees stay; the code and tests
that existed only for a panicking subscriber were removed.

The feature was unreleased, so no published contract changed. Executable
evidence: `tests/streaming/sockets/undeclared.rs`,
`tests/streaming/endings.rs` and `tests/streaming/subscribers.rs` in
`batter-axum`.
