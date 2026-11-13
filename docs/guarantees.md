# Guarantees, preconditions, and non-guarantees

These describe the implemented contract and its limits. All workspace packages
declare Rust 1.98.1, the pinned development toolchain, as their minimum.
SQLx 0.9.0 belongs to the optional PostgreSQL adapter and unpublished example
packages, not the foundation or generic test support. Passing tests do not establish guarantees beyond their scope.

The workspace's platform scope is Unix-only. Windows is unsupported and not
planned; no non-Unix process or signal fallback is provided. Linux x86_64 and
macOS arm64 have historical execution evidence on Rust 1.94.0 and 1.98.1, including
focused hosted macOS jobs before `batter-f0jz`; regular hosted macOS CI now uses
the pinned toolchain. Other Unix targets remain unverified. See
[ADR-007](adr/007-unix-platform-scope.md). `batter-outbound-http` postdates that
evidence; local macOS checks supplement its Linux evidence in `batter-8e8r`; see
[its contract](#outbound-provider-exchange).

## Contract sections

- [Native quota execution](#native-quota-execution)
- [Execution boundary](#execution-boundary)
- [Owned work and finalization](#owned-work-and-finalization)
- [Retry contract](#retry-contract)
- [Outbound provider exchange](#outbound-provider-exchange)
- [Admission contract](#admission-contract)
- [Process ownership and shutdown](#process-ownership-and-shutdown)
- [Owned finite commands](#owned-finite-commands)
- [Owned startup contract](#owned-startup-contract)
- [Periodic maintenance components](#periodic-maintenance-components)
- [Maintained Runlimit PostgreSQL storage](#maintained-runlimit-postgresql-storage)
- [Dependency health sampling](#dependency-health-sampling)
- [Cleanup contract](#cleanup-contract)
- [PostgreSQL client disposition](#postgresql-client-disposition)
- [HTTP boundary](#http-boundary)
- [Optional database fixture finish](#optional-database-fixture-finish)
- [Native Runledger handler panic records](#native-runledger-handler-panic-records)
- [Application-installed redacting panic hook](#application-installed-redacting-panic-hook)
- [Explicit settings and redacted diagnostics](#explicit-settings-and-redacted-diagnostics)
- [Offline legacy retirement](#offline-legacy-retirement)
- [Runledger invocation phases](#runledger-invocation-phases)
- [Shared Runledger workspace](#shared-runledger-workspace)
- [Native Runlimit source identity](#native-runlimit-source-identity)
- [Managed-object release ordering](#managed-object-release-ordering)
- [Retained enqueue-intent reconciliation](#retained-enqueue-intent-reconciliation)

## Native quota execution

The PostgreSQL `AttemptRunner` separately owns outcome-aware attempts: reserve,
verify outside a transaction, claim a still-live receipt, decide/write application
state, complete native retry state, and acknowledge commit. The final accepted or
rejected decision is made inside the transaction. Rejected authentication commits
its audit and consecutive-failure increment; infrastructure failure rolls back.
A stale claim never invokes application writes. The native row lock protects a
valid claim through commit even if the original verification lease expires.
One original operation budget covers every phase. `batter_sqlx::run_atomic_profiled_in`
retains the full acknowledged or uncertain outcome before boundary telemetry;
an interruption without acknowledgement cannot establish rollback. Native
attempt algorithms, lease fencing, storage, and migrations remain Runlimit-owned.
The runner performs no refund of ordinary quotas and no automatic replay.

The optional `batter-runlimit` adapter owns one native atomic batch before the
work factory, using the same total operation deadline for both phases. It does
not implement quota storage, policy validation or transactions. Denial/backend
failure prevents work; a returned grant remains retained even if cancellation
prevents work or work later fails. There are no automatic retries or refunds.
An unobserved started check has unknown consumption, not proof of rollback.

Its authenticated HTTP boundary owns lifecycle/deadline admission before auth,
then quota before body extraction. The complete protected router passed to
`prepare`, including custom root, nested and method fallbacks, is guarded.
Without a custom protected fallback, unmatched paths receive the default 404.
Public probes require the named `with_public_probes` opt-in and a route-only
`PublicProbes` builder; it rejects capture and catch-all patterns at
construction, so arbitrary Routers, dynamic routes and custom fallbacks cannot
become public handlers. A public GET probe and protected GET route at the same
path cause a startup panic during Axum route assembly; no service is registered
from that preparation. Declared GET/HEAD probes bypass lifecycle/deadline admission,
authentication and quota, and receive no protected `OperationContext`. It does not treat raw
principal extensions as authority and installs a privately constructible `Authenticated<P>`
handler extractor only after authentication. Applications must extract that
type for authority; raw `Extension<P>` remains unrelated application metadata.
It uses the transport peer, never forwarding headers. The opaque prepared service
installs native peer metadata and exactly one observer across assembled business
routes, public probes and fallback. Retained bounded quota facts survive outer
timeout/drop and are separate from the final status. The record owns one
irreversible writer claim shared by every clone of request metadata; neither
handlers nor a timeout renderer can reclaim it after admission or writer drop.
The admission failure renderer receives ordinary request metadata without the
private writer, shared observation state or operational ownership marker,
including when admission fails before the quota boundary runs. The same filter
protects captured interruption metadata, probe renderers and browser rejection
renderers. Public correlation and application extensions remain available;
redispatch through an operational wrapper owns a separate correlation and
completion and cannot replace the original observation's quota facts.
The non-cloneable writer moves from unstarted to started to consumed terminal
publication. Dropping it before a check leaves `NotChecked`; dropping it during
a check leaves `Unresolved`. A published terminal fact cannot be replaced with a
later timeout/nonterminal fact through this API.
This observation capability is not authentication or quota authority: the
supported adapter still owns recording native truth. Application authorization, proxy trust,
backend initialization, body streaming and detached descendants stay outside
these guarantees; PostgreSQL expiry maintenance is covered by
[maintained Runlimit PostgreSQL storage](#maintained-runlimit-postgresql-storage). Memory atomicity and offline native error/type tests
are not live PostgreSQL or fresh-agent usability evidence.

## Execution boundary

OperationContext uses a Tokio monotonic deadline. Children clamp their deadline
to the parent and inherit cancellation in one direction. Clones of one context
share observation and execution capability, but cannot cancel their scope.
`OperationOwner` retains explicit cancellation authority for an independent root
or derived child; it is not cloneable. Dropping the owner or a context does not
cancel outstanding context clones. `RootDeadline` selects an absolute independent
root deadline and cannot establish a parent relationship. Process admission
returns an owner tied to process cancellation; adapters may explicitly consume
it into a context when they do not need request cancellation authority.
`run` creates a child token and cancels it on return or drop. The original parent
remains reusable until cancelled or expired.

`child_until(Instant)` returns an owned child at the earlier of the requested
instant and the parent deadline. Elapsed instants are valid expired children;
this never creates a fresh independent root or grants cancellation over a parent.

`run_tracked` uses the same execution boundary as `run`, and distinguishes an
opaque `NotStarted` (this call never invoked its factory) from `Interrupted`
after invocation and a concrete returned `Failed` error. It supplies no `From`
conversions that erase these distinctions. Invocation may itself perform work;
started does not mean a request was sent, and `NotStarted` says nothing about
earlier calls, capture destruction or independently running work. It grants no
replay authority. The reference provider dispatch handles this call's
`NotStarted` as known non-dispatch; after-start interruption still requires its
existing reconciliation protocol. Dropping an unpolled or pending run produces
no result evidence. Legacy `run` deliberately reports both phases as interruption.

Preflight checks prevent starting a factory that is already cancelled/expired.
When branches are simultaneously ready, cancellation precedes deadline, which
precedes future completion. A just-completed side effect can therefore have an
interrupted caller outcome. Selection is cooperative: the completion branch does
not reread the clock or cancellation token after work returns. Factory
construction must not block, and a non-yielding poll can exceed any configured
time allowance and still return its result when completion is selected. This is
not a hard wall-clock boundary and does not imply rollback.

Returned errors retain their concrete type. Panics propagate to a task boundary;
ordinary OperationError is not a panic recovery mechanism. A never-polled run
produces no work and no telemetry. Dropping a polled run records `dropped`, not a
fabricated application failure, and signals its child scope.

`run_resolved` is the explicit boundary for an application whose final result can
be retained before cancellable local cleanup completes. Its synchronous resolver
runs exactly once after the completion/interruption choice and before operation
telemetry is finalized, so the returned typed result and recorded outcome share
one classification. It cannot await cleanup, extend the deadline, or run when
the outer future itself is dropped. The application owns the retained evidence
and may reclassify interruption only from that evidence; Batter does not prove a
remote commit, rollback, or other effect.

`operation::low_level::run_resolved_value` uses that same execution boundary when
an adapter must return a value even for interruption. Its resolver returns one
`ResolvedOutcome<T>`: `Succeeded(T)`, `Failed(T)`, or
`Interrupted { reason, value: T }`. Classification and value cannot be split
across an operation result and a separately coordinated optional slot. The
library records the selected outcome before returning the value. There is no
constructible dropped-with-value variant. An unpolled call invokes neither
callback; abandoning a pending call destroys work and resolver captures under
the first-poll dispatch without invoking the resolver. Panics propagate and
cancel the child. The low-level caller still owns classification truth and any
remote-effect claim; the supported adapter owns that translation for its users.

Batter restores the originating tracing dispatcher during polling and destruction
of its observed futures. Completion/drop events and nested span destruction stay
with that subscriber when a runtime abort happens outside the original poll.
Operations and HTTP boundaries capture this context on first poll; this does not
install global state or supervise application tasks spawned outside Batter.
When an operation INFO span is filtered, execution and completion retain the
enabled application parent selected at first poll. Operation fields are recorded
only on the operation span; later polls and destruction cannot adopt an unrelated
ambient parent or overwrite inherited application fields.

Adapter authors can use `batter::telemetry::with_current_dispatch` to preserve
the dispatcher at wrapper construction through inner-future polling and
destruction. It returns a future and adds no task, heap allocation, or `Send` /
`'static` requirement to the wrapped future. Keep observations and nested spans
inside it. It does not capture or enter the current span, drive a dropped future
to completion, or supervise work.

The opt-in `metrics` feature records each polled operation, started retry
attempt, whole retry execution, bulkhead, process or root lifecycle admission
decision, observed task exit, cleanup hook and supervisor shutdown exactly once,
after the result is known and outside admission locks. The canonical setup is
`telemetry::metrics::install(recorder)`: it accepts only a recorder on the
re-exported `metrics` 0.24 facade, so another major version fails to compile,
and it publishes descriptions to that recorder only after installing it, even
inside a local recorder scope. Rejected recorders receive no catalog descriptions
and are returned to the caller. Polled but dropped
operations, retry attempts and executions, admission waits and supervisor
drives record `dropped`, or `panicked` when destroyed during unwinding, through
one shared guard. `Bulkhead::run_blocking` records one bulkhead decision after
its post-admission context check, immediately before handing the closure to
Tokio; its result wait is an internal boundary with tracing only, and an
abandoned wait adds no decision. Retry execution recording follows destruction
of discarded errors and owned callbacks, including jitter samplers. A panic
during that destruction propagates and records `panicked`; values and errors transferred to
the caller are outside the completed execution. A cleanup hook taken by a destroyed close driver records
`abandoned`; hooks of an unclosed stack record `dropped`. Skipped hooks leave
the stack before being counted, so a capture destructor panic cannot count them
again as dropped. Every skip warning is emitted before metrics and captured-value
destruction; a captured destructor panic still propagates and prevents report
return. Other futures dropped before their first poll record nothing. A retry attempt counts only once its
factory is invoked. Foundation-owned waits (admission, backoff) are not counted
as operations, while adapter boundaries such as `http.response_construction`
are. Tracing diagnostics are emitted before a boundary's metric, and
publishing `Stopped` never waits for recorder code: shutdown is recorded after
`Stopped` and destruction of the owned shutdown future, before the report is
returned, so flush after awaiting the driver's completion. A shutdown-future
destructor panic records `panicked` without a completed-duration sample.
Its duration is measured from the final canonical lifecycle
stop instant, including earlier native timestamps learned after drain begins;
abandoned drivers record no duration and record their shutdown after the
supervisor's own queued work and cleanup, and a startup that fails before its
running driver records only its cleanup hooks. Process admission decisions are
recorded when the submitter's decision completes, before an accepted task can
start. Once queued, an accepted task receives its lease even if admission
recording unwinds, so recorder failure cannot strand its active count or create
a missing-lease task panic. The recorder panic still propagates to the submitter.
Root-admitted work is unsupervised and not ordered with shutdown. Without the
metrics feature, shutdown does not read the shared clock for metric recording.
Batter retains at most `MAX_SERIES` pre-built keys, so recording allocates only
on a series' first observation. Labels come
only from closed foundation vocabularies or a fixed-capacity, write-once table
of operation/task names using the component-registration vocabulary; other
names or names beyond capacity record `<invalid>` or `<overflow>` and increment
a separate coalescing counter for each affected observation, without logging.
Retry attempts and their enclosing execution are separate observations.
The vocabulary rejects URLs,
e-mail addresses and error text, but not identifiers embedded in otherwise valid
names; leaked names can occupy slots first-come, never evicted. The catalog
therefore has at most `MAX_SERIES` series regardless of traffic. Batter
owns no metric buffer, queue, flush, retry or database write, so a slow,
unavailable or saturated collector cannot change returned results or admission
decisions. Durations come from monotonic `Duration` values and are finite and
nonnegative. Recorder code runs synchronously, including in destructors; a
blocking, panicking or unboundedly buffering recorder, runtime death and
non-yielding destructors are outside this guarantee, as for tracing subscribers.
Nested recorder panics during unwinding can abort the process.
These metrics are best-effort diagnostics, not authoritative audit records.

`reserve_finalization` divides an existing context into sibling work/finalization
contexts. Work ends at the original deadline minus the positive reserve;
finalization keeps the original deadline. Cancelling/finishing work does not
cancel finalization; parent cancellation cancels both. The caller must await
finalization explicitly while the enclosing owner remains active. The reserve
does not extend total time, shield parent cancellation, or prove rollback.
`split_finalization` produces the same siblings with no reserve: both phases end
at the original deadline, so finalization has time only after work ends or is
cancelled early. Both forms require positive work time when called, and
`reserve_finalization` keeps rejecting a zero reserve.

**Not supplied:** asynchronous descendant joining, preemption, cancellation
shielding, remote cancellation acknowledgement, transaction rollback proof,
commit certainty, or exactly-once effects.

## Owned work and finalization

`operation::PhasedOperation::new` derives work and finalization from one parent
and a positive reserve. `OperationPhases::into_operation` transfers an existing
pair, including invocation-derived and zero-reserve pairs, without resetting its
clock. Both specifications are inert until `start`. The work callback receives
only its shortened context. The finalization callback receives its context and
read-only `PhaseOutcome` only after the work future is destroyed. The driver
retains the original value/error, boundary interruption and destruction panic
before entering finalization; finalization failure cannot replace them. Work
completion cancels its context and descendants before finalization starts, even
on success. The sibling finalization context remains live within its own bounds;
this cancellation does not join arbitrary spawned tasks.

The driver shares command phase polling and panic containment. Cancellation and
deadline are checked before each factory and before polling; a result produced
by a synchronous overrun is retained together with its late interruption. A
phase whose parent is already cancelled or deadline has expired does not invoke
its factory. Work panic, returned error or work-only interruption still proceeds
to finalization when its context permits. Finalization stops at the original
total deadline and inherits parent cancellation. Work cancellation never travels
upward or into its sibling. Both the phase coordinator and result publisher
retain the originating tracing dispatcher and span through execution and
captured-value destruction, including skipped work and unclaimed reports. A
private guarded publication retains that protection after the publisher exits.
Only a completed `wait` claims the shared result and transfers its destruction
to caller context; observer clones and cancelled pending waiters do not claim it.

`RunningPhasedOperation` owns work cancellation. Dropping it, even before first
poll, requests work cancellation but leaves the coordinator to attempt
finalization. Its borrowed waiters and cloneable `PhasedObserver` are read-only;
dropping a waiter changes no execution. `PhaseOutcome::classify` returns an
accepted value only without late interruption or destruction panic; otherwise an
opaque `PhaseFailure` retains all raw evidence, including a returned value.
`wait` returns an opaque `PhasedSuccess` containing both accepted values, or
`PhasedFailure::Report` containing an opaque `FailedPhasedReport` with both
outcomes. Success and failed-report proofs cannot be constructed or interchanged
by consumers. Neither value extraction nor observer cloning requires domain Clone. Default formatting
omits application values and causes. A coordinator join failure is separate. Publication and
finalization require a live runtime; this does not join arbitrary spawned work
or prove remote rollback. Application work placed inside a finalizer, captured
root authority and independently constructed roots remain application policy.

Manual `OperationPhases::{work,finalization}` access remains lower-level: it
requires caller sequencing and does not retain results or drive finalization.

## Retry contract

Each attempt invokes a fresh factory. The initial call is attempt 1 and consumes
the attempt limit. Only returned application errors reach the classifier.
ReplaySafety is an explicit caller assertion, not an enforcement mechanism or
proof about an HTTP method/SQL statement. Never permits only the first call.

One total deadline covers attempts and backoff. Exponential backoff saturates at
the configured cap. RetryAfter is a provider LOWER bound and is combined using
max, never min. When the required sleep consumes all remaining budget, return
InsufficientBudget immediately, retaining the application error; do not falsely
report an already elapsed deadline. The library makes no estimate of how much
execution time the next attempt will require.

`RetryOptions::with_attempt_maximum` opts into a separate cooperative deadline
for each factory. Immediately before every attempt, including after backoff, its
child deadline is `min(input context deadline, attempt start + attempt maximum)`.
The input may already be a shortened `OperationPhases::work()` context; retry
never recovers the parent or its finalization reserve. A later attempt can
therefore start with less total time remaining than the configured maximum.

Expiration of a strictly earlier attempt cap returns the non-exhaustive
`RetryExecutionError::AttemptDeadlineExceeded`, retains the latest earlier
application error and counts only factories actually invoked. It is terminal:
the interrupted attempt is not classified and no replay follows. A cap tied
with the input deadline is reported as total `DeadlineExceeded`. Cancellation
wins over either deadline, and a deadline wins over simultaneously ready work.
The attempt future is dropped and its child context is cancelled; this does not
join detached work or establish a remote effect's outcome.

`Interrupted { reason: Cancelled, .. }` means cancellation was observed in the
input lineage or the current attempt scope. Retry callbacks receive
`Attempt.context` for observation and execution, not cancellation authority.
A controller can stop the sequence through a retained owner of its input context
or an ancestor, which also cancels that owner's other descendants. To stop only
one retry sequence while leaving its caller and siblings active, derive a
dedicated child owner from the caller's context and execute the sequence through
that child's context. Retry reconciles cancellation after the factory returns
and before accepting a value or classifying an error, including a same-poll
cancellation.
An error returned in that poll is retained as `last_error` but is not classified
or replayed; a returned value is discarded. The result does not by itself
identify which scope originated the cancellation.

Attempt-versus-total classification follows absolute deadline ordering. A
blocked thread, non-yielding poll or stalled runtime can delay observation of an
earlier attempt cap until the total deadline has also elapsed. The attempt-cap
result therefore does not promise remaining total budget at return. A
non-yielding poll that returns after a deadline can also have its result accepted
when completion is selected before the timer is observed. Check the input
context before starting fallback or other follow-up work.

Cancellation or expiration of the total context also stops the sequence and is
not automatically replayed. A current interrupted attempt may have no returned
error at all. Panics are not classified. Legacy `execute` retains deterministic
backoff and its original `RetryError`; `execute_with_jitter` retains its original
caller-supplied sample argument. `execute_with_options` is deterministic unless
its options use `with_jitter`; both jitter paths select equal jitter in
`[max(1ns, backoff / 2), backoff]`, then apply the provider lower bound with max.
Options are consumed once and are not clonable. Tests can replay samples;
production callers need a freshly constructed, independently seeded sampler for
each concurrent execution.
There is no global RNG, fleet coordination, retry-token budget, or circuit breaker.

Replay keys, provider deduplication retention, payload matching, transaction
isolation, and ambiguous commit handling belong to the application/upstream
protocol. Choose one retry owner so nested budgets do not multiply attempts.

## Outbound provider exchange

`Origin::public_https` retains a public-address policy on the origin and filters
DNS candidates before the existing bounded dialer; no preflight/re-resolution
gap exists. The exclusions include globally reachable AS112 and AMT
special-purpose blocks; public reachability does not exempt them from the
address policy. Ordinary `Origin::https` still supports private services. See the
[public-destination contract](../crates/batter-outbound-http/README.md#public-destinations).

`batter-outbound-http` (facade `batter::outbound_http`, feature `outbound-http`) is
experimental. [ADR-015](adr/015-outbound-transport-evidence.md) records the
transport decision and [references](references.md#owned-outbound-http1-connections-2026-11-04)
record the inspected upstream APIs. `batter-u387` replaces the original pooled
transport; prior Linux/macOS evidence in `batter-8e8r` predates this change.

`Client::new(origin, connect_timeout)` is the only constructor. A client fixes
one validated origin and shares only its TLS configuration when cloned. Every
exchange opens a fresh TCP connection, performs TLS for HTTPS, and uses hyper's
low-level HTTP/1 connection API. The exchange future drives that connection
itself; it never spawns a connection driver or writer. There is no pool,
connection reuse, resend, redirect, proxy lookup or content decoder. Every call
therefore pays connection and TLS setup. Feature unification with another
client's HTTP/2 or gzip support cannot change this transport.

Hostname setup uses bounded address fallback (`batter-4p4a`). It preserves the
first resolved family preference, starts the other family after at most 300 ms
or half the remaining setup budget, and starts it immediately if the first
family fails. Within each family, remaining time is shared across its remaining
addresses with a one-second minimum per candidate capped by the shared deadline
(`batter-wznf.1`). A stalled address can consume a shorter family budget; not
every answer is guaranteed an attempt. At most two TCP candidates are active, driven in the exchange future;
losing candidates are dropped before TLS or HTTP handoff. The single absolute
setup/exchange deadline bounds all of them. This does not replay an HTTP request.

TLS keeps the explicit *ring* provider, TLS 1.3 and 1.2 only, ALPN `http/1.1`,
and the operating system verifier for HTTPS. Loopback HTTP clients use an empty
root store and never read the host certificate store. On Linux, HTTPS client
construction loads the host store (`SSL_CERT_FILE`/`SSL_CERT_DIR`); an empty
store returns `ClientError::Build`. On macOS, the platform verifier runs during
the handshake. No process-default provider is installed or needed here. Other
crates using default-provider rustls builders still need application-owned
configuration when their graph enables both providers.

`Origin` admits HTTPS or literal-loopback HTTP with a nonzero port, and rejects
credentials, non-root paths, queries, fragments and non-HTTP authorities.
Preparation reports the first problem before connecting: empty/dot segments
(including the part before `;`), tab/CR/LF normalization hazards, an invalid
HTTP URI, reserved framing/hop headers, repeated bodies, JSON serialization
failure or CONNECT. Segments stay percent-encoded inside one path component;
query pairs keep insertion order. Callers provide path components separately,
such as `["v1", "effects"]`. The library builds origin-form request targets,
Host and body length itself. Caller header values and response header values
are sensitive. Preparation supplies `Accept: */*` only when no caller `Accept`
exists; explicit values keep their order and sensitivity. Wire tests check
the default and explicit values. `PreparedRequest` is non-Clone and consumed by either send path.

| Outcome | Evidence |
| --- | --- |
| `NotStarted` | No handoff; entry budget was too short, or the exchange expired/was cancelled before handoff. TCP/TLS setup may have begun. |
| `NotDispatched` | DNS/TCP/TLS/HTTP/1 setup failed or the connect timeout elapsed, before handoff. |
| `Unknown` | Handoff occurred, but no accepted response head is available. The request may have reached the peer. |
| `Responded` | An accepted response head is retained, with `Complete`, `TooLarge` or `Failed` body evidence. |

`Exchange` remains exhaustive and must-use, with twelve stable labels.
`proves_unsent()` holds only for the first two outcomes. Their evidence values
remain opaque, non-Clone and unavailable through public constructors. A
response does not prove provider processing: an intermediary may have sent it.

The exchange deadline is the caller's deadline minus `RESERVE` (50 ms).
Connection setup begins only with at least `MINIMUM_ATTEMPT` (50 ms) left before
that exchange deadline. DNS, TCP, TLS and HTTP/1 setup have one connect timeout
of more than zero and at most 60 seconds, inside the same exchange deadline.

Before handoff, cancellation wins over the exchange timer, which wins over
connection progress. The final clock/cancellation check sits immediately before
`send_request`, with no await between them. An expired or cancelled exchange
that has not handed its request to the connection returns `NotStarted`, even
when TCP or TLS setup already began. A setup error or connect timeout returns
`NotDispatched`. These outcomes follow owned phases; they do not infer dispatch
from reqwest or hyper-util error kinds.

After handoff, an available response or body frame wins over cancellation and
the timer, and cancellation wins over the timer. Each write, vectored write,
flush and shutdown checks the gate before polling the underlying transport.
The TCP stream under TLS is guarded too, because a TLS read can write pending
records. After stopping, the connection can still read an available reply but
cannot advance its writer. A queued or partially written request therefore
cannot resume writing when a stalled exchange is polled after its deadline or
cancellation. No detached connection task can progress while the exchange is
unpolled. Without a response head, interruption after handoff is `Unknown`;
a received status survives every body failure.

With `operation`, await `send_in(context, limit)` directly in the work phase of
`reserve_finalization`, then record the outcome in the finalization phase.
`RESERVE` lets classification precede an outer deadline; it is not a recording
budget. Dropping a send future drops its connection and produces no outcome:
treat a lost exchange as `Unknown`. An outer `run`, `retry`, `timeout` or
`select!` can still discard a ready exchange according to its own precedence.
Sending requires a Tokio runtime with its time driver enabled.

Executable evidence lives in the package's preparation, before-head, body,
operation, wire, redaction, ClientHello and Linux trust-store targets, and the
facade's `outbound_exchange` example tests. The private transport tests use the
production handoff and driver with controlled I/O and a paused Tokio clock:

- `expired_or_cancelled_handoff_never_submits_a_request` checks the final gate.
- `stalled_exchange_has_no_background_writer_and_never_resumes_writing_after_stop`
  checks queued and partially written requests for both deadline and cancellation.
- `deadline_expiring_while_connecting_is_not_started` checks stalled TLS setup.
- The private `transport/dial/tests.rs` covers stalled preferred families,
  same-family address stalls, long lists of healthy delayed candidates, immediate
  family failure, short shared budgets,
  either family winning and losing candidates being dropped before return.
- `tls_error_reads_cannot_flush_buffered_http_after_deadline_or_cancellation`
  uses the production TLS assembly and HTTP driver over controlled raw I/O.
  Real TLS 1.2/1.3 requests are buffered behind a blocked writer, then a bad TLS
  record triggers its read-side error flush after expiry or cancellation. An
  unstopped control proves that buffered HTTP ciphertext would be flushed.
- `a_setup_poll_returning_after_its_connect_timeout_never_hands_off_http`
  advances the paused clock inside one ready setup poll and checks both the
  connect timeout and exchange expiry, with an on-time HTTP dispatch control.
- Ready-head/body tests retain available replies over interruption; cancellation
  still wins when it ties with the timer and no reply is available.
- `owned_tls_verifies_certificates_and_keeps_readable_replies_after_expiry`
  rejects an untrusted self-signed certificate and exercises a private trusting
  control over TLS 1.2/1.3, including a readable body at expiry.
- `clones_open_independent_connections` and
  `a_closed_previous_connection_is_never_reused` check fresh connections.
- The private `send/buffer/tests.rs` and `send/body_storage_tests.rs` observe
  each body allocation from its own destructor, after the wipe and before
  deallocation, through a `cfg(test)` probe that reads no freed memory. They
  cover replaced allocations, `Bytes` clones and slices released in every order
  and on another thread, streamed size-limit failure, truncated
  length and chunked bodies, deadline, cancellation, a dropped exchange future
  and the initial allocation, with status and single-request checks on the real
  hyper path. Each protection was temporarily removed and the matching tests
  failed; see [references](references.md#outbound-response-body-storage-2026-11-09).

The facade feature runner checks type identity and `send_in`, rejects imports
without `outbound-http`, and checks that both the normal leaf graph and the
shipped facade example exclude reqwest and hyper-util's legacy client.
Dev-only reqwest compiles gzip/HTTP2 as a feature-unification control. The
standalone no-default-features tests use no batter-core.

- `ResponseLimit` accepts 1 byte to 64 MiB. A declared or streamed length above
  it returns `TooLarge`; chunked framing is removed and content codings remain
  encoded. Preallocation is capped at 1 MiB and buffer growth stays within the
  limit. Growth copies into a new allocation before releasing the old one, so two
  allocations briefly coexist, together below twice the limit. A completed body
  keeps the allocation it grew into, at most twice its length and never above the
  limit, until its last handle is released. A body delimited only by connection
  close cannot reveal truncation.
- Response-body storage the package allocates is zeroed before release: after
  `TooLarge` and every `Body::Failed`, when the exchange future is dropped while
  reading, when growth replaces an allocation, and for
  `Body::Complete` when the last `Bytes` clone or slice is released. The buffer
  is a fixed boxed slice, so no `reserve` or shrink can release populated memory
  unwiped; the returned `Bytes` comes from `Bytes::from_owner` on that same
  owner. hyper, rustls and kernel buffers, response headers, request bodies and
  copies made by the caller are outside the guarantee, including `to_vec`,
  `Vec::from(bytes)` and `BytesMut::from(bytes)`. A returned body is never
  uniquely owned, so `try_into_mut` returns `Err` and those conversions always
  copy. A destructor performs the wipe: a leaked or forgotten handle, an abort
  and process exit skip it, and live bytes can reach swap, hibernation images
  or core dumps. The wipe is proportional to the allocation and does not yield
  (about 18 ms for 64 MiB in one release-build measurement).
- Deadlines are cooperative. Scheduling delay, synchronous certificate
  verification on macOS or growing a large body's buffer can delay return beyond
  `RESERVE`. One growth step copies and wipes the old allocation without
  yielding. Separate release-build measurements of copying and wiping 64 MiB
  give an estimated combined cost of about 56 ms (38 ms + 18 ms). A gate cannot
  interrupt an already-entered poll or eliminate thread preemption between a
  clock check and a socket operation. The handoff and write checks have no
  intervening await. Bytes already accepted by the kernel can arrive later,
  including through TCP retransmission. `Unknown` remains necessary.
- A request that fails after handoff remains conservatively `Unknown`, even if
  no byte was actually written. Only owned pre-handoff phases prove unsent.
- No formatted value from this package includes URLs, queries, header values or
  bodies. Response accessors retain the actual headers and body; header values
  are sensitive. A query still reaches the provider and intermediaries.
- The package adds no instrumentation. `label()` provides twelve fixed labels.
  rustls-platform-verifier can log certificate failures and names; rustls can
  log names and certificates when another dependency enables `logging`.
  Application logger configuration owns filtering. The outbound transport no
  longer invokes reqwest or hyper-util's legacy client, so their destination
  and pool logs are absent from this path.
- Provider semantics, replay authorization, idempotency keys, Retry-After,
  dispatch fences and reconciliation remain application-owned. HTTP/2, proxies,
  signing, token acquisition, certificate pinning and operator-supplied
  destinations remain unsupported.

Unexecuted cases include DNS failure, a TCP setup timeout, production platform
verification against a public CA and fresh-agent adoption tasks. New macOS and
hosted execution evidence are not claimed. Actual verification outcomes belong
to `batter-u387`, and those for response-body storage to `batter-53nf`.

## Admission contract

`BulkheadCapacity::new` rejects zero and counts above Tokio's semaphore limit.
`Bulkhead::new` accepts only that validated witness and is then infallible;
there is no raw-capacity constructor. Bulkhead is a process-local semaphore.
Reject fails immediately without a free permit. Wait uses the caller's total
deadline. Permits are private to library-owned execution; `Bulkhead::enter`
was removed. Closing admission wakes waiters but does not revoke admitted work.

`Bulkhead::run(context, admission, operation, factory)` is the canonical path
for async work. Admission and execution share the caller's absolute deadline.
The library checks the context after acquiring capacity and runs the factory
inside an operation boundary. It retains the permit until the entire work
future has been destroyed, including cancellation, returned errors and unwinding.
There is no permit handoff, spawned task or separately callable release step.
A never-polled call invokes nothing. `BulkheadError` distinguishes admission
refusal from operation failure/interruption and redacts retained errors.
Returned streams and detached work are outside the owned future's lifetime;
this does not join them or prove remote effects. A nested durable protocol still
uses `Bulkhead::run_resolved` to resolve retained evidence inside the admission
boundary before telemetry; an outer deadline is not commit evidence.

Both async entrypoints capture the tracing dispatcher on first poll. While
admission is pending, destruction of uncalled factory and resolver captures,
including nested spans, stays under that dispatcher even if another subscriber
is current when the waiter is dropped. Neither callback runs on that drop.

`Bulkhead::run_blocking(context, admission, work)` is the canonical path for
synchronous CPU work or blocking I/O under a bulkhead. It checks the context
again after a permit is granted and records
one bulkhead decision after that check. A refusal returns
`BlockingError::NotAdmitted` and destroys `work` uncalled; the closure is never
handed to Tokio. Otherwise the permit moves, in the same poll and with no
intervening await, into the closure handed to Tokio's blocking pool. It is
released only after the closure returns or unwinds and its captures are
destroyed, or when Tokio destroys the closure unstarted, and always before the
result is published. The context bounds only the result wait, with
`OperationContext::run`'s precedence, so a result that becomes ready in the
same poll as an interruption is discarded. Cancellation or the deadline returns
`Abandoned`, and dropping the caller's future abandons the result in the same
way: the closure may be queued, running or finished, and it keeps its permit
until it returns or unwinds. Batter never aborts it, and Tokio cannot stop a
started closure. A closure panic is caught inside the blocking closure, and
`Panicked` retains the original payload behind redacted `Debug` and `Display`;
the process panic hook still runs on the blocking thread. `NotStarted` means
Tokio destroyed the admitted closure unstarted while its runtime was shutting
down. Tokio does not guarantee that outcome: a busy
blocking thread keeps running queued closures after shutdown begins. A caller
can see it only while the runtime's blocking pool is shutting down and its time
driver has not yet shut down, which depends on the runtime flavor. A
multi-thread runtime, the `#[tokio::main]` default, shuts its pool down while
workers still poll its own tasks, on a plain drop as well as
`Runtime::shutdown_timeout` or `shutdown_background`, so those tasks, and calls
polled elsewhere, can receive it. A current-thread runtime's own tasks never
do; only a call that is not one of its tasks, such as one polled on another
thread under `Handle::enter`, can, during `shutdown_timeout` or
`shutdown_background`. Once the time driver has shut down, a poll panics in the
deadline timer instead, unless it first observes cancellation, or a deadline
that had already passed when the context was checked; that returns
`NotAdmitted` before handoff and `Abandoned` after it. Closing
the bulkhead refuses new runs and wakes waiting ones; running closures keep
their permits and deliver their results. Capacity counts queued and running
closures, not threads, and Tokio's blocking pool is shared with `tokio::fs`,
DNS resolution and other `spawn_blocking` callers. `work` runs, and an unstarted
closure is destroyed, under the dispatcher captured at first poll and inside a
`batter.blocking_work` span whose parent is the caller's span at handoff; that
span is also closed under the captured dispatcher, which releases the caller's
span there. The runtime and its time driver are checked at first poll: a first
poll outside a Tokio runtime, or on one without its time driver, panics before
admission, and later polls build and poll the result wait's timer in the
runtime checked then, so a call finished from another context cannot panic for
lack of a timer after handoff. Both deadline timers therefore belong to the
first-poll runtime, as any Tokio timer belongs to the runtime it was built in:
the deadline fires only while that runtime's time driver runs (a current-thread
runtime drives it only inside `block_on`), while cancellation is unaffected. A
poll after that runtime has shut down panics in its deadline timer, whether
the call waits for admission or for the result; after handoff the closure still
keeps its permit until it returns. An abandoned closure's value or panic
payload, including a capture's panic while Tokio destroys an unstarted
closure, is destroyed unobserved under the dispatcher captured at first poll,
whether the closure finishes before or after the wait ended. It is destroyed
after Batter has left the `batter.blocking_work` span and released its handle;
a span it holds keeps that span and the caller's open until then, and they
close there under the same dispatcher. A value or payload delivered to the
caller is the caller's to drop.

This bounds permit holders, **not** waiting callers, request bodies, socket
buffers, database connections in other components, or global fleet concurrency.
Context preflight cannot atomically serialize a concurrent cancellation with
semaphore acquisition. Tokio may grant capacity after the deadline before its
timer fires; every public execution path rechecks the context after acquisition.
Rate limiting, concurrency limiting, and retry accounting are separate policies.

## Process ownership and shutdown

`run_until` returns a `#[must_use] ShutdownReport`. Owned-driver
`wait_checked` and `shutdown_checked`, and observer `wait_checked`, return
`Result<ShutdownSuccess, ShutdownFailure>`. Success retains the immutable shared
report for diagnostics or application policy. Failure retains the unsuccessful
report or the coordinator's original `JoinError`; ordinary `?` propagates it.
`ShutdownFailure` itself redacts direct `Debug` and `Display`, but its native
`source()` is retained. `anyhow::Error` chain formatting can print a coordinator
panic payload through that source; use application-selected summaries at a
public exit or logging boundary.
`wait_checked` does not request shutdown, approve readiness, or extend process
ownership. Among the checked methods, `shutdown_checked` requests shutdown.
Legacy `shutdown`, raw `shutdown_report`, and dropping the last owner also request
shutdown. Cancelling a waiter leaves cleanup owned by the coordinator while its
Tokio runtime remains alive.

Legacy `wait`/`shutdown` signatures remain for source compatibility. Explicit
`wait_report`/`shutdown_report` return the same raw
`Result<SharedShutdownReport, Arc<JoinError>>` for deliberate classification.
The shared wrapper is `#[must_use]`, so discarding it after `?` or `unwrap()`
warns; coordinator completion alone does not establish successful shutdown.
Cloning the shared wrapper retains the same report and errors; dereferencing
borrows `ShutdownReport`.
The wrapper displays `owned shutdown report`; its `Error::source()` exposes the
concrete report and its summary. Chain-walking diagnostics therefore show the
task/cleanup summary once, while allowing a concrete report downcast.
`ShutdownReport`, `TaskRecord`, `CleanupReport` and `CleanupRecord` implement
redacted Debug and Display, like the command and startup reports: formatting
shows names, outcomes and whether an error was retained, never error contents.
Retained errors are reachable only through the public fields, so disclosure is
an explicit read. Propagating a report out of a `main` returning `Result`
therefore prints a redacted summary; applications still select their exit
output, as shown by the complete `ExitCode` example on `SharedShutdownReport`
and the SQLx executable. Neither the foundation nor a report wrapper redacts an
arbitrary domain error that an application formats itself. Direct
`ShutdownFailure` Debug/Display is redacted, but arbitrary error-source
traversal has no such promise, and Rust's default panic hook prints payloads
unless the application root installs the
[redacting panic hook](#application-installed-redacting-panic-hook). Raw-report lints
are advisory: binding, explicit dropping, or allowing the lint bypasses them.
Even a checked `Result` can be deliberately discarded by application code.

A finite receipt's
`ProcessTaskError::Failed` shares its `Arc<E>` with the report while exposing
the concrete E through `Error::source()`, so downcasts preserve domain identity.

Every registered component is long-lived and critical. Registration itself does
not start work. All direct JoinSet completions are observed by name, including
factory panics and early successful exits. Errors continue to be collected during
shutdown. Completion captures whether drain had started; a later drain cannot
reclassify an earlier successful critical exit as expected. There is no restart.
Application approval is a one-shot ownership transition from
`UnapprovedSupervisor` to `RunningSupervisor`: Ready requires that transition, a
running driver, and every registered critical component consuming its one
`ComponentStartup` acknowledgement after actual initialization. `ShutdownSignal`
has observation authority only, while both startup capabilities are
non-cloneable and consumed by their transitions; unauthorized and repeated
acknowledgements therefore do not compile. For direct `register`, acknowledgement
is an application assertion, not an inspection of its internal descendants.
Forgotten application or component acknowledgement leaves Starting.

`ShutdownHandle` is retained at the composition root for shutdown requests. It
does not expose application approval, readiness reads, waits, or raw operation
cancellation tokens.
`LifecycleStatus` can only read or wait for lifecycle state;
`OperationAdmission` can only create an `OperationOwner` after observing Ready;
`ShutdownSignal` can only observe drain and forced cancellation. Axum request
and readiness policies store those narrow projections, so they cannot request
shutdown or approve readiness. A request racing drain may enter only if its
admission read observed Ready; initial drain does not cancel its context, while
forced process cancellation travels downward. Managed supervision receives
private coordinator authority from the same registered lifecycle bundle as its
component-start capability and does not recover mutation through
`ShutdownSignal`.
There is deliberately no public raw process-cancellation token: new transient
work cannot be represented after admission closes. A component already in its
drain phase observes `ShutdownSignal::cancelled()` directly, while cleanup uses
its independent budget and, when useful, an independent bounded context.

The [component ownership comparisons](../crates/batter-core/tests/component_ownership.rs)
gate actual child initialization before acknowledgement, then gate child stopping
and join before dependent cleanup. A deliberately nonconforming wrapper returns
Ok during drain without joining its child: the direct report succeeds and cleanup
runs, yet the independently retained child answers a fresh request afterward.
The library cannot inspect hidden children or automatically skip cleanup because
they exist. Joining only the wrapper does not establish transitive termination.
Separate comparisons retain early-success failure, concrete task and cleanup
errors together, and native panic/abort JoinErrors with skipped finalizers.

`register_managed` is the adapter-facing path for a native runtime with its own
descendants. Name validation and the absolute startup context precede factory
invocation. An Err factory result asserts that validation rejected construction
before spawning native work; adapter implementations must enforce this. The
factory transfers initialization observation, native stop observation/control and
the complete native settlement driver together. Batter acknowledges initialization,
observes early native stop to drain peers, and retains the settlement owner outside
the direct waiter. A successful initialization poll is acknowledged only after
its future is destroyed and the startup context is rechecked; a returned error
remains intact if cancellation also occurs. Application readiness approval and dependency health remain
separate. No durable startup job is required by this protocol.
The private initializer owns the sole readiness acknowledgement and consumes it
only on live, successfully destroyed initialization. Drain or an early native
exit retains the interrupted initializer until the native stop callback has run.
Each guarded observer records pending versus observed completion: after returning
or panicking it cannot poll its inner future again, but retains that future for
separately caught destruction. Settlement is recorded before its future is destroyed.

Before initialization, observed native stopping or settlement records
`ManagedInitialization::Stopped`; process drain records `Draining`. Native
termination still drains peers and retains its settlement and failures separately.

The supported native adapter accepts `PreparedSupervisor` from native
`SupervisorBuilder::prepare`, rather than arbitrary caller factories. Preparation
validates configuration and owns cloned handles without starting tasks. A live
supervisor cannot cross this registration boundary; rejection and unstarted drop
do not launch native work. Preparation errors propagate through owned application
startup and its registered cleanup. Adapter tests include compile-fail controls
for live supervisors and closures, plus runtime task-count observations.

`ShutdownReport.managed` freezes each component's observed initialization,
original typed native report, additional failures and pending settlement at the
end of bounded observation. Wrapper abortion cannot discard the only native driver.
Any pending settlement, native refusal of cooperative dependency release, or
panic prevents finalizer execution. A component observer can retain a late report;
it never changes the frozen process outcome or runs skipped finalizers afterward.
Original initialization errors, polling/destruction panics and later native reports
coexist. Every caught stop-callback panic is published before waiting for native
settlement, including callbacks for later clock tightening. A pending native
report cannot hide an already-observed callback failure. Default managed diagnostics omit their contents. The adapter's native
ownership/classification contract remains necessary; Batter cannot discover
arbitrary hidden tasks. [Managed contracts](../crates/batter-core/tests/managed_components.rs)
exercise actual Tokio descendants and controlled failures. A panicking third-party stop
callback can leave retained native settlement pending; the bounded process report
retains the failure and skips cleanup, without fabricating native termination.

Drain, cancellation and abort/reap use consecutive absolute boundaries anchored
at the first recorded process stop, including requests before driver polling.
Repeated requests and delayed phase observation cannot restart the allowance.
The native managed budget uses the parent's drain interval for native graceful
shutdown and its cancellation interval for native abort/join; parent abort/reap
remains available for final observation. The idempotent stop callback receives
the earliest known parent timestamp and returns the earliest native/enclosing
timestamp. An earlier native stop tightens the process clock; Batter propagates
later discoveries to components already settling. Active drain/cancel/reap waits
wake on tightening. The native first failure cause stays separate and is never
overwritten by a clock update. Concurrent components share these intervals rather than each
adding new process time. Cleanup uses its own budget after settlement. This does
not preempt a non-yielding task or make runtime death recoverable.

Readiness only moves forward: Starting may become Ready or Draining, Ready may
become Draining, and coordinator completion publishes Stopped. Stopped cannot
be reverted by a concurrent shutdown request or late startup acknowledgement.
The private lifecycle state module serializes all transitions with admission;
its atomic readiness snapshot is published under the same mutex guard. Reads
remain available while admission is held. Explicit readiness/drain/cancellation
wakeups happen after releasing the guard; native queue enqueue can wake its
receiver while retaining admission.

`ProcessCapacity::new` validates the nonzero Tokio task bound once.
`with_process_capacity` consumes that witness and infallibly adds finite
process-owned admission. `try_spawn` rejects synchronously before
startup/readiness, after root drain, or at capacity; it creates no queue of permit
waiters and never invokes rejected factories.

Admission errors are classified after name validation: permanent closure
(`Closed`) takes precedence over startup (`NotRunning` / `NotReady`) and
capacity (`Full`). Root drain, forced cancellation, task failure, an expired
ancestor, or a closed coordinator queue permanently closes the affected
admission path. This includes shutdown before startup, dropping/aborting an
unpolled driver, and dropping an unstarted supervisor. A fresh supervisor still
returns `NotRunning`.

Queued plus executing tasks hold capacity until termination, not until their
receipt is dropped. Payload sizes and detached work are not bounded by the task
count. Active `ProcessScope` descendants may enter during drain and use the same bound;
escaped inactive scopes and forced cancellation cannot admit them. Parent/child
capacity is shared: awaiting a child at full capacity requires handling rejection,
not creating an unbounded waiter. Finite work remains in-memory, not durable.
Its callback carries the submitting span/subscriber, while process supervision
and dependency cleanup retain the driver's diagnostic context.
Critical components, finite tasks and cleanup hooks retain the available
application parent when filtering disables their Batter INFO span. The context
is selected before spawning (before enqueueing for finite work) and covers
polling and destruction, including abortion. Parent lookup for finite work
happens outside the admission lock. Subscriber and per-layer filters still own
which spans and events reach a sink.
A synchronous subscriber regression checks mutex availability during span
creation, current-parent lookup and parent cloning. It exercises real finite
submission without a competing coordinator, so a callback moved under the lock
fails immediately instead of relying on an async timeout to detect a deadlock.

A finite task future returns `Result<T, Fatal<E>>`; only `Err(Fatal(error))`
initiates process drain, and `?` on a plain application error does not compile
inside the task. Expected business denial should
be `Ok(Err(denial))`, not a fatal task error. Original failure E is shared with the
typed receipt and retained report. Successful finite work is counted, not retained
as an ever-growing history. Failures close admission and are retained for the
bounded outstanding set. Panics/aborts produce a terminated receipt and an
observed task failure in the report; they do not fabricate a typed E.

The initial task-triggered shutdown cause distinguishes ownership:
`ShutdownCause::ComponentExit(name)` identifies a registered critical component;
`ShutdownCause::FiniteTaskExit(name)` identifies an admitted finite task that
returned an error or panicked. Finite labels may repeat across invocations;
the cause carries a label, not a unique invocation ID. Successful finite work
does not initiate shutdown. The coordinator selects the cause when it begins
shutdown. A ready explicit request takes priority over task results in the same
poll, including failures that completed before the request but remain unobserved.
For task-triggered shutdown, the cause reflects observation order rather than a
guarantee about chronological failure order. Later observations do not replace
the selected cause, including after an explicit `Requested` shutdown.
Shutdown aborts occur after the cause is selected; inspect `abort_requested` and
`TaskRecord::outcome` for requested and observed aborts, and task/cleanup outcomes
for all subsequent failures.

On component failure, the caller's shutdown future remains owned by the driver
through dependency cleanup. Splitting shutdown coordination into private helpers
does not end that future's lifetime early.

Drain notifies components to stop admission/claiming without immediately
cancelling admitted request contexts. Forced cancellation follows the drain
allowance. Abort is requested only after a further cooperative allowance.
An additional bounded phase attempts to observe direct task termination.
Ready results are harvested before escalation and final reporting. Being present
in a JoinSet does not alone make a task an abort target: already-finished tasks
retain their actual outcomes and do not suppress cleanup as requested aborts.

`ShutdownSignal::abort_deadline` and `RunningComponent::abort_deadline` report
when that abort would be requested: the recorded stop time plus the drain and
cancel allowances. It is `None` before drain and for a standalone handle, which
no budget governs. A component that owns descendants can use the remaining
interval to end them itself and return joined, as the Axum serving adapter does
for connections. It is read-only observation of the phase clock: an earlier
native stop time moves it earlier and nothing moves it later, a task set that
finishes sooner is never aborted, scheduling delay can pass it, and an abandoned
driver cancels without phases. `ShutdownSignal::abort_deadline_before` waits,
read-only, until the deadline is earlier than one the caller holds, so work
scheduled against it can follow a tightened clock; the Axum serving adapter
reschedules its connection close that way. State unit tests cover the unset,
standalone, extended and tightened cases and that wait.

`lifecycle::low_level::admit_root_with_continuation` is a lower-level adapter
seam, placed apart from `OperationAdmission::admit_root`, the protected path. It
admits exactly as `admit_root` does and also returns an
`lifecycle::low_level::OperationContinuation`, proof that the readiness
decision was made. The proof can start one root that inherits only process
forced cancellation, with its own deadline and without reading readiness again,
which is what lets admitted work continue through drain. It has no public
constructor, is not `Clone`, and `start` consumes it, so one admission
continues into at most one root; a compile-fail doctest rejects a second start
and a copy. A start after the admitted operation was cancelled or passed its
deadline returns an already-cancelled owner; a unit test covers a live, a
cancelled, an expired and a forced-cancelled start. Its holder owns the
obligations the type cannot express: an admitted owner abandoned without
cancellation stays live until its deadline, so when the continued work starts,
and that it ends, remain the caller's. The Axum adapter keeps it private to one
admitted request and starts at most one body from it, only while that
request's response is being constructed.

Total configured allowance is drain + cancel + abort observation + cleanup work
+ cleanup final abort observation. Scheduling delays, synchronous blocking,
non-yielding task bodies/destructors, and OS suspension can exceed it. This is
not a hard wall-clock SLA. `Bulkhead::run_blocking` bounds blocking closures
for a waiting caller but is not process-owned work: drain, dependent cleanup
and shutdown reports neither wait for nor report an abandoned closure, which
may keep using a resource that cleanup closes. Tokio cannot preempt it.
Runtime destruction waits for started closures, including queued ones that a
busy blocking thread starts after shutdown begins, unless the application root
uses `shutdown_timeout`, which leaves them running. Nonpreemptible work that
must settle before cleanup still requires its own owner and shutdown strategy.

The [subprocess tests](testing.md#non-yielding-subprocess-tests) exercise these
limits with a direct task whose poll never returns. When another worker can
drive shutdown, the report retains its name in `abort_requested` and `unjoined`,
contains no joined task outcome for it, and skips dependent finalizers without
invoking them. `Readiness::Stopped` means the coordinator finished; the direct
task can still be live and native Tokio runtime destruction can still block.
On a blocked current-thread runtime, even a previously polled timer and the
shutdown report cannot progress. An external process kill supplies containment
for these tests. The fixture also exits on parent-pipe closure or its independent
emergency deadline, without releasing the blocked task or running its Rust
destructors. These are private test-process controls, with no claim of library
preemption or application finalization after process termination.
The blocked-runtime test watchdog bounds startup separately from observation:
five seconds from spawn to capture a complete drain record, then the fixture's two-second
observation plus one second of margin. Startup cannot consume that observation
window; missing startup evidence fails and triggers kill/reap. The combined
eight-second allowance precedes the ten-second emergency exit, subject to OS
scheduling/process-control limits. Watchdog evidence requires the selected
deadline to have elapsed when the first kill is requested, including when it
extends past five seconds. A live regression pins the selected wait deadline to
captured drain time plus observation for both blocked scenarios. Elapsed time
after reaping/capture is diagnostic only.
The delayed-panic control derives its synchronization bound from the scenario's
maximum wait, including observation after the latest accepted startup. It does
not impose a second, shorter startup deadline. Live wiring controls also reject
kill requests delayed by a full observation allowance past the selected deadline;
this is an executed scheduling check, not an OS latency guarantee.
Startup and final validation share one exact, complete-line protocol parser.
Capture timestamps and deadline decisions use the same mutex: an on-time record
survives late polling, while a late record fails and polling cannot reset the
observation allowance. Later panic diagnostics cannot erase an on-time startup
record: that record establishes timing, not a clean outcome. Final validation
still rejects every captured child panic. A known panic without a startup record
fails promptly, and capture overflow always invalidates startup evidence.
Capture retains 64 KiB of raw diagnostics and metadata
for at most 64 distinct events. Either overflow, or an unfinished protocol
record at final validation, prevents a successful evidence claim. These controls
measure parent-side capture time, not the child's unobservable exact write time.
The capture reader retries interrupted reads. An I/O error or reader panic makes
its explicit finish fail after joining the reader; partial bytes are diagnostics,
not proof of complete capture.
An event wait that observes child exit joins the reader before judging final
startup evidence. Byte and event limits report distinct causes; the first
detected cause survives later overflow. Live controls require natural exit after
flooding output beyond each limit, proving that overflow does not stop draining.
The Linux parent-death probe must reject incorrect termination and forbidden
cleanup evidence even when Python optimization is enabled; its checks do not
depend on Python's removable `assert` statements.
The fixture-owner unwind control must finish within its five-second startup
budget, before the child's ten-second emergency exit; waiting for that fallback
does not establish prompt kill/reap behavior.

A direct task join says nothing about descendants it detached. Component authors
must preserve ownership themselves. After any requested abort, observed panic,
or unjoined direct task, dependent finalizers are conservatively skipped.
The report is unsuccessful even if the immediate wrapper was joined afterward.

The [seeded scheduling suite](testing.md#seeded-scheduling-exploration) exercises
these contracts with bounded two/four-worker workloads, controlled action orders
and independent result accounting. A replay seed reconstructs test choices, not
Tokio or OS scheduling. Delayed-coordinator regression cases establish completion
before escalation; eventual success by itself does not make a racing abort false.
Cooperative work can legitimately miss a live shutdown deadline. The scheduling
oracle reconciles either its successful completion or its observed abort with the
report and cleanup decision, including skipped cleanup after an abort request
that races with successful completion. Paused-clock regressions preserve exact
cooperative-success and late-completion assertions without an OS latency premise.
Descendant closure cases keep post-closure admission rejection strict while
reconciling each admitted receipt with completed or named aborted work. They
preserve the original closing-task error and verify actual finalizer execution
versus explicit skipping. Controlled force- and task-failure cases exercise both
prompt completion and observation delayed beyond the cancellation allowance.
Process watchdogs contain hung tests and preserve bounded diagnostics, without
establishing application finalization. Scheduling child launch requires explicit
arguments and a PID-bound record; ambient flags leave ordinary discovery inert.
The test process owner distinguishes direct-child exit from pipe EOF, preserves
partial evidence on failed observation, and reports escaped pipe owners as
incomplete output rather than successful cleanup. Its 140-second maximum watchdog
and five-second cleanup observation precede the 149-second emergency backstop.
The test owner retains the direct child, alive or unreaped, until pipe EOF or the
group-termination decision; a reaped child never authorizes a later group signal.
Partial capture construction closes its selector before propagating an exception,
including interruption; descriptor release does not depend on garbage collection. The private process
owner requires the main thread, default SIGCHLD and a standard SIGINT disposition
before launch; custom or unknown SIGINT handlers are rejected. An inherited
ignored SIGINT stays ignored in the owner and its children. For Python-default or
Unix-default SIGINT, it temporarily records a stop request across acquisition,
observation and cleanup, then restores the exact prior disposition after resource
release. Repeated signals do not reset cleanup deadlines, and interruption remains failed evidence.
Descriptor closure runs even when settlement exits exceptionally. This does not
shield unrelated raising signal handlers, process death or arbitrary Python code.
The outside-group pipe-writer control retains its own direct-child handle through
bounded cleanup, including exceptional observation paths. The finite non-yielding
fixture checks that unjoined work can retain a pending receipt after the report
is published. Its watchdog allows startup, the entire bounded case, and hang
observation before termination; a delayed-start regression requires the same
report evidence. Both unjoined fixture tests independently require the twelve-second
watchdog observation and termination/reaping before twenty-one seconds, including
cleanup and interpreter/startup margins. These elapsed checks do not establish an
OS scheduling guarantee.
Passing this corpus is not exhaustive proof or a stronger runtime-death guarantee.

Local verification overlaps the core and workspace test configurations and runs
the Python scheduling controls in isolated processes. Both configurations and
every discovered control remain required; sharding changes neither assertions
nor fixture deadlines. The parent rejects incomplete/duplicate shard completion,
failed or missing child outcomes, output overflow and interruption. Matrix logs
retain bounded initial output and separate stdout/stderr tails, marking omitted
bytes explicitly; retained final diagnostics do not make overflow valid evidence. Its bounded
process-group settlement retains leader identity until output EOF or termination,
and never signals after reaping. This is test-runner containment, not a new
application guarantee for detached descendants. See [testing](testing.md#verification)
for execution bounds and the final-evidence reuse conditions.

`start` explicitly approves application readiness before it launches an owned
coordinator and completion monitor, so no component can acknowledge and observe
an unapproved ordinary path. It creates their completion channel at that boundary
and returns the cloneable `RunningSupervisor`. The exceptional
`start_unapproved` path instead returns a non-cloneable `UnapprovedSupervisor`;
consume `approve_readiness` to obtain the ordinary owner. A `SupervisorObserver`
is available from either owner immediately after its start boundary, even before
the coordinator's first poll.
`ShutdownHandle` provides root shutdown control and constructs status, admission,
and signal projections, not completion observation. Migrate former
`handle.observer()` calls to `running.observer()` after starting the supervisor.
`RunningSupervisor::wait`/`shutdown` and `SupervisorObserver::wait` may be cancelled
without cancelling the driver or finalizers. Last-owner drop requests graceful
shutdown; observers and admission/control handles do not prolong ownership.
Observers share the retained report or coordinator JoinError. The Tokio runtime
must remain alive; process/runtime termination cannot be shielded.
If runtime shutdown drops the completion monitor before publication, awaiting
the observer on another runtime panics rather than returning a fabricated report
or coordinator error. The same applies to `RunningSupervisor::wait`/`shutdown`.
An outcome published before runtime shutdown remains readable. Tests cover both
the unpublished-monitor failure and a late observer retaining a published report;
they do not establish cleanup or task termination after arbitrary runtime loss.
Command, startup and process completion observers use the same private
snapshot-before-wait mechanism. Their public outcome types and runtime-loss
diagnostics remain separate. `ManagedObserver` is intentionally excluded: native
settlement publication starts with a meaningful pending snapshot, and sender loss
returns that last evidence rather than claiming completion or panicking.

An unstarted `Supervisor` owns abandonment signaling from construction. Dropping
it withdraws readiness, signals drain and forced cancellation, and wakes
`LifecycleStatus::wait_ready` with `Err(Readiness::Draining)` before dropping
captured values.
It invokes no component or finalizer factory and publishes no completion report;
Stopped still means coordinator completion. Extracted cleanup must be explicitly
awaited. The owner signals forced cancellation before the extracted stack runs.
Finalizers must perform teardown independently of process operation cancellation,
using the stack's cleanup budget and, if needed, a fresh `OperationOwner::new`
rather than a context created by `OperationAdmission`. This also applies to normal
shutdown, which cancels admitted process operations before closing resources. Unstarted
supervisors, standalone shutdown handles and caller-owned `run_until` drivers
cannot construct completion observers; await `run_until` directly for its report.

The lower-level `run_until` remains caller-owned. Constructing its future transfers
the abandonment guard without starting factories or publishing readiness. Keeping
that future unpolled leaves startup pending. Dropping it, even before its first
poll, withdraws readiness, wakes readiness waiters, and signals drain and forced
cancellation. After startup,
JoinSet also requests abortion of owned tasks, but Drop cannot await them or run
asynchronous finalizers. Never put an outer timeout
around run_until and then describe the result as completed graceful shutdown.
Drive its own phased protocol and inspect the report. A process watchdog may
terminate an unresponsive binary, but no cleanup guarantee survives that action.
Ordinary `run_until` consumes application approval on its first poll. The
explicit `run_until_unapproved` returns a linear `UnapprovedDriver` that retains
the approval while it drives a Starting lifecycle. Its inner driver is pinned
independently, so policy can poll that state and later consume the still-combined
outer value to approve without exposing a detached approval capability.

## Owned finite commands

`Command::new` and `Command::within` are inert. `start_with_unix_signals`,
`start_without_signals` and `start` each retain the callback, its work context and
registered LIFO cleanup in one coordinator, independently of waiters. Command
cancellation stays below the supplied parent context. Returning an error with
`?`, factory/poll unwinding, cancelling work or a command-owned signal still
reaches registered finalization. Original work results actually returned by a
poll, late boundary interruptions, future-destruction panics and cleanup
outcomes coexist in `CommandReport`. A failed cleanup coordinator cannot erase
the work result. `CommandOwner::finish` and `check_command` interpret the same
complete contract, `CommandReport::is_success`; a discarded owner and an
implicitly discarded shared report both warn. Formatting excludes value/cause
contents.

`start_with_unix_signals` and `start_without_signals` return the sole,
non-cloneable `CommandOwner`, which has no observer or borrowed waiter. Its
consuming `finish` awaits the coordinator's `JoinHandle` directly and returns
`CommandSuccess<T>` exactly when `is_success` would hold, moving the work value
out. Its fields are private, so success evidence cannot be constructed by
callers, and its formatting omits the value. Every other report is
`CommandFailure::Report`, retaining the original work error, panics and every
cleanup error; a completed value there is reachable only by reference. A
coordinator lost without a report is `CommandFailure::Coordinator`, retaining
its `JoinError`: cancelled on runtime shutdown, or a panic when the coordinator
itself panics, as it does without the runtime's time driver, with no cleanup
claim; `finish` never panics. A command that completed before its runtime died still delivers its
value to `finish` on another runtime. `T` and `E` need only `Send + 'static`.
Dropping the owner, or an unfinished `finish` future, requests work cancellation,
downward only; before the coordinator first runs, the factory is destroyed
uninvoked. Cleanup continues on the live runtime, and the unclaimed report, with
its value, errors, panics and cleanup errors, is destroyed under the
coordinator's captured dispatcher and span rather than wherever Tokio drops task
output. The captured span itself is released under that dispatcher whether or
not `finish` claimed the report, so a nested span's parent closes in its own
registry. The work deadline and cleanup budget already bound `finish`; wrapping it
in an application timeout and then leaving the root lets runtime shutdown
abandon cleanup.

`start_with_unix_signals` installs SIGTERM then SIGINT listeners synchronously
before returning, unless the context is already cancelled or expired: then
nothing is installed and the report records that interruption without invoking
the factory. An installation error returns before anything is spawned, after the
factory is destroyed uninvoked. Only the coordinator polls the listeners, and only
while work can still be cancelled: one non-blocking poll before the factory, so a
visible signal destroys it uninvoked; every round in the order cancellation,
signal, deadline, work; and once more when work returns, so a coincident signal is
`interruption_after_work = Some(Cancelled)` and the command is unsuccessful, as
after a concurrent `cancel`. Each observation runs outside Tokio's cooperative
budget, so work that spent the task's budget in its final poll cannot hide a
visible signal. A signal cancels the command's own scoped child
context, reaching descendants before the work future is destroyed and never the
parent, and is recorded as `Interrupted(Cancelled)` without its identity. The
listeners are dropped after work-future destruction and before cleanup on every
path. Cleanup then runs under its own budget; a signal during cleanup or after
`finish` neither interrupts, fails nor restarts it. A command started inside
another command's work is a separately owned task: if the outer work is
interrupted while awaiting the inner `finish`, the inner owner drops and its
cleanup runs detached, joined and reported by nothing. Resources acquired inside
work register on the outer `CommandScope`.

Signal ownership limits: Tokio replaces each signal's process-wide disposition at
installation and does not restore it when listeners drop. After work stops,
during cleanup, after `finish` and between commands, SIGTERM and SIGINT are
consumed without effect; a root with later phases needs its own signal policy
for them. Whether a signal arriving between two signal-owning commands cancels the
second before its work starts depends on when a runtime's signal driver processes
it. Reception is cooperative, not a fence at kernel delivery, and non-yielding
work is not preempted. Signal identity is not recorded. A failed SIGINT
installation leaves SIGTERM's disposition replaced; there is no rollback.
Installation panics, as Tokio does, outside a runtime or without its I/O
(signal) driver, and returns an installation error once that driver is gone. The
coordinator also needs the time driver.
`start_without_signals`, `start` and an inert command install nothing, so
SIGTERM and SIGINT keep the process's existing disposition.

Command interruption is captured at the final poll before destroying the work
future. Cancellation or deadline expiry during destruction cannot reclassify
completed work; destruction panics and cleanup failures still make the report
unsuccessful. This differs from startup readiness, which requires a live context
after initializer destruction before acknowledging readiness.

For the shared path, cancelling borrowed waiters has no effect. Dropping
`RunningCommand` requests work cancellation; observers retain completion without
keeping that owner alive.
Work completion cancels child contexts before cleanup. Finalizers must use their
native independent cleanup, not captured work cancellation tokens. Resources are
registered through validated reservations; no takeable stack lets the canonical
scope silently relinquish finalization. Unregistered resources and arbitrary
spawned tasks remain outside this future/cleanup ownership contract.

`Command::new` gives cleanup its own allowance after work stops. `Command::within`
reserves cleanup's total work and abort-observation allowance inside an enclosing
absolute deadline. Work and cleanup cannot restart that total. Scheduling delays
can consume the reserve and produce explicit skipped hooks. Neither variant can
preempt non-yielding code, undo remote effects or finalize after runtime death.
Shared published reports survive runtime destruction; an observer whose monitor
died before publication cannot fabricate a completed report. Tests in
`tests/command` cover these ownership/budget distinctions, private
injected-source tests in `src/command/tests` the signal observation points, and
fresh-process `tests/command_signals` real SIGTERM/SIGINT; the finite example
exercises native UDP and real signals at its executable root.

## Owned startup contract

`Startup` initializes a running supervisor, not a standalone finite command.
After successful initialization, no critical components **and** no configured
finite-work capacity produce `ShutdownCause::EmptySupervisor`. The initializer
can succeed and every finalizer can succeed while the shutdown report remains
unsuccessful. Such a supervisor closes admission before its driver starts, so it
never publishes `Ready`; `service::start_with_readiness` reports
`StoppedBeforeReady` for it. Configured finite capacity permits a component-free running
supervisor; root work still requires readiness, submission and explicit shutdown.
The executable startup rustdocs and `startup_composition` tests cover these
distinct outcomes without changing the empty-supervisor contract.

`Startup::protected` is the canonical agent-consumer path. Its initializer is an
owned async closure (or `async fn`) over `ProtectedStartupScope` whose future the
library boxes; the `Initializer` bound requires that future to be `Send` and its
output to be `Send + 'static`, so the output cannot borrow the scope.
`Startup::scoped` accepts the lower-level caller-boxed `StartupFuture` closure
with unit output and otherwise shares the scope, coordinator and policies. The
`ProtectedStartupScope` exposes only validated stage selection, direct cleanup
reservation and a private-field `Registration` view. That view forwards ordinary
and managed registration but cannot start or replace the process, extract cleanup,
recover a `Supervisor`, or outlive its borrow. The sealed `RegistrationTarget`
lets native adapters accept either this protected scope or the lower-level
supervisor without allowing application-defined targets. Handles and contexts
captured before startup remain outside this restriction.

Protected startup retains application failures inside the redacted,
non-exhaustive `InitializationError<E>` envelope. The driver maps a returned `E`
at its poll boundary before destroying the completed initializer future, so the
application error and a separate destruction panic can coexist. It uses the same
coordinator, cleanup, report publication and running handoff as the legacy path.
`E` stays concrete. `ApplicationError` is the opt-in dynamic error: `?` converts
anything that converts into `BoxError`, the box is retained unchanged, and
`InitializationError<ApplicationError>` implements `std::error::Error` with that
box as its `source()`. `ApplicationError` itself deliberately does not implement
`Error`, so its blanket conversion is coherent; Rust coherence forbids giving
`InitializationError<BoxError>` that implementation beside the generic one.
Formatting of both stays fixed text.

Typed initializer output travels only in the one-shot handoff:
`StartingSupervisor::finish_with_output` returns it with the running (or
unapproved) owner, observers receive only the supervisor observer, and every
failure carries no output. Output returned in the same poll that observes drain,
cancellation or deadline, or followed by a destruction panic or a failed final
lifecycle check, is destroyed before cleanup. Its destructor panic is retained in
`destruction_panic` unless the initializer future's own destruction panic already
occupies that field. The later payload is then released only when it is a
`String` or `&'static str`; any other payload is intentionally leaked, because its
destructor could panic again inside the coordinator before cleanup. Dropping the
starting owner instead of waiting destroys the queued owner and output together,
on the dropping task or the monitor task, and requests drain. `finish` exists only for unit output. `service::start` accepts only unit
output; `service::start_with_readiness` delivers typed output after readiness.

`Startup::new` remains the explicitly lower-level compatibility path and invokes
no initializer. `start` launches the owner on a live Tokio
runtime. Dropping the inert builder only abandons its unstarted supervisor;
no asynchronous cleanup is implied. Once started, dropping `StartingSupervisor`
requests drain while the coordinator drives registered cleanup independently.
`finish(self)` and `finish_with_output(self)` consume that owner, so another
handoff call on the same value cannot compile. Their returned future owns the
pending handoff even before its first poll. Dropping it requests drain; cancelling
a borrowed wait on the retained, pinned future leaves initialization and handoff
owned. Resume that same future rather than creating a new handoff call. The
ordinary Rust future contract still forbids polling after completion.
`StartupObserver` retains a startup failure or a shutdown observer, never a
running owner. Dropping an unclaimed handoff requests running-driver drain.

Initialization checks the operation deadline/cancellation and process drain,
including before factory invocation and after successful future destruction.
While the initializer is pending, each private observation boundary first rejects
an already-ready drain, cancellation, deadline or configured Unix signal without
polling application work again. Otherwise it polls the initializer once and then
observes sources that became ready during that poll. A concrete application error
or initializer panic observed in that poll is retained; signal reception still
requests drain. A successful initializer observed with a signal transfers the
consumed reception into reserved registration, whose drain request is caught by
the final check, so it cannot publish readiness or require a second signal. An
independent destruction panic does not suppress that final lifecycle classification.
By default success consumes the one-shot application approval; the running driver
and every critical acknowledgement are still required.
`Startup::without_readiness_approval` instead hands off an
`UnapprovedSupervisor` while application readiness remains Starting, so a later
composition stage can consume it into `RunningSupervisor`. Component
acknowledgement does not substitute for that approval. A drain request cannot
revive Ready. Failure preserves the
application's concrete error and static stage, interruption or unwind payload,
an independent initializer destruction panic, and all cleanup outcomes.
Stage metadata must be suitable for diagnostics. Default error formatting omits
cause contents; explicit error sources/panic inspection are trusted operations.
The shared `batter::PanicPayload` type is used by startup, command and managed
lifecycle reports; its original `batter::startup::PanicPayload` path remains a
compatible re-export. `PanicPayload::try_inspect` never waits for another inspector: concurrent or
recursive access returns `PanicPayloadBusy`. A callback panic does not poison
future inspection. Inspection remains synchronous and temporarily excludes other
inspectors; it does not clone, format or expose the retained payload automatically.

Cleanup has its own budget, independent of the startup operation context.
Manual `CleanupStack::push` and `CleanupSlot::register` are lower-level paths:
the caller owns acquisition-to-registration sequencing. Use them for application
hooks and resource protocols without supported native/adapter acquisition.
`CleanupStack::push` can also reject the name after acquisition.

`reserve_cleanup` / `CleanupStack::reserve` validate names before acquisition;
the exclusive `CleanupSlot` registers its finalizer infallibly. An unused slot
adds no hook. The older `push` still consumes its factory on rejection. Native
acquisition cancellation may have external effects before yielding ownership;
resources acquired but not registered have only their native destruction
semantics. For manual registration, register immediately after success without
another await. No detached work, arbitrary destructor, non-yielding initializer,
aborting panic, process or
runtime destruction is covered. The default panic hook may still print secrets;
an application root that installs the
[redacting panic hook](#application-installed-redacting-panic-hook) first prints
each panic's location instead.
Unexpected coordinator termination retains a JoinError without inventing cleanup.

Protected `.with_unix_signals(name)` is inert until `start`. Start first retains
repeated selection as a configuration error; otherwise it checks existing drain
or interruption, reserves the component identity, and installs SIGTERM then
SIGINT synchronously before returning the owner. The coordinator owns reception
during initialization and fulfills the reservation with one real critical task
before running handoff. A signal observed during initialization requests drain;
it cannot approve readiness. Repeated signals do not restart cleanup's separate
fixed allowance or turn a timed-out finalizer into successful cleanup. Preflight failures skip the initializer but remain
owned through prior cleanup, even if the startup owner is then dropped. Cleanup
slots use a distinct namespace from component reservations.

`register_signals` and `install_signals` remain lower-level compatibility helpers.
The latter requires its caller to poll `InstalledSignals::received` during
initialization and register the same sources before handoff. Tokio changes
process-wide signal disposition and does not restore it on listener drop; a
partial installation failure is not rollback. Signal-enabled start requires a
live Tokio runtime with signal support. Reception is cooperative and is not an
atomic fence at kernel delivery. Process-global signal selection belongs at the
executable root; reusable in-process test helpers must not install it implicitly.

Standalone finite commands use `Command` to retain work and cleanup independently
of the caller's waiter; at an executable root, `start_with_unix_signals` owns
SIGTERM/SIGINT for the work phase only. The lower-level `OperationContext::run` and
`CleanupStack::close` remain separately driven operations; composing them manually
does not create an owned finalization driver. No remote database termination or
rollback guarantee follows from either local completion report.

For supervised services, the owned driver's `wait_checked` and
`shutdown_checked` classify completion. Failures retain the complete report,
including forced abort, skipped cleanup and unjoined work, or the original
coordinator error. Their redacted formatting does not inspect those causes.
`check_shutdown` remains available for callers deliberately handling a raw
report.

The reference root uses protected startup with `.with_unix_signals("signals")`,
a `pool_in` close hook registered on its reserved `postgres.pool` slot, and managed
native settlement. Library-owned listeners exist before the initializer runs and
belong to the running driver after handoff. Even after generic shutdown succeeds,
the application root requires its named `postgres.pool` cleanup record;
`pool_in` registers the close hook, the cleanup stack writes its record, and
checked completion separately requires that hook to have succeeded.
Cleanup registration rejects duplicate names, so if the otherwise successful
report does not contain that record,
the retained `ServiceCompletion` from `runtime::run` or `runtime::start` carries the public fixed-diagnostic
`RuntimePoolCleanupFailure` as its service failure; its `report` accessor retains typed inspection without
formatting application errors.
A signal observed during acquisition
or schema initialization yields `StartupCause::Draining` inside
`ProtectedRuntimeStartupFailure`, with the awaited pool cleanup record and no
fabricated application error; the executable renders only its fixed diagnostic
and exits with status 1. The earlier startup wrapper was removed in the
coordinated hard cutover. Native loop acknowledgement, fresh PostgreSQL sampling and
application readiness approval are separate facts. The reference now registers
its delivery handler before native preparation and grants ordinary application
approval; readiness still requires all three inputs.
Production starts no durable control job, advisory-lock owner or reconciliation
pool. The startup allowance is 20 seconds. Process drain, cancellation and reap
allowances are ten, one and one seconds, followed by separately bounded pool
cleanup. Native graceful/abort settlement consumes the same process stop clock.

Applied legacy migrations and the owner-epoch sequence remain intact. The separate
finite retirement command requires explicit database identity and operational
quiescence, disables the exact legacy definition and cancels only its nonterminal
jobs through native APIs. It preserves history and domain rows. Independent
readback can clarify a lost commit acknowledgement but never replaces or erases
the original command failure. Retirement is not run by service startup; its
fixture acceptance is not evidence of a deployment retirement.

## Periodic maintenance components

`periodic::register_periodic_in` registers recurring maintenance through the
same sealed `RegistrationTarget` authority as native adapters, so a component
can be added without process start, replacement or cleanup-extraction
authority. Registration is inert and fully validated first: the name shares the
one component vocabulary and rejects duplicates and invalid spellings, and
`PeriodicPolicy` rejects a zero or unrepresentable interval, run budget or
initialization allowance before anything can run. Construction, a rejected
registration, an abandoned unstarted supervisor and a never-polled driver all
invoke no application work. A run budget longer than the interval is a valid
selection. The returned `PeriodicReader` is read-only: it owns no work and no
history, cannot run, stop, cancel or clear anything, and reading it is never
required because the completion report carries the same evidence.

One registered component owns exactly one directly polled serial future. There
is no detached per-run task, no event queue and no second scheduler, so
invocations cannot overlap. The first invocation is immediate once the
component can run; later ones follow a fixed interval with missed ticks
skipped. One overdue invocation may run immediately after an overrun, and the
schedule then realigns without replaying a burst of missed work, including
intervals and overruns shorter than Tokio's five-millisecond missed-tick
tolerance. The private schedule advances to the first future point on its
original cadence before admitting that overdue run; this is start-to-start
scheduling, not completion-plus-delay. Tokio timers retain their native
resolution, so sub-millisecond intervals do not promise sub-millisecond wakeups.
If no later cadence point fits the monotonic clock, the component waits for its
existing initialization or shutdown boundary instead of overflowing or spinning.
Each run receives a
library-created `OperationContext` with its own fresh deadline and a
cancellation lineage descending from process forced cancellation, so a child
cannot exceed the run and ending one run cancels only its own child scope, not
a sibling or a future run. The recorded semantics of the schedule and pinned Tokio timers
are in [references](references.md#periodic-maintenance-scheduling-semantics-2026-10-08).

Startup acknowledgement is an explicit policy. `PeriodicStartup::immediate`
states that the loop is initialized, not that maintenance succeeded.
`PeriodicStartup::after_first_success` requires a selected positive total
allowance, measured from the component's first live execution and including
failed runs and the interval waits between them; each initialization run is
additionally capped by whichever of the run budget and that allowance expires
first. When its run budget is longer, a cooperative pending run receives the
original initialization deadline and is destroyed when it expires under either
stopping class; a preceding failed run and cadence wait do not restart the
allowance. A qualifying success acknowledges exactly once, and later run failures
never revoke it or establish continuing lease, renewal or dependency health.
The driver owns one explicit state: initialization carries its startup capability
and absolute allowance deadline; running carries only the acknowledged capability.
The transition consumes the startup capability and removes its deadline, so there
is no empty or doubly owned capability state and later runs cannot reuse the
initialization allowance. Normal exit consumes whichever capability remains.
Because the run boundary is cooperative and rechecks neither its clock nor
drain after the work's own poll returns, a success that only arrived after the
allowance had expired, or after drain began, is still counted as a successful
run but cannot acknowledge startup or assume the support obligation. An
observed drain abandons that pending initialization; an expired allowance stays
a retained initialization failure. Either way readiness is not published.
Observed drain abandons pending initialization, including for support
components, and expiry is a distinct retained library initialization failure
(`PeriodicInitializationExpired`) that initiates the existing drain sequence.
Neither policy can publish readiness without its acknowledgement, and both run
their initial invocations while the process is still `Starting` through a
private admission path that does not weaken readiness-gated public operation or
root process admission.

Registration requires one of exactly two library-owned stopping classes.
`PeriodicShutdown::StopAtDrain` stops admitting runs and destroys an active run
on global drain. `PeriodicShutdown::SupportThroughDrain` keeps running bounded
work while ordinary direct components, queued or active finite work and
admitted descendants, or native settlement still need it. The coordinator owns
that stopping point and closes support admission once ordinary direct work has
been joined with termination evidence adequate for dependency cleanup, no
finite work or descendant remains, and every retained managed outcome allows
dependency cleanup. A joined panic or abort is deliberately not adequate: the
same exit makes the coordinator skip finalizers and its descendants may still
need support, so support then runs to forced cancellation. A returned
application error is cooperative termination and does close support. Pending or
uncooperative native settlement is not a stopped proof: neither a joined
wrapper nor a finished native report alone suffices, and settlement is observed
during drain rather than after support stops. All support components are excluded
together from the predicate they await: two support schedules do not wait for
each other after ordinary work settles, and a process with no ordinary work
closes support promptly on drain. If that boundary is never reached, support
closes at the existing global forced-cancellation boundary; earliest-stop-clock
tightening, the cancel and abort/reap limits and conservative cleanup skipping
are unchanged, and support work is joined or destroyed before finalizers run.
For an initialized component, observing the support stopping point interrupts
an active cooperative run; it does not wait for that run's budget
or the remaining drain allowance. Its future is destroyed before dependency
cleanup and the interruption is retained in `stop_interrupted`. A paused-time
regression keeps a run pending through ordinary drain, then proves destruction
after ordinary settlement and before cleanup without forced cancellation.
A run admitted during drain is additionally capped by the already recorded
forced-cancellation instant.

A component whose initialization is still pending is admitted as ordinary work
whatever its stopping class, because it has taken on no support obligation yet
and pending initialization abandons on global drain. That abandonment is an
expected exit even while ordinary work keeps the later support boundary open.
Treating it as support would grant a run after its applicable stopping point
and leave only the cancellation preflight to stop it.

Admission transitions stay private to the lifecycle state owner. Grants and
rejections are taken under the single transition mutex and returned as plain
values, so no application factory, destructor, poll, span, subscriber or
recorder code runs under that guard. An ordinary task failure closes ordinary
admission, exactly as it closes finite process admission, without closing
initialized support admission while consumers are still draining. The
guarantee is that no run is admitted after its applicable stopping point.
Batter deliberately does not claim that invoking an already admitted callback
is atomic with stop: cancellation preflight happens, but a previously admitted
callback can still race physical invocation.

Run outcomes are exhaustive and distinct: success, recoverable application
failure, run deadline, normal stop interruption, and explicit escalation.
`From<E>` on `PeriodicFailure` produces the recoverable variant, so `?` on an
ordinary application error keeps the schedule and cannot silently drain the
process; `PeriodicFailure::Fatal` has no conversion and must be written out.
The `recoverable` and `fatal` helpers select those same outcomes explicitly.
Consuming `into_inner` returns the original concrete cause and deliberately
discards its classification, as the executable helper examples demonstrate.
Only that explicit escalation escalates a returned application outcome, while a
native factory, poll or destructor panic, unexpected component termination and
initialization failure stay terminal. A recoverable failure or an expired run
deadline permits the next scheduled invocation and never an extra immediate
retry. Registration authorizes recurrence only: it proves no remote rollback,
idempotence, successful renewal, fencing or absence of a timed-out effect, and
lease-loss, freshness and durable witness policy stay application- or
native-owned.

Retained evidence is bounded. `PeriodicSummary` keeps saturating counters that
report saturation instead of wrapping, counts deadline and stop interruptions
separately, records at most one outcome classification per admitted run, and
explicitly accounts for the recoverable failures it did not sample. The first
and last recoverable causes are retained concretely: `first_failure` keeps the
first cause for the component's lifetime, while `last_failure` remains `None`
until the second recoverable failure and then keeps the newest subsequent cause.
When `last_failure` is `None`, `first_failure` is the newest retained cause, if
any. Both slots retain concrete causes without an `E: Clone` bound or
error-string conversion and stay inspectable through the
inherent `downcast_ref` on `dyn Error + Send + Sync`; only the library-owned
sample count is bounded, not
application payload size or caller-retained clones. Later success never erases
earlier evidence. The supervisor retains that history independently of the
runner, so it survives a later panic, abort or lost waiter and appears in
`ShutdownReport::periodic` with no second application join and no mandatory
observer polling. A runner that published no final snapshot leaves
`PeriodicCompletion::Pending`, which marks the snapshot explicitly incomplete
and claims nothing about termination; so does a runner the coordinator lists as
unjoined, because a published marker is the loop's claim about itself and
cannot stand without observed join evidence. That reconciliation uses only
registered component names, because finite task labels are a separate
vocabulary that may repeat and may match a component's name. `PeriodicSummary::has_failures`
also reports the terminal `Fatal` and `InitializationExpired` completions,
which advance no recurring counter. Expected `AbandonedDuringStartup`
completion alone does not make this predicate true, but any recorded run failure
or interruption still does, including a pending initialization run interrupted
by drain. This predicate does not determine checked shutdown success.

`invocations` includes active work and runs lost before classification through
panic, abort or an unjoined runner, so it need not equal the classified outcomes
plus a fatal run. A final completion marker describes how the loop ended; it
does not promise exact accounting when counters saturated or an interruption
classification was lost to a destruction panic, as described below.

The concrete terminal cause is retained in `PeriodicSummary::terminal_failure`
as soon as the run produces it, while that run's own future is still alive, and
therefore before either that future or the factory's captures can be destroyed.
A panicking destructor in either place unwinds the component's normal return,
so the error it was carrying would otherwise be replaced by the panic alone;
the retained slot keeps the original cause while the panic stays visible in the
task records and still forces conservative cleanup skipping. Every outcome the
run itself produced — a success, a recoverable failure with its bounded sample
and accounting, or an escalation — is published at that same point, so a
destruction panic cannot erase the invocation's evidence. The boundary's own
outcomes, an expired run deadline and a stop interruption, carry no application
cause and are counted by the loop afterwards, so a destruction panic can still
cost one of those counters; the panic in the task records is then the
authoritative evidence for that invocation. Retained terminal
evidence is reported by `has_failures` independently of the completeness
marker, because a runner can retain its cause and then be lost before
publishing one, or have its marker reset for want of join evidence. An escalated run's error is shared
with that slot, so the task record exposes it through `Error::source`, exactly
as a finite process task's shared failure does; the library's own
initialization-expiry error stays directly downcastable because it is
reconstructible. That library event uses the number of runs admitted before
expiry as its `terminal_failure.invocation` (zero if none); unlike an escalated
run's one-based index, it does not identify a failing invocation. `Debug` and `Display` stay redacted, a
displaced cause is destroyed outside the publication lock inside the
component's protected dispatch, and recoverable history alone does not make
checked lifecycle completion fail. Terminal task failures, unjoined work and
unsuccessful cleanup keep their existing failure meaning.

## Maintained Runlimit PostgreSQL storage

Native PostgreSQL quota storage keeps an expired counter, and the capacity slot
it occupies, until a bounded `cleanup_expired` call deletes it; a full shard then
denies new keys. `batter_runlimit::maintenance::register_in` (facade
`batter::runlimit::maintenance`, feature `runlimit-postgres`) is the canonical
construction of a supervised PostgreSQL limiter. It takes a native
`PostgresLimiter` or `PostgresGcraLimiter` and a validated `ExpiryMaintenance`,
registers that limiter's cleanup through `periodic::register_periodic_in`, and
only then returns the `MaintainedLimiter` the application serves with.
`MaintainedLimiter` has no other constructor, so the protected path cannot produce
a PostgreSQL limiter whose cleanup was never registered. It implements the native
`Limiter` trait by delegating each check unchanged, so `Quota::new` and the HTTP
assembly serve it exactly as they serve the native limiter. The native limiters
remain reachable through `batter_runlimit::postgres` as the named, unmaintained
low-level form: nothing deletes their expired rows unless the caller schedules
cleanup itself.

Registration is inert and validated like any periodic component: an invalid or
duplicate name is rejected and nothing touches the database before the
supervisor starts it. `ExpiryMaintenance` takes a nonzero row bound, because the
native call treats zero as "do nothing", and validates its interval with the
`PeriodicPolicy` rules. Each run calls the native `cleanup_expired` once, on the
limiter's own pool, login and configuration, deleting at most that many expired
rows; there is no second cleanup query in the adapter. The per-run budget is not
selectable: it is the limiter's pool-acquisition timeout plus its operation
timeout plus one second, so during ordinary running the native bounds report a
slow acquisition or statement with their own `MaintenanceError` classification
before that budget expires. An earlier initialization allowance or shutdown
boundary can still destroy the native future before it returns that error;
the periodic report then retains its deadline or stop-interruption outcome,
with initialization expiry reported separately. An executed negative control that
shortened the derived budget below the acquisition bound turned that native
classification into an unclassified `deadline_exceeded` run. Every native
maintenance failure, including an uncertain commit that may have removed rows, is
retained as a recoverable periodic failure; maintenance never initiates a drain.
Removed-row counts reach a native observer installed on the limiter.

Maintenance runs under the limiter's login, which therefore needs both its
store's admission selection and its expiry-cleanup selection from
`batter_runlimit::grants`. Neither cleanup selection needs relation-level
`SELECT`. A cleanup transaction checks its privileges even when nothing has
expired, so `PeriodicStartup::after_first_success` withholds readiness until one
cleanup has committed under that login; with `PeriodicStartup::immediate` a
missing grant appears only as recurring retained failures while the process
serves. Every stopping class is joined or destroyed before finalizers run, so a
pool closed by a cleanup finalizer such as `batter_sqlx::pool_in` begins to
close only after maintenance has stopped and any active run has been destroyed;
the destroyed run closes its own connection.

Registration does not prove that the schema is installed, that rows were
removed, or that cleanup keeps pace: each run skips rows locked by concurrent
admission and removes at most its row bound per interval. That bounds deletions,
not rows scanned: PostgreSQL chooses the access paths, and a forced generic plan
can scan the counter table even for a small deletion bound. Migrations, pools and
their ownership remain the application's. The offline `maintenance` tests in
`batter-runlimit` and the two `maintenance_cases` live cases in the facade's
`grants_live` suite are the evidence for these statements.

## Dependency health sampling

`HealthMonitor` owns one native probe factory and creates no tasks. Constructing
it or its consuming `run` future is inert. Canonical `register_in` validates the
component before transferring the writer and returns its `HealthReader`; failure
drops the inert writer without invoking a probe. Direct `run` remains available
for lower-level composition. First poll acknowledges that the sampling loop is usable,
not that the dependency is healthy. Application startup approval and dependency
health remain separate. Returned probe errors and timeouts update health and
allow recovery; panics propagate to the critical task boundary and stop the writer.

`HealthPolicy` requires positive, representable probe budget, completion-to-next
probe delay, maximum age and scheduling margin. Checked arithmetic requires
maximum age >= delay + probe budget + margin. Probes never overlap. Each delay
starts after the previous attempt's completion and destruction; delayed polling
admits one next attempt, without an interval catch-up queue. The budget covers
the complete application future, including acquisition and query work. A result
observed at/past its deadline is classified as timed out. Blocking construction,
polling or destruction cannot be preempted by that deadline.

Readers synchronously copy one result/timestamp and writer liveness under a short
private mutex. They perform no probes, create no tasks or per-reader queues, and
never renew an observation. No application formatting or replaced-error destructor
runs under the publication mutex. Only the latest probe is retained; a fresh
in-budget failure retains its concrete E behind Arc without requiring E: Clone.
Debug omits E's contents. Trusted inspection and normal ownership/destruction of
retained application values remain application responsibilities.

Unknown, failed, timed-out, stale and stopped states are all unready. At exactly
maximum age the last result is stale, even if the owner still exists or has
stalled. A later successful probe restores healthy status. Dropping the sole
writer invalidates future reads immediately; reader clones and saved snapshots
do not keep it alive. A snapshot is historical as of `observed_at`, so readiness
must obtain a fresh read rather than caching a healthy snapshot forever.

On observed drain or forced cancellation the monitor admits no new probe and
destroys active directly owned work before returning. A completion followed by
drain during destruction cannot publish success. Unpolled abandonment and outer
run-task abortion stop the writer too. Retained last-probe details after Stopped
are diagnostic history, not evidence that the dependency remains healthy or that
remote work stopped. No detached-child, remote-query cancellation, async Drop,
runtime-death or non-yielding shutdown guarantee is added.

The HTTP example combines process Ready with a fresh health read at `/ready`.
Observing lifecycle drain overrides cached success; this is not an atomic joint
snapshot with concurrent lifecycle transitions. A request racing a later drain
can still use its earlier Ready observation, as with existing admission.
Dependency observations do not mutate process state or automatically alter
`RequestPolicy` business-route admission. The example's probe is an explicit
simulated dependency read, not evidence of actual database availability.

## Cleanup contract

The coordinator receives an owned task summary only after releasing its task
collection. The private join operation records each consumed result before
returning; abandoning a pending join waiter neither cancels owned work nor
discards its later result. Summary extraction retains unjoined names and does
not promote asynchronous abortion into proof of termination.

`CleanupReport` is `#[must_use]`. Awaited completion can still contain failed,
skipped or unjoined finalizers; callers must inspect the retained report.

Finalizers are owned Send factories, invoked inside directly owned Tokio tasks.
They run sequentially in LIFO order. Per-hook work is capped by a shared work
deadline. After timeout, abort observation is capped by both a per-observation
allowance and one final absolute deadline. Total phase allowance cannot reset
once per hook. Errors and observed panics are retained while later hooks run.
Every skipped hook receives one report entry and one warning, in LIFO order;
skipping never invokes the registered factory.

A timed-out finalizer that is joined has stopped its direct future. If joining
cannot be observed, remaining dependency hooks are explicitly skipped. Hooks
must not detach work; joining a hook cannot prove hidden work completed. No
source or panic payload is automatically included in tracing fields.

Dropping CleanupStack does NOT call the hooks. It emits a count-only warning.
Skipping or dropping a hook still drops its captured values using their native
Rust Drop behavior; it does not keep resources alive indefinitely or suppress
those types' own destruction protocols.
For `CleanupSlot::acquire_semaphore` and `bind_udp` that drop is the whole
finalizer, so a skipped stack releases the permit and the library's socket
reference while its report lists both names under `skipped`: the entry means
the factory was not invoked and dependents were not shown to have stopped, not
that the resource is still held. A unit test asserts both the release and the
report entries. The report types are unchanged.
Dropping close mid-flight aborts its local task but cannot await completion or
return a report. Shutdown/readiness, resource acquisition, and cleanup are not
interruptibility-masked like a full managed effect runtime.

Supported acquisition uses `CleanupSlot::acquire_semaphore` and `bind_udp`, or
an adapter-owned operation such as SQLx `pool_in`. The core native methods poll
the actual native acquisition future, retain its returned resource, register the
library's cleanup action, and destroy the completed native future before returning.
There is no caller callback, projection or await between these steps. A native
future destructor panic leaves the finalizer installed and releases the undelivered
caller reference during unwinding. Native acquisition errors add no finalizer;
never-polled calls do no acquisition. Pending semaphore cancellation retains
Tokio's queue-position policy. These methods do not bound the number of waiters.

Semaphore permits remain owned exclusively by cleanup until release. UDP returns
an Arc of the native socket; cleanup releases its own reference, and the socket
closes only when all caller references are dropped too. This does not join tasks,
close arbitrary retained handles, or establish remote effects. Cleanup still
requires its owning driver and a live runtime.

`cleanup::low_level::acquire_pair` registers an opaque `low_level::Acquired` pair
before delivering its handle or destroying the pair-producing future. Its caller
still owns the earlier native acquisition-to-pair transition, including any await
or destructor panic inside that future. This is deliberately not equivalent to
supported native acquisition. `CleanupSlot::register` is the manual handoff;
resource-specific protocols without a supported adapter retain that obligation.

## PostgreSQL client disposition

Managed apply with application SQL has one native transaction for definitions,
ledger and callback grants. `migrate_and_apply_with` reuses savepoint/XID checks;
refusal cannot become success, and lost continuity is unconfirmed even when the
callback also returned an error. It rechecks managed evidence and PUBLIC policy
before commit, after forcing deferred work. Committed migrations remain outside
that rollback boundary. The opaque SQL capability is not a sandbox: explicit
transaction control may have irreversible effects before detection. Every
callback path retires its session; success has no await after acknowledged COMMIT.
The compatible no-callback method retains its earlier disposition contract.


The optional `batter-sqlx` package owns checked-out client disposition and protected
pool acquisition. `pool_in` synchronously consumes a prevalidated cleanup slot,
constructs a lazy `PgProfiledPool` from native options and an explicit profile,
and registers awaited `Pool::close` before returning the pool. All three session
hooks are library-owned. `RunledgerDatabase::from_profiled` checks the existing
one-schema rule and shares capacity, profile and closure without another finalizer.
This establishes neither connectivity nor readiness. SQLx may
start native minimum-connection maintenance during construction; constructor
panics and runtime death remain native limits. Dependent work must be joined and
checked-out connections released before successful cleanup can be expected.
When cleanup is skipped, the registered close is not awaited: the finalizer's
pool handle is dropped, which closes nothing while another handle exists. Each
server session then ends when its socket closes, at the latest at process exit,
without a Terminate message, and PostgreSQL rolls back an open transaction; a
statement already running outside a transaction block may still commit. Handles
kept by surviving tasks keep their sessions for as long as those tasks live.
This follows from the PostgreSQL 18 protocol text and the resolved SQLx source
([references](references.md#postgresql-execution-managed-objects-and-verification));
it is unverified against a live server.
Local completion evidence requires the named successful cleanup record, zero
remaining pool size and `PoolClosed` from a later acquisition; `is_closed()` alone
only witnesses that close began. These facts still say nothing about detached
server sessions.
`low_level::register_pool_close` remains a lower-level path for pools acquired
elsewhere, with caller-owned cleanup if registration fails. The workspace's
canonical service roots use `pool_in`; the finite retirement command deliberately
uses `low_level::pool_in` to retain its physical-session guard. Standalone native
constructors are also confined to `low_level` and retain caller-owned cleanup.

`run_atomic` owns the complete READ COMMITTED workflow. Body outputs are withheld
until acknowledged commit, and rejections until acknowledged rollback; uncertainty
retains the result and cause through the narrow `PgAtomicUncertainty` type.
Caught terminal failures poison the scope with their first shared native cause;
later calls cannot replace it or invoke new work. A dropped polled operation
records `OperationAbandoned`, not a fabricated boundary-loss error. Ordinary
application rejections and terminal scope failures use separate types.
Recoverable operations use private savepoints and XID continuity.
The policy-based `run_atomic_with` / `run_atomic_profiled_with` entry points
fix the consumer error type before entering SQL. They use the same owned runner;
policy mapping cannot restore a lost owner or turn an uncertain disposition into
`Ok(T)`. All five failure-policy methods are required, and receive concrete
provisional output, rejection and native causes as applicable. Interpreting or
discarding these values in consumer error mapping is application policy, not a
remote-effect proof. The `_with_in` variants preserve an observed completion
before local budget resolution. Cancellation triggered by a completion-mapping
callback retains its mapped error as `OperationError::Failed` with the observed
evidence. The `scope_lost` callback runs inside the body; if it cancels the
operation before rollback is acknowledged, the operation can instead return
`OperationError::Interrupted`. Native Runledger keeps the same policy through its consuming
intent-to-queue transition and named operations. Intent observation and queue
enqueue methods convert native storage errors through the consumer's
`From<runledger_postgres::Error>` implementation.
`record_required_job_enqueue_intent` instead wraps storage errors in
`RequiredIntentError::Storage` and uses the consumer's `From<RequiredIntentError>`
implementation. These recoverable errors are returned only after successful
savepoint recovery; catching them permits later SQL and acknowledged commit.

`run_atomic_fail_fast_with` and its profiled/budgeted variants use one private
guard at transaction birth, with no savepoint around each successful operation.
The body and validation cost two statements for a one-query unprofiled call,
three when profiled, excluding setup and completion. Ordinary rejection rolls
back the guard, validates the original transaction, and acknowledges whole
rollback before returning the original error. Missing guards, replaced transactions,
validation/transport failures and abandonment retain uncertainty. A consumer
must implement `From<PgScopeRolledBack>`: catching a rejection cannot authorize
later SQL or turn the closed transaction into successful completion. This marker
retains rollback evidence, not a clone of a deliberately discarded application
error. `recoverable_sql` explicitly selects operation recovery but cannot reopen
a closed transaction. Application savepoints can survive successful fast calls;
arbitrary SQL and external effects remain outside rollback guarantees. Native
Runledger preserves this behavior through its consuming phase transition.

Native SQLx query helpers accept sealed `Query`, `Map` and `QueryScalar` values.
They preserve SQLx macro checking and native fetch mapping, including missing-row
and decode errors; runtime query constructors keep their native weaker checking.
`execute` discards rows and mappers. Scope helpers use the existing runner-selected
recovery mode and never release provisional outputs as committed values. The
native intent-to-queue transition remains consuming. Their dispatch futures are
boxed to preserve Send through lending callbacks on supported Rust compilers;
query output and consumer error types remain concrete.

`PgQueryHandle` owns one positive, parent-clamped total deadline and error mapper
for independent pooled queries. Each query owns its acquired lease, with no public
resource replacement or return authority. Acquisition/query/cleanup share the
same budget across calls. Successful cleanup permits reuse; failed work, failed
cleanup and interrupted cleanup retire that lease. The native query result is
retained before asynchronous pool-return normalization and resolved before
operation telemetry, so an observed result is not overwritten by cleanup
cancellation. The mapper receives native acquisition/query errors or interruption
when no result was observed. A native result does not prove transaction disposition
for arbitrary SQL; no automatic replay, remote cancellation or arbitrary-session
reset is claimed. Transaction workflows belong in atomic runners.

Profile and atomic continuity validation share one statement. After successful
recoverable work, validation precedes the acknowledged RELEASE; no redundant SELECT follows
RELEASE. Unprofiled opening checks are omitted while ownership excludes intervening
SQL; profiled opening checks remain because schema existence/USAGE is external
state. Recovery revalidates after savepoint rollback/release, and final commit or
rollback always validates immediately before its command. No cached profile check
is a permanent authority witness.
Cancelling a polled inner operation consumes usable state, even if caught by the
body. There is no await after acknowledged completion. Every completion retires
the session; acquisition resets inherited state with ROLLBACK, SQLx cache clearing,
and DISCARD ALL. Native pool hooks are not trusted to clean arbitrary SQL effects.

`PgSessionProfile` lets `run_atomic_profiled` and `inspect_profiled` re-establish
explicit login/effective roles, trusted schema path, timeouts and custom settings
after reset, before beginning a transaction. Validation precedes callback access
and follows scope work. Setup queries run before BEGIN so snapshot inspectors can
still lock authoritative objects before their first snapshot-bearing query.
The declared login is compared with `session_user`, independently of the effective
role. A mismatch fails setup before protected application or inspection callbacks
receive SQL access.
PostgreSQL 18 is the minimum supported server for all PostgreSQL integration
paths, including both profile constructors.
`PgSessionProfile::with_timeouts` requires all four server timeout selections,
including idle-in-transaction and total-transaction limits. The adapter validates
exact bounded milliseconds before I/O, applies explicit zero as disable, and
revalidates both settings alongside the existing policy. The shared validation
also checks the login's current SET permission for the effective role. Ordinary
profiled-pool acquisition verifies this and schema USAGE without repeating the
reset performed before idle admission. It also reassigns declared custom settings
their current values in one native batch before verification, checking assignment
authority and extension hooks without overwriting drift with declared values.
Acquisition uses one policy statement without custom settings, two with them,
and never clears the statement cache. External revocation rejects that
acquisition; native fast acquisitions still skip the hook, and no check fences
later grant changes or withdraws a held lease. Profile application first uses
native `SET ROLE`, then two bound arrays for the remaining settings. Both function
EXECUTE and parameter assignment checks therefore use the effective role;
the login need not inherit its function grants. Accepted large declarations do
not become a wide SELECT target list. Unsupported parameters
fail setup. The compatibility `new` constructor leaves those two settings
undeclared and preserves their reset defaults without checking them. Startup
options survive `DISCARD ALL`; subsequent session `SET` values do not. Server
timeout termination cannot release a held Rust lease or cancel arbitrary callback
work; context-owned runners retain cooperative local deadline responsibility.
Setting keys are ASCII-lowercased before validation and duplicate detection;
values are preserved. Public profile reset/setup failures carry a `SqlxFailure`
inside `sqlx::Error::Configuration`, so default formatting and SQLx's pool-hook
error logs omit native contents. The native cause remains deliberately accessible
through the payload and its error source. This does not protect independent
query/notice logs, error-chain reporters or PostgreSQL server logs.
Arbitrary SQL can cause effects before revalidation; profiles do not sandbox SQL.
The Runledger runner requires `RunledgerDatabase`; ordinary runtime SQL uses its
mandatorily profiled pool, not an independently configured native pool.

Runledger's initial intent scope is consumed into a queue scope with no recording
method. Its named API therefore cannot enqueue then record an intent. Arbitrary SQL
against internal tables remains an explicitly lower-level escape hatch.

Read-only inspection has a distinct capability and verifies its private guard on
both success and failure before claiming rollback. These APIs do not sandbox
arbitrary SQL, external side effects, malicious guard manipulation, or process
death. The explicitly `low_level` consuming owner leaves output/completion pairing
to its caller and is not equivalent to the canonical runner.

`PgLease` detaches and drops its client unless `with_connection` receives an
application `Ok` and successfully
executes `ROLLBACK` on that exact connection. Only then does the private
pool-return proof exist; there is no direct pool-return call, so a failed,
interrupted or dropped unit of work cannot reach the pool. The consuming
`with_retiring_connection` path retires for every outcome. Both closures receive
an opaque low-level `PgSession` whose executor permits SQL but has no typed
transaction-start method or atomic-result guarantee. The old public
`PgTransaction` protocol is removed. This wrapper exposes no
`DerefMut`, `AsMut`, a native connection, or a native transaction, so safe
application code cannot replace the physical connection before the lease disposes
it. Native SQL can still issue raw transaction control; the return-time rollback
synchronizes both open and failed raw transactions without pretending to reset
arbitrary session settings, session advisory locks or prepared transactions.
Keep the lease inside the future whose interruption should retire it. Panics propagate;
Rust's default panic hook can still print payloads. No native error contents are
added to adapter diagnostics, but trusted source inspection and upstream logging
remain application-owned. `run_atomic` is the canonical acknowledged-disposition
path; `low_level::PgAtomicTransaction` is the exceptional explicit owner.
The low-level session has no typed transaction lifecycle, and raw transaction-control
SQL carries no typed atomic-result guarantee. If the return-time cleanup reports
failure, `with_connection` preserves the application `Ok` but retires its
connection. Cancellation drops the future, so the generic lease cannot return
an arbitrary application value; it still retires the connection. A
disposition-sensitive consumer must retain any acknowledged commit result or
rolled-back failure outside that cancellable future, as the reference
submission state machine does.

The optional verification module owns a `PgLease` acquired from the supplied
serving `PgPool`. One `OperationContext` covers acquisition, raw-state reset,
repeatable-read read-only inspection, rollback and disposition. It never borrows
an application transaction. Only a complete operation with acknowledged rollback
permits pool return; native failure or interruption retires the lease. The
library preserves session options and current role, recovering the initially
authenticated login transaction-locally for the excess-authority audit. Masking
session authorization cannot conceal recoverable superuser authority. A trusted
transaction-local search path prevents application-defined overloads from
redirecting catalog helpers; rollback restores the serving path.

Executable verification input is valid by construction. Mutable generic
authority configuration exists only as `AuthorityPolicyBuilder`; `build` (or
`TryFrom`) checks aggregate and per-list capacities, every object/privilege
pair, duplicate identities, conflicting grant-option declarations, and required
privilege contradictions before returning an immutable `AuthorityPolicy`.
Set-like discovery schemas, allowed roles and required unsupported surfaces are
sorted and deduplicated. Identical privilege atoms within one allowance and
identical legacy PUBLIC grant atoms normalize to one entry; conflicting grant
options fail. Keyed object/column declarations, exact PUBLIC allowances and
required object/privilege pairs reject duplicates. Relation targets (including
column parents) and sequence targets share a checked physical-identity index, so
one schema-qualified name cannot compile as both kinds. A later catalog mismatch
does not invalidate that compiled value: the declared kind produces a
`MissingObject` or `MissingPrivilege` finding, while a separate internal catalog-evaluation policy
applies discovery defaults to the one observed kind. The runtime never combines
the expected and observed kinds into one `AuthorityPolicy`. That internal view
checks catalog identities and appends normalized defaults during the shared
checkpointed traversal; it does not synchronously re-run external-draft
normalization or construct a second whole-policy index after traversal.
Generic migration policy construction likewise rejects duplicate versions,
oversized checksums, and oversized combined required/allowlisted sets.
`VerificationPlan` has no
empty/default constructor: each constructor selects one validated component,
and fallible composition rejects a second authority, migration, or schema
component. Composition also rejects a migration ledger declared as a sequence
by its authority component, in either construction order. The executor accepts
only that plan and performs no caller-policy
validation after polling starts. Raw nested declaration structs remain
configuration DTOs and are not independently executable.

Combined and migration-only verification take an ACCESS SHARE ledger lock
including inherited descendants before any snapshot-taking SELECT. Conflicting
DDL waits; ordinary migration row writes remain possible. The shared executor
then proves that the relation visible under the requested name carries that
backend lock. For an ordinary, RLS-free ledger with the common history columns,
it then opens a transaction-local cursor whose declaration plans the quoted row
source while retaining the proven OID as a numeric constant; policy-controlled
names are never copied into SQL string literals. The executor records its
relation locks before the declaration and rejects any newly acquired non-catalog
lock outside the protected heap, indexes and TOAST relations. This catches an
empty replacement as well as one returning rows. The later bounded history fetch
also requires every returned row's `tableoid` to match the locked OID. A
schema-renamed-and-replaced name before the cursor opens therefore returns
Incomplete/UnprotectedMigrationLedger, while a successfully attested portal
continues to address the relation planned by its declaration. The protected
SQLx subset path also
returns that incomplete result when an absent lock is followed by a visible late
ledger; the legacy path retains its MissingMigration violation. Ledgers with
parents or descendants in that snapshot return Incomplete/InheritedMigrationLedgers.
Supported standalone ledgers use ONLY so a later attachment cannot change the
read relation set. Enabled ledger RLS is a finding, and local
`row_security = off` prevents silently filtered reads without granting bypass.
`verify_migrations` covers only the ledger; `verify_authority` omits the ledger
read and lock. The focused helpers and plan-based entrypoint share the same execution and retirement
rules. They never execute DDL, take a migrator advisory lock or repair ACLs.

The validated plan components use that same executor. SQLx exact and
installed-subset manifests require the six SQLx 0.9 columns with `pg_catalog`
type identities, all NOT NULL, and an exact `version` primary key. Subset means
set membership and permits absence only when the captured snapshot also sees no
relation. Their cursor-backed history read has the same retained-OID and
`tableoid` checks as the legacy ledger path. Scoped schema inspection compares
every SECURITY DEFINER routine's
stored `search_path` entry exactly while accepting unrelated settings within the
same fail-closed UTF-8 byte bounds; every selected schema must exist. Routine bodies
and trigger execution remain unsupported. A compiled exact role may opt
into login-rooted current-database ownership denial using the database owner and
database-local `pg_shdepend` owner addresses. Unknown owner classes remain
coarse findings; unreachable well-formed owners do not consume the relevant-row
bound. Unknown, malformed, impossible, or missing role-reference forms,
inconsistent database ownership or membership endpoints, bounded relevant
catalog overflow, and reachable pinned-role capabilities other
than implicit `pg_database_owner` are incomplete
because PostgreSQL does not retain ordinary dependency records for pinned roles.
PostgreSQL forbids stored membership grants into or out of
`pg_database_owner`; observing such a catalog row is also incomplete rather
than being treated as the implicit current-owner edge.
Other databases and application-specific ownership semantics
remain outside the claim. All protected fragments share the authority
evaluator's visit, finding-count, retained-byte, and cooperative-yield budget.

Authority checks cover authenticated-login-rooted SET/INHERIT/ADMIN potential,
including target-specific ADMIN and CREATEROLE management of ordinary roles,
separately from `required_privileges` of the current role through
direct/PUBLIC/INHERIT grants. An ADMIN-only membership in a superuser role is not
promoted: PostgreSQL requires an already active superuser to grant or revoke
membership in that target. A separate SET path to a superuser remains visible as
superuser authority, and an INHERIT path still contributes that role's ordinary
object ACLs.
Bounded discovery can cover every supported object in selected user schemas;
per-kind defaults and exact role/PUBLIC overrides remain application policy.
Discovery preserves each selected array/multirange identity while resolving
its effective source ACL, even across the schema boundary. Shared ACL data
does not merge exact policies or erase coverage. Qualified finding names quote
each identifier component, including routine argument identities.
Relation ownership is reported at the relation rather than repeated for every
column; independently granted column privileges, grant options and PUBLIC
authority remain distinct findings. Unlisted relation, column, sequence, routine
and ownership authority is therefore
inspectable without an application-maintained second ACL engine. Required
privileges do not prove schema visibility unless USAGE is also required, nor do
they prove RLS-visible data, accepted parameter values or function behavior.

`ExactRoleManifest` is a pure, bounded compiler into that existing authority
policy. Required-and-provisioned declarations become current-role requirements,
role allowances and normalized grant-plan atoms; allowed-only declarations and
all verification ceilings produce no SQL. Exact PUBLIC allow/deny choices remain
separate from role delivery. Compilation normalizes input order and identical
duplicates, rejects conflicting declarations and invalid object/privilege pairs,
and applies both pre-expansion and low-level policy capacity checks. `GrantPlan`
only renders deterministic, quoted, role-targeted PostgreSQL `GRANT` text. It
does not connect, execute, create roles, revoke grants, wrap transactions or prove
that rendered SQL was applied. Database grants require an explicit validated
database identifier; routine grants retain structural scalar/array overload
identity and use `ON ROUTINE`. Column groups require an explicit parent relation
group, preventing discovery defaults from silently choosing that relation's
ownership, row-type or PUBLIC behavior. A column PUBLIC deny that contradicts a
parent-relation PUBLIC allowance for the same privilege is rejected because the
relation grant already reaches the column. Rendering rejects PUBLIC/NONE and the
reserved `pg_` role namespace; arbitrary application role membership remains
caller-owned policy.

`GrantFragment` is the additive, object-only half of that manifest. A library that
owns PostgreSQL relations publishes the schema, relation, column and routine
declarations its own statements need; it cannot carry or replace the
current-database declaration, the discovery scope or its defaults, role-attribute
ceilings or the current-database ownership guard. Those stay application-owned,
and `FragmentObjectPolicy` carries the application's explicit choices for the
fragment's own objects: the schema privileges, whether a reachable role may own
the schema or its relations, exactly which privileges PUBLIC may deliver, and
whether PostgreSQL's default PUBLIC USAGE on composite row types is retained.
Every builder consumes its value, and `ExactRoleManifest::with_fragment` consumes
the manifest, so a rejected declaration or a capacity overflow leaves no partially
extended value and no executable partial grant set. Fragments reach the one
existing compiler and renderer: overlapping and reordered selections normalize to
one deterministic plan, and contradictory purposes, PUBLIC choices or row-type
options are rejected before any policy or grant plan exists.

`GrantFragment` itself constrains which policy a declaration may carry, not which
purpose or grant option it names: its builders accept any declaration the manifest
accepts, including an allowed-only ceiling or a permitted grant option, because a
library may legitimately publish one. The narrower contract belongs to
`FragmentObjectPolicy`, which is the only way the native producers build their
declarations: every declaration it emits is required and provisioned, never
permits a grant option, and takes its PUBLIC delivery from the exact privileges
the application named, so permitting PUBLIC to deliver one privilege never widens
another. That allowance is also per declared object: where a fragment declares a
privilege only on exact columns, a whole-relation PUBLIC grant reaches columns it
never declared and stays excess authority. A library that bypasses it and calls the fragment builders directly keeps
whatever purpose and grant option it declares.

`runledger_postgres::grants` and `batter_runlimit::grants` publish the
operation-to-object inventory of their own source version through that fragment.
Selections name operations, never application role names, and there is no
administrator preset. The inventories describe this version's statements; they are
not evidence that the native schema is installed, that a remote effect occurred,
or that an application's authorization policy is correct, and rendering grants
cannot remove privileges a role already holds. Required `UPDATE` and `DELETE`
privileges remain arbitrary SQL authority over those rows: row-locking `UPDATE`
columns are ordinary mutation authority, `job_enqueue_intents.enqueue_request`
holds submitted request data, releasing an unstarted claim deletes its
`job_attempts` row, and `MAINTAIN` on the catalog relations also permits
`VACUUM`, `ANALYZE`, `REINDEX` and `CLUSTER`. A column-restricted fragment bounds
which columns a login reads; it is not a payload-secrecy, row-level, tenant
isolation or history-protection claim. No Runlimit selection needs
relation-level `SELECT`: fixed-window expiry cleanup identifies expired rows by
primary key rather than by the `ctid` system column, whose read PostgreSQL checks
against relation-level `SELECT`, so a cleanup login cannot read a counter's usage
or policy and scope metadata. Runledger's full-schema snapshot is a
separate relation-level selection because PostgreSQL checks `LOCK TABLE` against
relation privileges, and the direct-job requirements support a direct-only
workload: workflow-linked rows reach the same hooks and reaper, so they establish
neither workflow isolation nor mixed-workload support.

The 10,000-entry bounds apply independently to retained manifest input, raw
group expansion and the generated low-level policy. Each generated exact PUBLIC
override and required-privilege row is charged before policy construction, so a
manifest below its retained-input limit can still be rejected without producing
a partial compiled value.

Captured evaluation yields every 64 object/role visits under the same owned
operation. It rejects more than one million visits, 100,000 findings or 16 MiB
of finding payload with `EvaluationCapacity`, followed by unsuccessful client
retirement. Policy validation bounds original nested input to 10,000 entries
and each privilege list to 32 entries. These limits do not preempt native SQLx
polls, OS scheduling, allocation or destruction. Idle checkout normalization
emits PostgreSQL's expected ROLLBACK warning through SQLx's native notice target;
raw empty, written and aborted transaction recovery remains mandatory.

Migration rows and checksum bytes are capped before client materialization.
Catalog queries, including ACL expansion, reject a 10,001st row instead of
returning a partial report; parameter names have a 1,024-byte capacity and
case-normalized comparison keys. Parameter catalog assembly indexes names across
ACL, visible-settings and selected-name sources; declared missing-object checks
index known or hidden captured names once. Captured parameter objects can enter
evaluation only through the private catalog boundary, which retains at most one
object for each normalized name; tests reject a conflicting duplicate fixture.
Authority-policy validation builds one checked declaration index for relations,
columns, sequences, schemas, routines, types and parameters, plus indexed schema
discovery and PUBLIC declarations. Duplicate validation and required-allowance
resolution share that index; the database allowance remains a singleton field.
The all-kind scale regression uses hundreds of distinct required lookups per
keyed kind, with separately required nested columns, and rejects counted scan
mutations of each index. It also checks the exact input-capacity boundary.
The number of logical index visits is therefore linear in the bounded policy and
catalog inventories, while hashing still examines the bounded name bytes.
Parameter scale regressions count catalog traversal and name decisions at private
production boundaries, including repeated traversal or index construction. They
do not prescribe standard-library hashing, allocation or rehash schedules.
Separate full-evaluator scale fixtures exercise ordinary required authority,
PUBLIC observability/existence and declared existence lookups. Each has an exact
policy-capacity control and retains explicit missing, denied and incomplete
verdicts; independently restored caller scans fail their work assertions.
Policy and captured-parameter lookup callers now use private collection wrappers
that expose indexed operations in production. Their test-only traversal records
each visited entry, so replacing a caller's lookup with a map/set scan is visible
to the oracle. PUBLIC override conflicts and schema-discovery membership have
separate scale controls, including late conflicts and out-of-scope requirements.
Declared absent custom placeholders retain
implicit SET authority; loaded extension parameters retain visible context.
An unobservable requested parameter produces `ParameterUnobservable` and
`Incomplete`, retaining other findings. Hidden metadata cannot become a fabricated
missing-object or missing-privilege verdict. A privileged boolean-only existence
probe can distinguish absent built-ins, but cannot supply hidden parameter context.
For an absent custom definition, conservative potential SET authority does not
prove a required privilege: required custom targets also produce Incomplete.
Reserved prefixes and hidden extension definitions remain indistinguishable from
ordinary placeholders without observable context; verification never probes SET.
These are finite retained-input limits, not a server execution-memory bound.
Cluster-wide role, membership or parameter ACL growth can reject even a small
policy; the checker never turns a truncated catalog into a passing report.

`VerificationReport` distinguishes policy violations from incomplete coverage
and typed operation/native failures. Its coverage identifies the actual stage;
supported and unsupported surface slices are unique and use their enums'
declaration order independently of plan composition. Catalog identity failures
retain the exact typed `PolicyError` source while their Display/Debug remain
fixed and redacted.
Requested unsupported surfaces produce `Incomplete`. Function bodies and trigger
protocols, extension semantics, other-database ownership, and application durable
schema rules remain application/native checks. Future sessions,
grants or schema changes are not certified. See the [adapter contract](../crates/batter-sqlx/README.md#read-only-schema-and-authority-verification).

Retirement releases local pool capacity without awaiting interrupted SQL or
SQLx's pool-return ping. It does not prove remote cancellation, rollback or server
session termination. Detached sessions may outnumber max_connections and survive
completed Pool::close. The live regression separately proves local replacement
and close while locks remain held, then session disappearance after unlock.
Repeated interruptions record independent residual sessions, not a remote bound.
Ordinary successful return retains SQLx's asynchronous health-check policy.
Neither operation interruption nor native failure classification authorizes replay.

## HTTP boundary

`HttpBoundary` assembles probes, admission, correlation and the observer in one
library-owned order, and is the canonical path. The deliberately caller-ordered
middleware and registration helpers live only in `batter_axum::low_level`,
reachable as `batter::axum::low_level`, with no crate-root aliases: an import of
`observe_http`, `request_admission`, `request_scope`, `operational_http`,
`operational_http_with_quota`, `readiness`, `liveness`,
`dependency_readiness`, `register_http`, `register_http_in` or
`register_http_with_connect_info_in` from the crate root does not compile.
Policy and value types, `AdmittedRequest`, `CorrelationId`,
`render_infrastructure_failure`, `readiness_status`, `default_readiness_level`,
the browser transport primitives and `quota_observation` keep their existing
supported paths, because the canonical path uses them too. Choosing `low_level`
takes on the ordering, placement and lifecycle obligations each helper
documents; none of what it allows is reachable through `HttpBoundary`.
`low_level::request_admission` applies the combined readiness/deadline
`RequestPolicy` and records the admitted request that handlers extract as
`AdmittedRequest`, described below. `low_level::observe_http` independently
observes response construction without lifecycle state, a deadline or a context
extension. The existing `low_level::request_scope` combines those behaviors for
compatibility. `HttpBoundary::with_probe_response_policy` selects a
`PrivateResponsePolicy` for every probe response, including the method
rejection that answers an unsupported method on a probe path before any
renderer runs. A probe sits outside every route group, so a group's
`BrowserPolicy` never reaches it, and without this selection those `405`
responses carry no `Cache-Control`, `Referrer-Policy` or
`X-Content-Type-Options`. The boundary installs the policy around the probe
routers during assembly, so a private administrative surface needs no
post-assembly layer and keeps the sealed `AssembledHttp` through registration.
The selection changes response headers only: probe status, the readiness
decision, the rejection's `Allow` header, correlation, the single observation
and the rule that probes stay outside application admission are unchanged, and
it applies to empty-body and application-rendered probes alike. Header values
are set rather than appended, so a renderer applying the same policy leaves
them single-valued. It is opt-in and covers probes alone: without it probe
responses are exactly as before, unmatched paths remain the guarded or
rendered fallback's concern, and guarded routes keep their own group's policy.
`HttpBoundary`'s probe methods accept only opaque `ProbePath`
values; captures, wildcards and other non-literal route syntax are rejected
before a route can be mounted outside admission. Reusing a path across liveness or
readiness declarations returns a sanitized `ProbeRegistrationError` before
Axum routing, rather than panicking during startup. Guarded application routes
enter the canonical path through `GuardedRouter`, which retains route patterns
across route, merge and typed nesting. Opaque `nest_service` inputs cannot enter
that path because their inner route identity is unavailable. Awaited assembly
sends one inert inspection request per reserved path through a library-owned
inventory and returns `BoundaryAssemblyError` when it reports a matching route.
Application handlers, fallbacks and middleware are never polled by that
inspection, and Axum never receives overlapping routers to merge. Only the outermost observer emits an HTTP completion event;
nested Batter middleware contributes retained adapter facts to that shared
observer state, so stacked wrappers cannot duplicate observations or hide inner
quota facts. Operation events remain separate. Subscriber filtering and
transport delivery are application-owned.

Named route groups extend that order without changing it. `HttpBoundary::new`
takes the default group's `GroupPolicy`, into which a `RequestPolicy` converts,
and `with_group` adds a `RouteGroup` with its own `RequestPolicy` and optional
`BrowserPolicy`. Correlation and the single observer stay outermost. Each group's
routes then receive, from the outside in, the group's `PrivateResponsePolicy`
headers when it has a browser policy, lifecycle admission with the group's
response-construction deadline, `MutationPolicy` checks on every method except
GET, HEAD, OPTIONS and TRACE when configured, together with the credential
decision that `BrowserCredentials` reads, and the application's own layers.
The private headers therefore cover that group's admission, deadline,
method-fallback and mutation rejections. A mutation rejection is rendered by the
application's renderer from the sanitized `MutationRejection` and the admitted
request's metadata, without the body. Only the default group, the routes passed
to `assemble`, may declare root or nested fallbacks, and it is merged last, so
by default every unmatched path reaches its fallback inside its admission,
including paths under a named group's prefix. `with_group` rejects an invalid or
reused name (`default` is reserved), a group without routes, and any fallback,
before routing. Awaited assembly rejects a probe path that any group's route can
match (`GuardedProbePath`) and two groups whose route patterns can match one
request path with any methods (`OverlappingGroupPaths`, naming both groups). The
overlap decision constructs a shared path under the pinned matchit 0.8.4 rules:
a capture may be empty except in the final segment, a wildcard needs a
non-empty tail, and `{{`/`}}` are literal braces. Assembly confirms that path by
routing it through an inert router for each pattern alone, without polling
application handlers, fallbacks or middleware. If a request URI cannot preserve
the shared path verbatim, assembly conservatively rejects the overlap before
native merging; unreachable route literals can still collide during that merge.
These are local routing and
header mechanics: browser signals are not authentication, authorization, CORS
or complete CSRF protection, and a group budget bounds response construction
only.

Routers built outside `GuardedRouter` join the canonical path only through
`GuardedRouter::from_router` with a `RouteInventory` of the route patterns they
may serve. Construction rejects an empty inventory and any pattern outside the
Axum 0.8 syntax that the overlap analysis parses. Before anything is served,
awaited assembly builds an inspection copy of each admitted router:
`Router::layer` replaces every path route, including its method fallback, every
fallback route and the catch-all fallback with a library-owned reporter, and
`route_layer` then marks the path routes outermost. Routing a request through
that copy runs Axum's matcher, which records `MatchedPath`, but calls or polls
no application handler, fallback or middleware service. Preparing the copy, like
Axum's preparation of a served router, runs the constructors of application
layers that Axum applies lazily to handlers, once, even when assembly is then
rejected. For each declared
pattern, assembly routes one path of it, with `{}` filling every capture and
wildcard, and returns `RouteInventoryMismatch` naming the group unless that path
reaches the route registered with exactly that pattern. A pattern whose path no
request URI can carry verbatim, a route inside an opaque nested service and a
pattern whose path another route takes all fail closed. Any route of the copy,
declared or not, that matches a probe path returns `GuardedProbePath`. Declared
patterns join the cross-group overlap check and must not share a request path
with the group's native routes or another admitted router
(`OverlappingRouteInventory`). An admitted router is never merged into the
native router. Instead the native router's only fallback is a library dispatch,
reached through its root fallback routes, which capture nothing: a request that
no native route matched goes through the inspection copies there, carrying no
path parameters or matched path, and is forwarded unchanged to an admitted
router, inside its group's policy and its own outermost correlation and
observer, only when its copy reaches a declared pattern. An index of the
declared patterns, built at assembly under the same matchit 0.8.4 rules as the
overlap analysis, offers the request only to admitted routers with a declared
pattern matching its path, in admission order; it may offer a router whose own
undeclared route takes the path, and that router's copy then refuses it, but it
never skips one with a matching declared pattern. Its lookups and teardown use
stack bounded by the 25-capture limit, not by a declared pattern's literal
depth. Every other request
continues to the selected fallback. Guarded root or nested fallbacks sit behind the
dispatch with their policy and observer, because a nested fallback below a
capture that matched first would add that capture to the forwarded request.
Undeclared routes and an admitted router's own fallbacks therefore never serve
a request. Assembly prepares each admitted router, its inspection copy and the
moved default fallbacks once, as Axum's make-service conversion prepares a
served router, so no request rebuilds their layers and state held by those
layers is shared by all of their requests; native routes keep Axum's own
preparation. Axum exposes no route enumeration, so the inventory remains
application input: a route it omits is unreachable rather than verified. Each
admitted router offered a request adds one inspection routing, outside the
observer. Under the analyzed rules, overlap rejection leaves at most one such
router for any path, so a request that no native route matches costs at most
one inspection routing whatever the number of admitted routers, and none when
no declared pattern matches its path.

`HttpBoundary::with_rendered_fallback` selects a synchronous metadata-only
renderer outside every group's admission, while retaining server correlation
and the single observer. It receives only unmatched paths after both native and
declared-route dispatch. Unsupported methods on a real route remain in that
route's group; probes and all inventory/collision checks retain their meaning.
Duplicate renderer declarations and a renderer combined with any declared
root/nested guarded fallback return `BoundaryAssemblyError::ConflictingFallback`.
The renderer chooses status, body and headers in every lifecycle state. Group
browser policies do not apply to it: application prefix policy may use
`PrivateResponsePolicy::apply` on its response headers. Metadata retains public
correlation but excludes the body and private admission, quota, observation and
operational ownership state. Assembly never calls the renderer. Like probe
renderers it must not block or perform business work, and carries no request
deadline or body-lifetime guarantee. Existing guarded fallback behavior is
unchanged when no renderer is selected.

Probes can carry the application's own bodies. `with_rendered_liveness` and
`with_rendered_readiness` reserve their `ProbePath` exactly as `with_liveness`
and `with_readiness` do: the path joins the same duplicate-path rejection and
the same assembly check against every group's routes, and the probe is mounted
outside every group's admission, inside correlation and the single observer.
The application renderer runs synchronously for each probe request with the
request metadata, including the generated `CorrelationId`, but without the
body or Batter's private quota writer, observation state or operational ownership
marker; a probe is never admitted, so no
`OperationContext` or `RequestInterruptionResponder` is present. The renderer
returns the response body and headers, for example an OpenAPI-documented JSON
document, but not the probe's outcome. The boundary then sets liveness to 200
and removes any `HttpObservationLevel` override from it, so its completion
keeps the empty-body probe's default INFO. It sets readiness to
`readiness_status` of the one fresh decision it passed to the renderer and
replaces the `ReadinessDecision` and `HttpObservationLevel` response extensions
with that decision and the policy's severity. A renderer therefore cannot
report an unready process as ready, forge the decision extension or change the
completion event's severity. Assembly never calls a
renderer. A renderer that blocks or panics does so inside the probe request
like any handler; probes carry no response-construction deadline.

Guarded handlers and route layers extract `AdmittedRequest`: one value holding
the admitted request's `OperationContext`, the `CorrelationId` that its
operational wrapper generated and its `RequestInterruptionResponder`. Admission
records it privately after the readiness read, and only inside the
`operational_http` wrapper that owns the request's correlation, as
`HttpBoundary` and the protected `HttpQuota` assembly install it. The recorded
identity is the one on the response header and the completion event, even when
an application layer between that wrapper and admission removes the public
`CorrelationId` extension. The record's type is private, so no application
layer can insert, replace or remove it by type. The public value has a private
field and no constructor, and it is not `Clone`, so it can be neither
constructed nor inserted into request extensions (compile-fail rustdocs).
Extracting it where admission recorded nothing answers
`AdmittedRequestRejection`. That covers a route outside admission, a
lower-level `low_level::request_admission` or `low_level::request_scope`
without `operational_http` outside it, and that wrapper placed inside
admission. A route appended outside the boundary is no longer among them:
assembly is sealed, as described below. The rejection is the
fixed 500 Problem JSON of `HttpFailure::Internal` with `no-store`, naming no
type, where Axum's missing-extension rejection answers text naming the missing
type; its completion keeps the WARN default for 5xx. Admission still inserts
the native `OperationContext` and `RequestInterruptionResponder` extensions, and
`operational_http` the public `CorrelationId`, for Batter's adapters and
existing handlers. They are ordinary extensions that any layer can insert,
replace or remove; raw values neither make the extractor succeed nor change
what it returns. The shared renderer filter also removes the admission record,
so a request redispatched from renderer metadata is admitted only by admission
of its own. Copying a whole extension map from an admitted request into another
request copies the record with every other value: a transplant of values that
admission created, not a construction.

Observation alone does not short-circuit and may sit outside operational
correlation. Admission, deadlines and authentication can return without polling
an inner layer. Therefore a manual composition that promises generated identity
on every outcome must place `operational_http` outside `request_scope`,
`request_admission` and other rejecting middleware. The reverse order is not a
supported composition; the canonical `HttpBoundary` cannot express it.

Assembly is sealed. `AssembledHttp` has exactly two outcomes: registration for
protected serving, and the consuming `in_process()`, which returns an opaque
cloneable `InProcessClient` with `request(&self, Request<Body>) -> Response`.
The client keeps its router private, with no router conversion or accessor, no
`Deref`/`AsRef`, no public field, no route or layer method, no Tower
`Service`/`MakeService` implementation and no serving conversion, so no layer or
route can be added outside the observer and neither value can be handed to
`axum::serve`. One compile-fail control per escape rejects each of those,
including the removed `into_router`, a `tower::Service<Request>`
implementation, an `Into<Router>`, `AsRef<Router>` or `Deref` implementation
that would reintroduce the router by bound, and reading the private router
field. The client prepares the router once, with
the same `Router::with_state(())` preparation pinned Axum 0.8.9 performs inside
`into_make_service` and `into_make_service_with_connect_info`, then clones that
prepared router per request, so a layer wrapping a lazily built endpoint is
constructed once; make-service stays unexposed. A request is polled, and its
future destroyed, inside the caller's task under the boundary's existing
dispatch and observation ownership, with nothing spawned, so dropping a
`request` future drops the boundary's future as a dropped connection would. A
request may carry explicitly inserted synthetic extensions, such as a chosen
`ConnectInfo`; choosing them and their meaning belongs to the caller, and this
constrains direct API conversions, not an application's own transport wrapper or
remote peer authenticity. An in-process response establishes response
construction only: it is not evidence of serving, an accepted connection, a
listener, a TLS handshake, a remote peer, body streaming or a stopped detached
descendant. Because the boundary's own observer is outermost and the plain
observer allocates no quota record, a boundary request carries no quota writer
at all; a composition that needs an outer quota wrapper belongs entirely in
`low_level`, as the protected `HttpQuota` assembly does.

`ResponseConstructionBudget::new` validates the positive bounded duration once.
`RequestPolicy::new` consumes that witness and is infallible; it has no raw
`Duration` overload. The witness proves only local duration representability, not
body transmission, remote cancellation, or completion of detached work.
The policy also consumes only `OperationAdmission`, never `ShutdownHandle`; it
can admit a process-cancelled context but cannot mutate lifecycle state.

HTTP completion events carry normalized method, matched route template (or
`<unmatched>`), actual numeric status when a response exists, HTTP outcome and
construction latency as event fields. With `operational_http_with_quota` (also
installed by the protected `HttpQuota` assembly), the completion additionally
carries `quota_outcome` and `quota_consumption`. The outcome is one of
`not_checked`, `unresolved`, `allowed`, `shadow_denied`, `quota_denied`,
`storage_capacity`, `other_denial`, or `backend_failed`; consumption is `consumed`,
`not_consumed`, or `unknown`. Allowed means consumed; an unresolved check means
unknown; a backend failure retains its native certainty; the other outcomes mean
not consumed. These facts are independent of final HTTP status, including a
timeout after admission. The pinned native Runlimit adapter does not emit
`other_denial`; that value remains available to manual observation writers.
On the protected quota path, the runner owns the observation writer. Starting a
native check consumes the unstarted writer; publishing its native result consumes
the started writer before work can begin. Only terminal native results enter that
publication path. Dropping or unwinding a pending check leaves unknown consumption;
a later handler panic, timeout or cancellation cannot erase an already recorded
grant. Authentication failure leaves the record unstarted and the body unpolled.
The quota wrapper publishes its fresh record into the outer observer's shared
state in either supported operational-wrapper order.
Ordinary `operational_http` allocates no quota record
and its completion omits these quota values. The observer retains these facts separately
from its INFO span, so disabling that span does not remove fields from an enabled
completion event. The span carries method, route, status, HTTP outcome and latency
for nested context; quota fields exist only on the completion event. Application-owned
correlation in other spans remains subject to those spans' filtering.

At first poll, observation retains its enabled HTTP span or the current enabled
application span for execution and completion parenting. Later ambient spans
cannot replace the completion event's parent; if neither was available, that
event remains a root event. HTTP fields are never recorded into the inherited
application span. Per-layer subscriber filters can still hide retained context
from an individual sink; this is not an exporter delivery guarantee.

Completion events default to WARN for 5xx responses and INFO otherwise, including
probes. `HttpObservationLevel(tracing::Level)` in response extensions selects a
different event level in both observers. It does not change response status,
HTTP outcome, field sanitization, the INFO span level, or event count. A 503 at
INFO still has `http_outcome="server_error"`; status/outcome alerts need their own
probe policy. DEBUG/TRACE events require subscriber configuration that enables
them. This is event severity selection, not guaranteed delivery or suppression.

Handlers, failure renderers or middleware inside observation must explicitly
attach the override to the returned response. It remains in the response's
extensions for other middleware and is not serialized as a header. Middleware
replacing a response/status owns retaining, replacing or removing its override.
Only the outermost Batter observer emits and reads the retained override on the
response it returns. Nested Batter wrappers publish their adapter facts into the
same private observation state instead of producing another event. Middleware
outside the outermost observer can subsequently rewrite the response without
changing that already completed observation.
Request extensions, client headers, route names and missing OperationContext do
not infer severity. Status-only probes do not add overrides. A future destroyed
without returning a response remains WARN, even if the handler constructed an
annotated response internally. No policy callback runs from the guard's Drop.

Assemble the application fallback with the guarded routes, apply admission over
that complete guarded router, then merge only validated literal probes outside
admission before applying observation with `Router::layer`. Axum runs that layer
after routing, so matched route
**templates** are available; raw paths, queries, headers, bodies and error contents
are not recorded. Nonstandard methods normalize to `OTHER`; absent route metadata
uses `<unmatched>`. A wrapper outside routing lacks that metadata at entry even
for a matching route. Routes appended after `Router::layer` bypass it. Place
trusted identity outside observation and rejecting/status-changing middleware
inside it to observe their returned status.

A polled response future emits its actual response status/outcome and construction
latency on completion, or `dropped` without a status if destroyed before a response.
The first-poll subscriber protects full future destruction, including the
observation guard and nested instrumented spans. Never-polled entry points do no
application work and emit no completion. Dropping the returned body afterward
emits no second HTTP completion and is not classified as a dropped request.

Handler panics propagate through this middleware; `HttpFailure::Internal` is not
an automatic panic catcher. Process-task and cleanup-hook panic observation does
not establish HTTP recovery or redact Rust's default panic-hook output.
Dedicated tests verify an unwind before a response propagates as a Tokio task
panic, cancels admitted context, and emits one WARN `dropped` HTTP event without
a status or panic payload. This covers Rust unwinding, not aborting panics.

The admission point is the readiness read. A request racing drain may be admitted
when that read sees Ready. Its admitted `OperationContext` is tied to forced
process cancellation, not immediate drain. Server-side duration is fixed
by RequestPolicy; the middleware trusts no client deadline or proxy metadata.
With the documented `Router::layer` composition, admission wraps guarded routes
and their default, custom, nested and method fallbacks: unsupported methods and
unmatched application paths return 503 while Starting/Draining, and their normal
405/404 responses while Ready. Only explicitly configured literal probes bypass
admission, and all responses remain observed.

The timeout ends when Next returns a Response. It does not bound streaming body
polls, WebSockets, an upstream Tower queue, or slow upload behavior occurring
outside this layer. An escaped request context is cancelled at response
construction, so a body that outlives its handler needs the distinct owner
below.

`AdmittedRequest::into_streaming_response` consumes the admitted request and
gives the body of the built response a lifetime of its own. Its `BodyScope`
context starts from the request's admission through a private
`OperationContinuation`, so no second readiness decision is made and a body that
begins during drain is not refused. That context is cancelled by process forced
cancellation and when the body ends or is destroyed, including when the builder
panics, and never by response construction; a clone kept beyond the body
observes cancellation. The authority to start a body belongs to the request,
not to one extracted value: every extraction shares one take-once slot, and the
slot is honoured only while the request's own context is live, that is, during
response construction. A second call, or a call on a request the application
kept after its response, builds the response with a scope that is already
cancelled and never starts work that drain admission would refuse. A body with
content ends at its first poll, truncated, and every refused start is reported
`cancelled`, including an empty body that the transport learns has ended from
`is_end_stream` and never polls. `BodyBudget` requires a positive total and a
positive idle bound no longer than it. The library enforces both without the stream's
cooperation: the total bound and an armed idle wait are checked before every
frame, so an always-ready source cannot outrun them and a frame that became
ready only after its wait expired does not extend the body, and a pending body
registers for forced cancellation and the nearer bound. The idle wait starts at
the body's first pending poll after a frame, so time the transport spends not
polling a body it cannot yet write is not counted. Each of the three ends the body with an error, which
Hyper turns into a closed connection without a final chunk; a library-ended
body is never reported to the peer as complete over HTTP/1.1. An HTTP/1.0
request is the exception: Hyper answers it in HTTP/1.0, where a body without a
declared length is delimited by closing the connection, so that peer receives
an orderly end of stream after the data already written and cannot tell the
truncation from the message's end. A declared `Content-Length` keeps the
shortfall visible to it. Ending at drain is left to the
stream through `BodyScope::shutdown`.

The body is polled, and asked for its end and size, under the tracing
dispatcher current when the handler handed it over and inside the request's
span context, whichever task asks. The application's body and that span are
also destroyed under it: closing a span releases its parent through
the current default dispatcher, so releasing either on the transport's task
unprotected would reach another registry. The request's observer publishes
that span to the body only while the observer is alive and releases its copy
when it finishes, so an admitted request or request extension kept beyond the
response holds no span. An owned body holds its own copy until it ends, so the
`batter.http` span closes when the body ends and span-close timing, including
exported span durations, covers the body, while `latency_ms` and the boundary
completion event still measure response construction only. Exactly one
`HTTP response body finished` event carries
`request_id`, `bytes`, `duration_ms` and `body_outcome` (`completed`, `failed`,
`deadline_exceeded`, `idle_timeout`, `cancelled`, `panicked`, `dropped`),
emitted before application destructors run. A stream error, either bound,
cancellation and a panic are WARN; completion and `dropped` are INFO.
`dropped` names a body discarded before its end by whoever held it, not ended
by the library, a panic or the stream: a peer that went away, a HEAD response
whose body Axum strips, or a response discarded because response construction
ended without it; the request's own completion event, with its method and
status, tells these apart. `panicked` names a builder that never returned, a poll of the body that
unwound (the transport destroys such a body after catching the unwind), a
question about the body's end or size that unwound, whether the owner asked it
at the hand-over, the transport asked it, or the owner asked it while
destroying the body (the owner owns the body before its first question, so it
still finishes under the captured dispatcher before the application's body is
destroyed, and the panic then continues), or destruction
during unwinding, such as a handler panicking after the hand-over; it carries
no payload. A body destroyed after its scope was cancelled or
expired reports that reason instead of `dropped`, and a source that produced
its whole message is `completed` even when the transport never polls for its
end: Hyper stops once `is_end_stream` reports the end, once the advertised
`Content-Length` or exact size has been written, and after trailers. The
advertised length is taken again from the response request admission returns,
because Hyper frames the message by the headers it encodes and Axum applies a
handler tuple's headers after `into_streaming_response` returns. The started
body hands its framing to request admission through the admission record, not
through the response, so a response the handler rebuilt around the body, which
keeps nothing else the built response carried, is settled the same way; a
layer outside admission that changes the length is not observed. A source
that yields its end, reports it through `is_end_stream` or yields trailers
before producing its advertised length is `failed` (WARN): Hyper aborts that
message and the peer sees it truncated. Hyper, not the owner, enforces the
framing. A response Hyper writes without a body (to a HEAD request, a 2xx
answer to CONNECT, or a 1xx, 204 or 304 status) frames nothing, so a length it
declares is not held against its body: an empty body there is `completed`. Error contents are not
copied into the event. The request's own completion event is unchanged, and no
metric is recorded for bodies.

[In-process tests](../crates/batter-axum/tests/streaming/) on a paused clock
cover steps that run after the handler returned beside the unowned control,
the escaped context, both bounds and their restart on each frame, a frame ready
only after its idle wait expired, an always-ready source, a parked frame woken by forced cancellation, drain as the
stream's choice, a body started during drain, a stream error, a builder
panic, a handler panic after the hand-over and a panicking stream, the event
before application destructors, an admitted request kept beyond its response,
a second and a late body start for one request, a refused second start that
leaves the length declared after the hand-over to the body that started, a
refused empty body that is never polled, a HEAD request, a declared length on a 304 and on a HEAD
response, a body whose answer about its end panics during destruction and when
the transport asks first, a body whose first answer about its size panics at
the hand-over, its end and size answered under the handler's subscriber and
request span when a foreign subscriber asks, and a
response-construction timeout after the hand-over, a dropped and a never-polled body, server-sent events and a sized body, and a
foreign subscriber on the polling task.
[Real-socket tests](../crates/batter-axum/tests/streaming/sockets.rs) through
the registered server show a pending owned body ending itself at forced
cancellation before the server's forced close, a stalled reader ended only by
that close and reported `cancelled`, a departed peer reported `dropped`, and a
stream that produced its declared `Content-Length` and one that ended with
trailers, neither of which yields its end, each reported `completed`, and a
source that ended or reported its end five bytes short of a declared ten,
each reported `failed` with the peer's message truncated, a length a handler
tuple declares after the hand-over, or a response rebuilt around the body
declares, which decides `failed` when short and `completed` when met, and an
HTTP/1.0 request whose body, ended at its idle
bound and reported `idle_timeout`, reaches the peer without a declared length
or chunked framing and as an orderly close.
Mutation checks were run by hand on 2026-11-11, each applied temporarily and
reverted; they are not an automated gate. Removing scope cancellation, the
cancellation wakeup, the captured dispatcher, the per-frame bound check, the
expired-idle check before a frame, the advertised-length completion or the
trailers completion each failed at least one of these tests, as did keeping the observer's published
span after it finishes, destroying the application's body before the event,
dropping either panic check, dropping either the take-once slot or the
liveness check on the body-start authority, releasing the request span or
destroying the application's body outside the captured dispatcher (with the
request inside a parent span), and dropping the refused-start record for an
empty body. Restoring `Clone` and a borrowing `start` on
`OperationContinuation` made its compile-fail doctests compile. In the same
way on 2026-11-12, reporting a short end as `completed` failed both
short-length cases, and holding a declared length against a response without a
body failed the 304 case, and the HEAD case on its own. Later the same day,
skipping the framing settled at request admission failed the late-declared
length case, asking the source for its end in destruction without catching
the unwind failed the panicking-answer case, as did forgetting a transport
question that unwound, and starting a continuation without checking its
admitted operation failed the core unit test. On 2026-11-13, asking the
body for its end or size outside the captured dispatcher failed the
foreign-subscriber query case, asking for its size before owning it failed the
hand-over panic case, dropping the admission-owned framing hand-off failed the
late-declared length case, the committed extension-based hand-off failed
its rebuilt-response half, and letting a refused start hand over its framing
failed the refused-start framing case.

Limits: the bounds are observed only when the transport polls the body. A peer
that stops reading leaves it unpolled, so only the forced connection close at
shutdown ends it; there is no write-stall timeout. The owner covers response
bodies, not upgraded connections. Admission permits are not held for the body's
lifetime. An unowned streaming body still compiles, because `OperationContext`
is `Clone` and `'static` by design; the owner makes the supported composition
available, it does not make the other unrepresentable. HTTP/2 was not exercised.

A response returned from `RequestInterruptionResponder::render` carries private
interruption evidence. Admission uses it to classify the response-construction
operation before emitting telemetry, including when a nested bulkhead refuses
expired work before its factory starts and the outer timer has not fired. The
original status, headers and body are retained in one `ResolvedOutcome<Response>`
without calling the renderer again. Admission consumes the private marker before
returning; its own timer/cancellation path renders directly without producing a
handoff marker. In stacked low-level admission, an inner interruption therefore
does not reclassify an outer boundary whose own authority remains active. Both
interruption paths have the same ownership. Every stored adapter renderer returns
an opaque private output that removes recycled interruption evidence from custom
response extensions while retaining unrelated metadata. This includes request
failures, early admission rejection, browser rejection, probes and unmatched
fallback rendering; their existing status and severity policies still apply.
Only the public interruption responder attaches a current handoff reason.
Rendering and discarding a response
does not classify a later response;
an unmarked application 503 remains successful response construction. The single
HTTP completion still describes the actual response status. The operation event
retains request correlation when INFO operation spans are filtered out.

The two HTTP/1.1 lifetime targets share launch/completion evidence but retain
separate raw-wire and instrumented-IO fixtures ([ADR-008](adr/008-http-transport-ownership.md),
[ADR-009](adr/009-http-lifetime-observations.md)). They independently observe
handler/upload destruction, response observation, complete message framing, body
destruction, client EOF, native socket destruction, direct server outcomes and
cleanup. Incomplete upload reads inside a guarded handler consume the response
budget. A blocked response body remains outside it after the escaped request
context is cancelled. Ordinary drain lets admitted handlers finish; a later
keep-alive request may receive 503 or encounter connection closure. A separate
withheld-graceful case must witness admission rejection.

Both suites require native connection graceful acknowledgement before releasing
cooperative work, then retain live resources and absent server completion/cleanup
at a 50 ms held-work checkpoint. The instrumented suite additionally requires a
pending wire read. This ordering depends on resolved Axum 0.8.9 and the enforced
current-thread runtime ([testing](testing.md)); the producer signal alone is
insufficient. These are bounded observations, not indefinite survival guarantees.

Forced cancellation can drop a handler, return 503 and allow clean direct-server
joining and cleanup. Aborting the direct wrapper instead leaves a blocked body
pending through report inspection and skips cleanup for the unsafe exit. Later
body release establishes separate transmission/destruction, not a transitive join.
Full client shutdown plus socket close drops pending handler/body work in these
fixtures before test release: both suites bound their post-close resource
observations by one second. Write-half-close is a separate native-default
case. No immediate or universal disconnect propagation follows. Before response
construction the observer reports dropped without status; after headers it keeps
the original completion with no second status/event.

Both targets require scenario-specific successful completion from a PID-authorized
child; ordinary successful process exit is insufficient. Independent Unix bounds
reject deliberate stalls. Failed event assertions retain destructor evidence.
Their shared startup/exercise/teardown phase budgets fit below the parent watchdog.
Terminal report waits belong to teardown; only the intentional blocked-body abort
checkpoint observes a report during exercise. Delayed real finalizers prove that
successful teardown can outlast exercise's allowance without failing that phase.
The observation driver joins timed-out exercise work before teardown and retains
any observed shutdown report independently of later reconciliation failures.
Slow combined-phase and readiness-timeout controls test these diagnostic paths.
These are test-infrastructure contracts, not runtime-death or general async-drop
promises. No HTTP/2, WebSocket, capacity or detached-descendant joining claim follows.


Default infrastructure responses omit raw errors and use application/problem+json
with stable codes. `with_failure_renderer` can select an application envelope,
status, and headers using a request-parts snapshot taken before the handler runs.
The callback owns output sanitization and must use trusted metadata explicitly;
Batter does not trust a client correlation header for it. The callback covers
middleware failures, not application responses or separate health probes.
Default server execution exhaustion uses 503, not a claim that client
upload timed out. No automatic Retry-After authorizes write replay. Authentication,
authorization, proxy trust, body limits, and domain error policy remain
application-owned. Server correlation is separately opt-in. Liveness/readiness must be mounted outside the readiness gate.

HTTP fixture diagnostic deadlines belong to the startup, exercise and teardown
owners. Observation event/wire waits retain interrupted names, partial bytes and
event snapshots on destruction; the driver retains tracing capture and any
observed report. Fast and delayed-report negative controls require the same
teardown timeout result. Keep-alive handler counts are rechecked after terminal
shutdown, with late-entry controls proving the final checkpoint. These are test
harness contracts, not production deadline or async-drop guarantees.


### Browser credential transport

`batter_axum::browser` accepts trusted origins only through distinct HTTPS and
loopback-HTTP constructors. It requires explicit case-insensitive
`scheme://authority` input and rejects reverse solidus, credentials, non-root
paths, queries and fragments before URL recovery or path normalization can erase
them. It canonicalizes accepted values with `url` 2.5.8 and never derives
authority from Host, Forwarded, or request Origin fields. Loopback HTTP is
limited to loopback IPs and `localhost` names.

Handlers read incoming cookies through `BrowserCredentials`, which only the
gate a route group's `BrowserPolicy` installs can grant. The gate runs inside
the group's admission, before the application's layers, and decides from the
method the request arrived with: every safe method (GET, HEAD, OPTIONS, TRACE)
is granted; any other method is granted after `with_mutation_checks` passes,
or under `without_mutation_checks` with
`MutationCheckWaiver::ApplicationChecksMutations`, and refused under
`MutationCheckWaiver::NoAmbientCredentials`. A group without a browser policy,
a lower-level composition and a bare router record nothing, so extraction there
is refused on every method. Refusals do not depend on whether a Cookie field is
present, answer the fixed sanitized 500 Problem JSON of `HttpFailure::Internal`,
and are typed `UncheckedMutation` or `NoBrowserPolicy` for an application that
extracts the `Result`. The record's type is private to the adapter, an
application layer that rewrites the method cannot change the decision, and
renderer metadata never carries it. Copying a whole extension map into another
request copies the record with it. A handler parsing the raw `Cookie` field, or
calling `low_level::read_unchecked_cookie`, bypasses the gate and keeps the
forgery check itself.

Incoming cookie reads scan every Cookie field. Target names are exact and
case-sensitive; duplicate targets, non-UTF8 fields, invalid opaque target values,
and malformed target pairs fail with sanitized typed errors. `Strict` applies
Batter's narrower unquoted syntax to unrelated pairs and is suitable only when
the application controls the whole cookie jar; it rejects some RFC-valid input
such as quoted values. `TargetOnly` ignores invalid unrelated pairs while
keeping target and ambiguity checks strict. Values are borrowed without
percent-decoding, unquoting, or normalization. An empty target remains a present
opaque value; its application credential parser owns invalid/absent semantics.

`BrowserCookie` always emits a host-only `Path=/` cookie. HTTPS requires the
exact `__Host-` prefix and Secure; explicit loopback rejects browser-reserved
secure prefixes and SameSite None. `__Host-Http-` additionally requires
HttpOnly. SameSite, HttpOnly versus script-readable visibility, and session
versus positive whole-second Max-Age are explicit. Set and removal operations
append Set-Cookie fields. Removal retains scope/security attributes and emits
both Max-Age zero and a past Expires; it does not revoke server state.

Every `MutationPolicy` starts with exact configured Origin or one exact custom
marker. Marker construction rejects CORS-safelisted names, browser-controlled
names, non-printable values, tabs, and surrounding spaces that Fetch would
normalize away. Because a user agent can attach other accepted non-safelisted
names without script or preflight, every policy containing a marker automatically
requires exactly one `Sec-Fetch-Site: same-origin`; adding a marker strengthens
an existing compatible Fetch Metadata check and cannot weaken strict mode.
Missing Fetch Metadata fails closed. Marker-policy consumers therefore require
a potentially trustworthy mutation URL and browsers that emit the field;
non-trustworthy development URLs and older clients can be rejected.
Configured categories are ANDed and checked in Origin, Fetch Metadata, marker,
then JSON order with exact field multiplicity.
`RejectCrossSite` permits missing and syntactically valid future Fetch-Site
tokens but rejects explicit `cross-site`;
`RequireSameOrigin` accepts only exact `same-origin`. The optional JSON check
parses one complete `application/json` media type case-insensitively with the
RFC 9110 byte grammar, including empty parameter slots and quoted `obs-text`,
while rejecting whitespace around parameter `=`. It does not accept suffix JSON
types, malformed parameter tails, or coalesced lists. Typed rejections expose
only a stable status/code and no observed or expected values.

Set and removal operations append independent Set-Cookie fields without
replacing siblings. Before mutation they reject an existing field with the same
case-sensitive cookie name, preventing ambiguous set/set, set/removal, and
removal/set responses. The error is typed and contains no cookie name or value.

Private-response mutation overwrites `Cache-Control: no-store` and
`X-Content-Type-Options: nosniff` and sets one `Referrer-Policy` field selected
by the exhaustive `PrivateResponsePolicy`: `same-origin` (`SameOriginReferrer`,
the default and the compatibility `private_response` value) or `no-referrer`
(`NoReferrer`). Weaker, raw and comma-listed referrer policies are
unrepresentable. Status, body, extensions and unrelated headers are preserved.
Existing fields of all three names are replaced, except that an existing
`Referrer-Policy` made only of exact `no-referrer` fields stays `no-referrer`
under `SameOriginReferrer`. Nested private-response layers therefore never
weaken an inner `NoReferrer` choice unless intermediate middleware rewrites the
field.

The selected value is the document's initial referrer policy. Under
`SameOriginReferrer`, same-origin non-CORS mutations such as HTML form posts keep
the serialized origin, while a non-CORS form post to another origin carries
`Origin: null` and cannot satisfy that target's exact-origin policy. Under
`NoReferrer`, every non-CORS mutation, including a same-origin form post, carries
`Origin: null`, and same-origin navigations send no `Referer`. The `Origin` of
CORS-mode requests, the default for script `fetch`, and `Sec-Fetch-Site` are
unaffected. Page markup and `fetch` options can still change the policy; Batter
does not control page content. The layer and compatibility middleware cover
inner success, error, rejection and fallback responses; they cannot affect an
outer short-circuit that never calls them and create no observation of their
own.

These are bounded header/cookie mechanics, not authentication, authorization,
CORS, proxy trust, token generation/comparison, a complete CSRF proof, browser
history control, or server-side revocation. SameSite and Fetch Metadata are
defense in depth. `no-store` is not a sufficient privacy guarantee against
malicious caches or non-HTTP storage.

### Opt-in operational defaults

`operational_http` generates a new UUID on first poll and composes exactly one
HTTP observer inside its retained request span. It replaces all x-request-id
values and both Tower RequestId and adapter CorrelationId extensions before the
inner service runs. The response header is replaced after the service returns.
`CorrelationId` is opaque, cloneable and exposes a string for explicit metadata
propagation; it never selects authority. Inbound trace and proxy trust remain
uninterpreted. UUIDs are correlation values, not a durable uniqueness constraint.

The observer retains this typed ID as a completion-event field independently of
INFO spans, including WARN dropped observations under a different ambient
dispatch. Native nested operation tracing is still filtered normally and carries
context through enabled request spans. The INFO request span uses target
`batter::request`: `info,batter=warn,batter::request=info` retains its context
without enabling Batter INFO operation/HTTP completions. A span-name filter
can also enable nested events and is not equivalent to this static target policy.
Neither library installs a global
subscriber. Tests retain their subscriber registries across cases and check
completion fields after the event message, not only in formatted spans.

`with_infrastructure_json` opts into a fixed application/json envelope containing
exactly `code`, `message`, and `request_id`, with Cache-Control: no-store and the
HttpFailure status. Codes and messages are the sanitized literals in
`render_infrastructure_failure`; request_id is a generated string, or null when
the typed extension is absent. No inbound header fills that gap. The renderer
never formats causes. Legacy Problem JSON is unchanged; explicitly selecting a
custom renderer afterward replaces this policy. Domain responses remain owned
by their handler.

The reference application's canonical `http::register_in` operation assembles
its routes through `HttpBoundary` and registers the result with
`AssembledHttp::register_with_connect_info_in` at one application-owned
boundary. This prevents the production root from choosing direct-peer policy
and transport metadata independently; the generic adapter cannot infer an
arbitrary router's extension requirements. The boundary owns correlation, the
single observer, probes outside admission, and lifecycle admission with the
request deadline around every business route and the unmatched-path fallback.
The application installs `TrustedRequestMetadata` after admission and before
authentication, as route layers of its `GuardedRouter`; the body limit is a
guarded layer inside them. A draining or not-yet-ready process therefore returns
the admission rejection (503 `service_unavailable` in the infrastructure
envelope) before installing metadata or authenticating, including to requests
without credentials that an admitted process would answer with 401.
Authentication runs within the request's operation deadline. Unmatched paths
reach the 404 fallback only after admission, and an unsupported method on a
matched route reaches its 405 after admission, metadata and authentication.
Metadata installation accepts only `ConnectInfo<SocketAddr>` supplied by
that production native serving path or explicitly asserted by the opaque
in-process request client. It retains the IP as an opaque `TrustedPeer`; the source port is not a stable
admission identity. `Forwarded`, `X-Forwarded-For`, `X-Real-IP`, `traceparent`,
`tracestate` and client request-ID fields are ignored. Behind a proxy the trusted
direct peer is therefore the proxy. No trusted-proxy mode is implemented.
Missing native peer metadata fails closed with the shared sanitized internal
response and generated correlation.

Liveness and readiness are `HttpBoundary` probes outside the admission-,
peer- and authentication-gated business routes, so an in-process probe needs no
synthetic peer extension. Axum extractor rejections also remain native responses: malformed
JSON, invalid paths and body-limit failures are not promised the application's
JSON problem envelope. The outer operational middleware still supplies its
generated response ID header. Tests and callers must distinguish raw HTTP
responses from routes whose contract promises an envelope.

`TrustedRequestMetadata` has no public or test constructor. Production combines
the adapter-owned correlation with native `ConnectInfo` inside the application
middleware. The lower-level `http::in_process_client` awaits the same boundary
assembly and returns an opaque `InProcessRequestClient` that neither implements
a serving service nor exposes its inner router. A rejected assembly is returned
as the sanitized `BoundaryAssemblyError`, wrapped in `HttpRegistrationError` by
`register_in`. Each request requires a caller-selected synthetic peer, and
the client replaces the exact `ConnectInfo<SocketAddr>` extension itself. Axum's
extractor-only `MockConnectInfo` fallback is not observed by
middleware that reads request extensions directly.

The bearer credential remains the only reference authority input and replaces a
preexisting `OwnerId` extension before a handler runs. `TrustedRequestMetadata`
contains neither owner authority nor `OperationContext`; handlers extract the
owner, the metadata and the `AdmittedRequest` that carries the context
separately and pass each explicitly. Application success/domain/authentication bodies and
shared infrastructure bodies use the typed `CorrelationId`, so any body
`request_id` agrees with the outer generated response header. Correlation never
grants access, selects the durable owner, or extends request lifetime.
Cancellation keeps the request's selected metadata while cancelling its separate
operation context. No task-local inheritance, arbitrary-spawn propagation,
inbound trace retention, durable correlation envelope, or quota backend follows.

The foundation `ReadinessEvaluator` stores `LifecycleStatus` and an optional `HealthReader`,
reads a fresh dependency snapshot, then any application conditions, then
lifecycle readiness, and invokes no probe or writer-retaining operation. Every
`HealthStatus` is explicitly classified as
`DependencyReadiness`; Healthy is Ready, while Unknown, ProbeFailed,
ProbeTimedOut, Stale and WriterStopped carry the corresponding
`DependencyUnreadyReason`. Overall `ReadinessDecision` is either Ready or
Unready(ReadinessUnreadyReason), and `ReadinessUnreadyReason::Dependency` cannot contain a
healthy value. Ready requires configured dependencies healthy, lifecycle Ready, and every
application condition satisfied; an observed drain overrides cached health. A
subsequent transition may immediately obsolete the decision.

`ReadinessEvaluator::lifecycle_only` and `ReadinessPolicy::lifecycle_only`
construct the no-dependency case with `Infallible`: no monitor, probe task,
writer or freshness window exists. They assert no continuous dependency
observation, not healthy remote connectivity. Conditions still run before the
final lifecycle read. A configured dependency cannot be removed from an
existing evaluator or policy, and the exhaustive decision types are unchanged.

`ReadinessEvaluator::with_condition`, and `ReadinessPolicy::with_condition` in
the adapter, add an application condition named by a validated
`ReadinessCondition` (1–96 ASCII alphanumeric, `.`, `_` or `-` bytes). Its
synchronous check returns only whether the condition holds. It runs only after
the dependency sample establishes readiness (or no dependency is configured)
and before the final lifecycle read, in the order conditions were added, and the first unsatisfied condition
stops the remaining checks. That condition turns a decision that lifecycle and
dependency health would make Ready into
`Unready(ReadinessUnreadyReason::Condition(name))`. A condition is never asked
while the dependency is unready, and its answer is discarded when the final
lifecycle read is not Ready, so no condition can make an unready lifecycle or
dependency ready or replace its reason. Checks added under one name report the
same condition; clones of an evaluator or policy share its conditions. The
check must read state the application already holds, without blocking or I/O:
the evaluator adds no timeout, and a blocking check stalls the probe request
that asked it.

`ReadinessPolicy` translates that valid foundation decision. Responses have empty
bodies unless a rendered `HttpBoundary` probe supplies one, 200 for Ready and
503 for Unready, and retain `ReadinessDecision` in extensions. Starting/Draining
default INFO; Stopped, every dependency-unready reason and an unsatisfied
application condition default WARN. `readiness_status` and
`default_readiness_level` expose those adapter mappings; explicit level policy receives the complete decision, can
delegate unmatched cases to the default, and alters neither status, decision,
body nor outcome. The old `ReadinessReason` name is absent from both Axum and
the foundation so stale extension lookups fail at compilation even after an
import change; deliberate matching imports `ReadinessUnreadyReason` and unwraps
it from the decision. During supervised drain, a
stopped health writer still yields Draining/INFO; after process completion it
yields Stopped/WARN. Reading either state creates no probes. `ReadinessDecision`,
`ReadinessUnreadyReason` and `DependencyUnreadyReason` are intentionally exhaustive:
new semantic states require the corresponding API compatibility and consumer
policy review; `ReadinessUnreadyReason::Condition` is such a state, so an
exhaustive match must now handle it. The `low_level::readiness` and
`low_level::liveness` handlers keep their status-only contracts.

`low_level::register_http` transfers a bound listener and initialized Router into a
critical component. The listener is any `axum::serve::Listener` whose address
type is `Debug`: a `TcpListener`, a Unix listener, or an application-owned
listener that completes its own TLS handshakes.
The listener type is inferred without adding an explicit type argument. Existing
TCP calls, including `register_http_in::<Supervisor>(...)`, its connect-info
companion and both `AssembledHttp` methods with an explicit registration target,
retain their source compatibility and TCP function-pointer signatures.
The factory does no work before supervision starts and
acknowledges on its task's first poll; application approval and a running driver
remain necessary. Invalid or duplicate registration releases only the rejected
listener. Cancelling a borrowed wait on a retained startup finish future leaves the listener
owned. Dropping the startup owner requests drain, releases registered listener
captures before dependent cleanup, and lets observers await the completed report
on a live runtime. Dropping an unstarted Supervisor releases captures without
running finalizers.
Binding and its errors belong to owned Startup. Axum retries native accept errors
and spawns its own connection and graceful-signal tasks. Native graceful return
waits for connection completion; wrapper abortion does not join descendants.

A connection can outlast drain without limit: a response body that stays
pending, a peer that stops reading a large fixed response, or a request head
that never completes. Each of them once forced abort of the wrapper and skipped
every dependent finalizer, at any client's choice. The registered task now wraps
its listener, and every accepted connection's reads and writes fail once forced
cancellation is halfway to `ShutdownSignal::abort_deadline`, the stop time plus
the drain and cancel allowances. The first half of the cancel allowance is left
to cancelled handlers, whose interruption responses are still written whole. In
the second half Hyper ends each remaining connection in that connection's own
task. Axum counts a connection finished when its task drops its completion
receiver, which it does before the connection future, and with it the handler
future or response body Hyper still holds after the I/O error, is destroyed. So
each accepted connection's service, response futures and response bodies hold a
token, and the registered task returns only after Axum's graceful wait and the
release of every token; Hyper destroys the response body last. The wrapper is
then joined, the report is successful with `forced_cancellation` set and
dependent cleanup runs, never beside a connection's destructor: one still
running at the abort deadline leaves the wrapper to be aborted and cleanup
skipped. Each response body gains one wrapper allocation for this, and each
request a few atomic increments and decrements of one server-wide counter; a
lock is taken only when the last token is released or the server waits. The peer sees a truncated message or a closed
connection; an HTTP/1.0 peer receiving a body without a declared length cannot
tell that close from the message's end. Each I/O poll records its task's waker before reading the closed
flag, not only a poll the transport leaves pending: Hyper can wait on a response
body alone after ready polls, holding bytes of a following request or with its
read side closed, and the close must still wake that task, whose next pass
flushes and fails. That costs one uncontended lock per I/O poll and one registry
entry per connection. A component that reveals an earlier stop time moves the
abort deadline earlier after cancellation was observed;
`ShutdownSignal::abort_deadline_before` reports it and the close instant is
recomputed, halfway from the same cancellation instant to the new deadline.

[Real socket tests](../crates/batter-axum/tests/operational/forced_close.rs)
cover a pending body through both registration helpers, a pending body after
pipelined request bytes that Hyper's first read returned with the head, a
stalled reader of a 64 MiB fixed body, a half-sent request head that outlasts
drain, a cancelled handler whose 503 `operation_cancelled` still arrives
complete, an abort deadline that a managed component moves earlier after
forced cancellation, a connection task blocked inside its body's poll
across the close and the abort deadline, which still aborts the wrapper and
skips cleanup, and, through both registration helpers, a pending body whose
destructor hands its worker over and blocks after Axum counted its connection
finished, with the same for an in-flight handler (a fallback outside
admission that never returns and holds such a guard): destroyed within the
allowance, cleanup runs only after it; still destroying at the abort deadline,
the wrapper is aborted and cleanup skipped. A gate unit test shows that a zero
cancel allowance never closes the gate while a positive one does. On a paused
clock, a gate unit test drives the close computation through a private stop
clock: cancellation observed with the abort 4 s away, then the abort moved to
1 s, and the gate closes at exactly the recomputed 0.5 s. Another runs it
against a real supervisor, whose native component reveals an earlier stop only
after the gate computed its first close; holding that component needs a
multi-thread runtime and real time, so its outcome rests on a wall-clock
margin: the original close is about 15 s away and the recomputed one about
0.5 s, against a 5 s wait. A unit test shows the completion
wait holds for a response body that outlives its connection's service. A
[TLS case](../crates/batter-axum/tests/tls_serving/serving.rs) repeats the
pending body through the assembled boundary over rustls. With the close
removed, the four plain-socket cases that depend on it fail and the
interruption response still passes; the TLS case was not mutation-checked.
Hand-run mutation checks on 2026-11-11, applied temporarily and reverted and
not an automated gate: recording wakers only on pending polls failed the
pipelined case and a gate unit test of a task whose last I/O poll was ready,
and a close instant fixed at cancellation failed the moved-deadline case. On
2026-11-12, closing with no interval left failed the zero-allowance gate test,
and returning from the server once the gate closed failed the never-yielding
case. On 2026-11-13, returning without waiting for the connections' tokens, or
giving them only to the connection's service and not to its response bodies,
failed the slow-destructor case, as removing the wait also failed the
in-flight handler case, removing the wait from the connect-info path
alone failed that path's case, and dropping the `abort_deadline_before`
recomputation failed both moved-deadline gate unit tests, the paused-clock one
by closing at the original 2 s.

The limits remain. When forced cancellation is observed with no interval left,
as with a zero cancel allowance, the gate never closes, so the earlier contract
holds unchanged on any runtime flavour: a real streaming regression is aborted
with its body still owned, the report is unsuccessful, cleanup is skipped with
`UnsafeTaskExit` and detached connections still write their interruption
responses. A connection task that never yields cannot be ended by failing its
I/O: it outlives the close and the abort deadline, and the wrapper is aborted
with cleanup skipped. An abort deadline moved so close that half the remaining interval
cannot end the connections still aborts the wrapper. An upgraded connection
leaves Axum's connection accounting
at the upgrade: its task belongs to the application, is never joined here, and
its I/O fails only if the server is still serving another connection when the
close happens. Tasks a handler detached are untouched. Applications that serve
`axum::serve` themselves, as the two HTTP lifetime suites deliberately do, get
none of this. There is no new async-drop, response-stream, WebSocket or
disconnect guarantee.
`low_level::register_http` and `low_level::register_http_in` accept a plain
Router and supply no ConnectInfo extension. Their opt-in companion
`low_level::register_http_with_connect_info_in`
uses native `into_make_service_with_connect_info::<L::Addr>` so middleware and
handlers can extract the listener's own accepted peer. For a TcpListener, and for
a TLS listener over TCP, that remains `ConnectInfo<SocketAddr>` with the accepted
socket's remote address and port. Forwarded,
X-Forwarded-For and X-Real-IP headers are not interpreted; behind a proxy the peer
is the proxy. Authentication, proxy trust and application extension replacement
remain application-owned. The companion shares the registration, acknowledgement,
listener-release, graceful-drain, forced-close and conservative wrapper-abort contracts above.
It accepts no arbitrary make-service or custom metadata type. Pinned Axum 0.8.9
implements `Connected` for a bare listener only for TcpListener and generically
only for `ListenerExt::tap_io`, so the companion applies an empty tap itself; it
changes no accepted connection and no reported address, and its extra `Clone`,
`Sync` and `'static` address bounds are Axum's `Connected` requirements.

Accepting any listener moves no policy into Batter. The listener owns binding,
certificates, private keys, protocol versions, ALPN, client-certificate rules,
handshake concurrency and accept retry; Axum's trait cannot return an accept
error, so accept failures never reach the component exit. When drain arrives the
accept in progress, including a handshake awaited inside it, is dropped without
being awaited, and the listener is released before connection completion is
awaited. Work a listener spawns onto the runtime instead of polling inside
`accept` is a detached descendant with exactly the standing of Axum's own
connection tasks: registration proves nothing about its termination, and wrapper
abortion still conservatively skips cleanup. A real rustls regression serves the
canonical assembled boundary with a generated certificate and asserts startup
acknowledgement before readiness, a probe answered outside admission, the
transport peer against a client-owned oracle, a drain that waits for a response
still streaming on an open connection, the destruction of an accept parked in its
handshake, and release of rejected and abandoned listeners. TLS crates are
`batter-axum` dev-dependencies only; no published package gains a TLS dependency.

## Optional database fixture finish

`batter-sqlx/test-support` checks declared connection capacity before acquisition.
`FixtureSuite::start` retains native lease/template producers before their waiters
can be cancelled. Acquired resources register before delivery and native pools
register before initialization. Body exit is followed by joining all producers,
closing each database's pools, awaiting lease cleanup, then draining deferred
cleanup. Reports retain body, acquisition, database cleanup and drain results.
Owned reports and the borrowed `FixtureReportRef` returned by `wait` warn on
accidental discard. A successful driver join does not imply report success;
inspect it or use `into_result`. Explicit discard can still lose observation.
A failed producer keeps the report unsuccessful even if its waiter was abandoned
or the body handled the error. Template initializer panic reaches upstream's
awaited abort path with its native join cause retained.

The borrowed scope cannot escape into a detached task. A run rejects batches
exceeding its own simultaneous-lease limit; other harness owners must release
shared native capacity for waiting acquisitions to progress. Run completion does
not shut down a shared server. Caller-owned shutdown follows all runs and leases;
shared deferred drain covers prior accepted submissions, including other owners'.

`FixtureRun` holds one private state: the live driver or its retained completion
result. A completed driver is never polled again; repeated observation borrows
the same report or join error, and `into_report` moves the retained result out.
Cancelling `FixtureRun::wait` permits resumed observation. Dropping the waiter
loses the report but detaches the driver. Application operations/checkouts and
initializer-created pools still require explicit join/release/close. Detached
backend termination, runtime death and internal driver failure have no completion
guarantee. Low-level `DatabaseFixture` is must-use; its finish remains caller-owned
and cancellation can trigger destructive lease Drop. Low-level template waiter
loss can leave initializing templates outside the deferred queue.
The manual template path also propagates initializer panic without the owned
path's abort wrapper. Cleanup with initializer-owned live connections and forced
producer cancellation is unverified; neither is a supported completion claim.

`PoolAcquire` reports pending driver cleanup; low-level `Connect` reports the
outcome after explicit cleanup. Live probes cover cancelled native creation,
abandoned initializer error/panic, shared admission, partial/sibling acquisition,
body panic, simultaneous errors, close ordering and independent catalog absence.
`batter-kjl` additionally retains handled pool-acquisition errors in each database
report; the delivered `PoolAcquire` and retained `pool_failures` share the same
SQLx-typed cause through Arc. SQLx's internal after-connect retries may expose only
the final native acquisition error; this does not recover errors SQLx itself hides.

`batter-7r3.8` constructs and registers every declared pool of a lease before
the first connectivity check, so a failed check on either path, or a cancelled
owned `FixtureScope` acquisition waiter, leaves all of them closed before lease
cleanup. Cancelling low-level `empty`/`from_template` can still drop pools
unclosed and invoke destructive lease Drop. `PoolKey` lookup refuses keys from
another plan. `PlannedPool::password_login` replaces only the
username and password of the lease's native options; a lease URL selecting
another database is refused before any pool exists and retained on both paths
(live probe `fixture_lease_database_mismatch_retained`). Live probes prove distinct
`session_user` and `system_user` for a lease-login and a restricted-login pool,
42501 owner-only denials beside a granted read through the reference profile,
28P01 and session-policy failures retained by identity, both cancellation points
and pools registered after an earlier failure. Role lifecycle stays outside
fixture cleanup, and the lease binding holds at pool construction only.

`FixtureRun::wait_for` bounds observation only: `Ok(None)` means pending. A cancelled
wait returns no result and can be resumed; a cleanup-driver JoinError is separate.
`cleanup_progress` exposes historical database phases and observation errors.
With `with_session_observer`, cleanup closes every tracked pool, then queries all
sessions for the database through an independent admin pool. The query checks
that the database exists and the observer is outside it. The caller must select
the same server and stop all producers of new connections. This is a point-in-time
absence witness, not fencing, remote cancellation or permission to reuse a database.
Matching database names do not validate cluster identity. A differently targeted
server with the same name can satisfy the query; normal native catalog visibility
and actual same-server targeting are caller preconditions. The observation does
not clear prepared transactions, active logical replication slots or subscriptions;
native cleanup may still fail after session absence is observed.
The live SCRAM startup control shows why producers must stop: an authenticating
backend can have NULL database identity and pass this filter. Native lease
cleanup can then remain pending on PostgreSQL's process barrier until that
connection closes. A separate live autovacuum control proves that an assigned
background worker is included and can require explicit retry after timeout.
The startup control identifies the actual target DROP backend's ProcSignalBarrier
wait while its sole new startup socket stays open, then closes that socket and
requires completion. Its identity window requires the dedicated serial test server.

A bounded session observation that times out or returns an error retains the lease
and parks only that database until an explicit `SessionObserver::retry_with`
request is available. Requests broadcast; one made during an active attempt is
consumed after failure, and unread requests coalesce. Independent
cleanup continues. Explicit retries coalesce while an attempt is active; they
never interrupt active work. Each failed attempt remains in the final report,
which stays unsuccessful even after eventual cleanup succeeds. The driver retains
the retry channel: losing all external controls can leave it parked indefinitely.
There is no implicit destructive fallback while the driver remains live.

Native detach and adapter retirement tests hold acknowledged blocked backends
past successful pool close, observe their identities and retained database from a
separate one-slot admin pool, then release them and resume cleanup. Observer
PoolClosed and wrong-target failures retain identical causes across progress and
report. Separate native resources prove body, consuming-cleanup and deferred-drain
errors coexist; external harness shutdown does not replace drain. Default native
panic hooks and upstream harness diagnostics remain outside adapter redaction.

Report summaries count failed consuming database cleanups separately from retained
pool failures and observation attempts. Successful cleanup with a recovered error
still makes the report unsuccessful, while database_failures remains zero. Replacing
an incorrectly targeted observer does not close its sessions: callers must explicitly
close their native pool clone and witness backend exit before the corrected retry.

A live-runtime waiter-loss test proves driver cleanup continues after release of
a held checkout. A separate actual runtime-destruction test observes a cancelled
driver JoinError and native lease Drop deleting the database while a checkout is
still held. That is destructive fallback, not completed pool closure or a fixture
cleanup guarantee. The test drains upstream cleanup from a surviving harness on
another runtime; process death is not covered.

Reference fixture completion uses one bounded consumer helper. A pending error
retains the waiter, diagnostic pool, all session pools and retry control together for typed recovery;
its automatic formatting exposes phases/counts only. It never treats timeout as
permission to drop the database. Abandoning that error loses the recovery handle;
runtime destruction retains the native destructive Drop limitation above.

Specifically, an unrecovered pending result passed to the reference `assert_probe`
boundary panics and may destroy its per-test runtime, triggering that native
fallback. The helper's timeout itself does not delete a lease; subsequent test
runtime teardown may. This boundary does not guarantee database survival after
an unsuccessful test returns or panics.

The reference completion bound also covers administrative pool closure. Driver
failure closes session and diagnostic pools before propagating the original
JoinError. If a checkout prevents closure, the pending owner retains that cached
error and both capacities; resuming after release completes their close first.
A successful join leaves diagnostics open for the caller's catalog checks. Pending
formatting includes separate redacted driver-joined and driver-failure flags,
so successful driver completion with pending admin closure remains distinguishable.


## Native Runledger handler panic records

When Runledger catches a main-handler panic, its failure conversion discards
payload text and uses `Job handler panicked.` for the `job.handler_panic`
failure and the `job_handler` settlement diagnostic. The persisted outcome is
still `PANICKED` and nonretryable, even when attempts remain. Deadline and lease
fencing retain precedence over durable outcome classification. The worker can
continue to subsequent jobs. Application-created `JobFailure` messages remain
verbatim; this conversion does not sanitize them, other callbacks' retained
`RuntimeCallbackFailure::Panicked` messages, or process panic-hook output.
Separately, Runledger's observer, dead-letter-hook and task-join log events omit
panic payloads, and `RuntimeShutdownFailure::LoopJoin` and `DescendantJoin`
return `None` from `Error::source()`. Join-failure events, including supervised
runtime loops, carry the Tokio task ID and panic/cancellation flags without
formatting the join error. Log-capture tests in `runledger-runtime`
(`*_logs_omit_the_payload` and
`supervised_loop_join_logs_identity_without_payloads`, plus
`join_failures_keep_panic_payloads_out_of_the_source_chain`) check a sentinel
payload against each such event and the source chain.

The PostgreSQL-backed `process_claimed_job_catches_main_handler_panic` regression
checks a formatted synthetic secret against queue, attempt, dead-letter and
event records, the actual dead-letter warning, and settlement diagnostics.
`run_worker_loop_continues_processing_after_handler_panic` checks continuation.

## Application-installed redacting panic hook

`batter::redacting_panic_hook()` (also `batter_core::redacting_panic_hook`)
returns a `#[must_use]` `Box<dyn Fn(&PanicHookInfo<'_>) + Send + Sync + 'static>`,
the type `std::panic::set_hook` takes. Constructing it installs nothing, and no
Batter library installs, takes or replaces the process hook. No workspace library
source names `set_hook`, `take_hook` or `update_hook` outside comments, files
that a `#[cfg(test)]` module loads and no production module does, a test-only
status derived from module declarations rather than file names or directories,
and the binary roots that the executable-root inventory classifies
(such as `src/main.rs` and `src/bin/*.rs`). The entrypoints that
own process-wide signal handlers or catch panics keep an application's hook
installed and unwrapped, passing each panic they catch to it once. Dependencies
are not scanned. Every checked-in
executable root in this repository that consumes Batter, among the workspace
members' binaries, examples and benchmarks and the consumer sources built outside
the workspace, installs the hook, unwrapped, as the first statement of `main`,
except three named tooling roots, a live-suite preflight, a provisioning harness
and a benchmark, that keep the default hook so their failures print diagnostic
messages. Native Runledger and Runlimit roots name no Batter crate and keep their
own policy, and programs that scripts generate as compile probes are not
classified.

Each panic writes one line to standard error, after a newline:
`a thread panicked at <file>:<line>:<column>; panic payload redacted`, or `at an
unknown location` when std reports none. The text is part of the contract, and a
change to it is recorded in the changelog. The private renderer receives only the
location's file, line and column, so the payload cannot reach that output. The
hook never reads, writes or forwards the payload. Std formats a `panic!` message's
arguments, or the `Debug` of an error passed to `unwrap` or `expect`, into the
payload before any hook runs; `catch_unwind`, joins, Tokio `JoinError`s and Batter
reports retain that payload, and this hook never writes it. The hook calls
neither the previous nor the default hook, emits no tracing event, reads no
thread name and prints no backtrace, whatever `RUST_BACKTRACE` says. It returns
normally, so control flow is unchanged: an unwinding panic still runs destructors
and reaches `catch_unwind`, a thread join or a Tokio `JoinError` with its original
payload, and a panic escaping a libtest test function still ends the process with
the same status 101 as under the default hook. A line of up to 512 bytes is
rendered on the stack and written with one `write_all` to a duplicate of
descriptor 2; a longer one is written in pieces. The hook never takes the
`std::io::Stderr` lock, which another thread can hold indefinitely, and it does
not wait for one that is held. When the descriptor cannot be duplicated, for
example at the process descriptor limit, it writes nothing. Write errors are
ignored. Closing the duplicate after each report releases the process-owned POSIX
`fcntl` record locks the process holds on the file behind standard error;
open-file-description locks are unaffected.

The hook covers payload printing at panic time only and removes the formatting
and delegation the consumers in ADR-014 performed. It does not remove the
obligation to install it: nothing can prove that a binary installed it, installed
it before its first panic, or kept it. Any later `set_hook` replaces it, the
default hook is installed between a `take_hook` and the next `set_hook`, and a
wrapper or later hook that formats `PanicHookInfo`, whose `Display` includes a
string payload, or the payload itself, or that calls the default hook, prints the
payload again. Wrapper code that can itself panic, such as `eprintln!` to a closed
pipe, makes std abort the process, because a panic raised while a hook runs cannot
unwind. Std also aborts when a panic occurs while it formats a `panic!` message's
arguments for a hook; it then prints the nested panic's location, and its message
only when that message is a constant string, but not the original payload. In a
child that `std::process::Command` forks, std disables hooks before running
`pre_exec` closures and `exec`, so a panic there prints the full payload without
consulting any hook. With `#[tokio::main]` the default hook still covers runtime
construction, which precedes the first statement of the body. In-process tests
must not install the hook: its line bypasses libtest's output capture, and libtest
prints a failing test's panic message only through the default hook, except for a
`#[should_panic(expected = ...)]` mismatch, whose payload it prints itself. Tokio's `JoinError`
formats a string payload in its `Debug` and `Display`, so formatting a retained
one discloses it, as do error-source traversal of a report that retains one and
explicit `PanicPayload::try_inspect`; payloads retained in Batter reports keep
their explicit field access. The hook does not sanitize logs, other output or the
process. Aborting panics and a real `fn main` boundary are source-inspected, not
tested.

## Explicit settings and redacted diagnostics

`batter::settings` supplies std-based explicit inputs, a bounded literal reader,
unsigned decimal/duration bounds, `SecretString`, and static diagnostic projections.
It installs no source order, setting names, subscriber, environment mutation or
file discovery. Applications validate source structure before overlaying inputs,
then parse winning values. Missing, empty and non-Unicode remain distinct;
invalid winners never select defaults. Duplicate pairs fail before merging. `SettingsSource::select_dedicated`
rejects every unknown name; `select_environment` ignores unrelated names but rejects
unknown reserved-prefix names. The application selects the source policy; neither
method reads or infers environment provenance. The former boolean `select` method
is removed; callers must choose one of the named source policies.

`read_file` opens only its supplied path. `read_literal` reads at most the checked
byte limit plus one and requires UTF-8. The dialect supports LF/CRLF, blank lines,
full-line comments, ASCII identifier keys, outer space/tab trimming, and literal
unquoted or wholly single/double-quoted values. Unquoted whitespace, quotes and
`#` are rejected. Quoted interior bytes are literal; dollar and backslash have
no interpolation or escaping semantics. Export statements, multiline values and
inline comments are unsupported. Source errors identify static categories and
optional line numbers, never input keys, paths or values.

`SecretString`, `SettingsSource`, `SettingsError` and `RedactedError<E>` hide values
in ordinary, nested and alternate Debug/Display. Secret access and error-source
traversal are explicit trusted operations. The original native causes remain
available; arbitrary source-chain reporters can expose them. No encryption,
zeroization, process-wide logging policy or panic-hook redaction follows from
these types; see
[Application-installed redacting panic hook](#application-installed-redacting-panic-hook).

The reference root owns defaults, precedence and two concrete command schemas.
`ServingSettings` contains a password-qualified endpoint, concrete
authentication and validated `JobsConfig`, plus Batter/Axum capacity and budget
witnesses. `MaintenanceSettings` recognizes only its database endpoint field,
permits a passwordless externally provisioned endpoint, and has no conversion to
serving. Dedicated maintenance files and overrides reject serving-only fields.
Known serving fields may coexist in captured process environment and are ignored
by maintenance without parsing; unknown reserved names and every PG* name fail.
`runtime::prepare` consumes serving settings into a must-use non-cloneable inert
owner; only that owner can enter `runtime::run` or `runtime::start`, while canonical
`http::register_in` or explicit `http::in_process_client` consumes its
narrower opaque `PreparedHttp`. These local types cannot prove remote database
authentication or availability.

With the opt-in `metrics-export` feature and an explicit loopback collector, the
reference selects `batter-otlp` through `batter::service::start`, which owns
installation before protected startup and releases the final flush. The
reference calls the selected adapter's installation synchronously, then maps
its eventual report; an immediate competing recorder cannot take its slot.
The guard rejects names, label shapes, descriptions and keys outside the
foundation catalog and its `MAX_SERIES` bound before the bridge allocates. One serial owner
exports manual-reader snapshots under fixed deadlines and payload/response
ceilings, with no queue or retry. The adapter explicitly disables native HTTP protocol
retries even when consumer dependencies enable HTTP/2. Service startup rejects
a missing Tokio runtime before installing diagnostics or beginning startup.
The final snapshot is collected only after the service result is retained
(startup-failure cleanup, or complete driver
settlement including the shutdown metric after `Stopped`), exported under a
separate allowance and followed by exactly-once exporter/provider closure; no
readiness event, drain or cleanup hook can start it, and no service cleanup
budget waits on the collector. `ServiceCompletion` keeps the original service
result, report and exit classification beside application-owned `MetricsExport`
outcomes. The adapter's completed report contains only export and
rejected-installation outcomes; disabled configuration and abandoned diagnostic
execution are classified by the application.
`Acknowledged` means one decoded HTTP 200 collector response with the Protobuf
media type and without rejected points, even when the encoded response body is empty.
Wrong or missing media types cannot acknowledge an export. It does not prove
durable storage; a timed-out or cancelled request is neither delivery nor
remote rollback. `FinalCoverage::Incomplete` reports unjoined tasks, uncertain
native settlement and skipped or unjoined cleanup; `Reported` describes the
report, not unsupervised producers. Owner drop requests drain and waiter
cancellation requests nothing. Core retains the native service outcome independently
of diagnostic installation, periodic work and finalization, including diagnostic
panics. The reference retains that evidence through `ServiceCompletion::foundation`.
If an extreme schedule allowance ceases to fit in a monotonic instant after
validation, the export owner shortens that allowance until its deadline is
representable; this cannot lengthen an attempt or bypass final closure.
A diagnostic panic does not guarantee resource closure; synchronous collection,
closure and recorder calls cannot be preempted by async deadlines. Runtime death,
SIGKILL and blocking or panicking recorders inside application work remain outside
this isolation boundary.

The supported TCP URL subset requires explicit host, username, database and
sslmode; only password, sslmode and application_name query settings are accepted,
once each. Credentials are decoded once, invalid percent/UTF-8 encodings fail,
and unknown query settings are rejected before any SQLx URL parser can warn.

SQLx 0.9 lacks an environment-free default constructor. The reference rejects
all captured PG* entries, even empty/non-Unicode ones, and rechecks the actual
process environment before `new_without_pgpass` and explicit setters. The process
must not mutate its environment during construction. No passfile is loaded.
Returned native options and their Debug/URL conversion are trusted exposures;
upstream runtime diagnostics remain outside this projection. The selected graph
has no SQLx TLS backend: requested TLS modes are retained, not downgraded, but
TLS connections require the application to enable a native TLS feature. TLS
negotiation is unverified. SQLx's native URL formatter cannot round-trip a bare
setter-provided IPv6 host; the reference passes the bare host required by Tokio
and separately tests its native startup/password wire exchange.

Focused offline tests prove source policy, native field mapping, changed HTTP
response deadlines, independent admission capacities and retained startup/cleanup
failures. Explicit live cases additionally require a disposable PostgreSQL 18
endpoint.
The unpublished reference package now composes these constructors into a command
and provider-effect worker. Its application-owned protocol proves only stable-key
reconciliation against the local fixture with a 24-hour retention boundary. No
publication, deployment, exactly-once guarantee or external adoption follows.
The worker's private state API locks and revalidates the exact unexpired
Runledger lease before every effect mutation; callers cannot issue an unfenced
state write. Retained state selects terminal, local-expiry, reconciliation or
dispatch work before provider admission. POST and GET require one canonical
accepted-request identity check. This does not fence the external provider after
the database transaction ends: stable idempotency and retained uncertainty still
own that recovery boundary.

Retained provider payload and key identity must match the accepted command before
worker dispatch or either owner-scoped read can proceed. A future dispatch
retry-not-before does not defer required keyed reconciliation: an accepted lookup
may confirm immediately, while authoritative absence still cannot authorize an
early POST. These are reference-application boundaries, not framework guarantees.

Reference confirmation serializes with generation replacement: it locks the
record FOR SHARE, then the native job, then the effect, and retains all locks
through commit. A replacement committed before record-lock acquisition is read
by the subsequent confirmation statement and makes acceptance manual resolution.
A replacement arriving afterward waits for confirmation to commit. No provider
I/O occurs in that transaction. Waiting for the record does not hold the job-row
lock; later job/effect waits can still contend with heartbeat maintenance. The
post-lock and precommit lease checks remain mandatory.

Reference listener discovery is optional and application-owned. Explicit
`BATTER_LISTENER_ANNOUNCEMENT_PATH` selects an existing Unix datagram receiver;
publication is nonblocking async I/O under the startup deadline, with no spawned
blocking writer. Unconfigured startup never writes a listener announcement.
The datagram acknowledges binding, not readiness, and errors fail startup.

Manual resolution and business denial share a terminal writer whose source type
can represent only unresolved states. Both entry methods reject terminal sources
before database acquisition; the SQL equality predicate additionally rejects a
stale unresolved source. Neighboring confirmation, failure, invariant and dispatch
writes constrain their own legal sources. This is an application-private boundary,
not a new foundation provider protocol or protection from arbitrary external SQL.

Detected retained-state invariants become an explicit reference-worker action.
Their manual-resolution write runs in finalization, including when a lock wait
outlives work's shortened deadline. Interrupted-fence reloads use the same writer
within finalization. The write still requires the live native lease and original
deadline; failed storage or lost authority cannot establish recorded recovery.

`Bulkhead::run` owns each provider exchange and releases its capacity before
finalization persists the outcome. Application-private exchange methods own
admission, dispatch-fence execution and provider entry together with their
interruption evidence. There is no consumer progress setter. A returned fence
error clears its recovery obligation before leaving that boundary; interrupted
fence execution retains reload authority. Denial, manual-resolution and admission
actions carry only unresolved source states. SQL still revalidates the live
lease and current state; these local types cannot prove remote outcomes. Send and body errors remain distinct, with static
diagnostic codes and connector-only known non-dispatch. One owner-scoped query
boundary validates identity and projects unresolved terminal jobs from a single
job/effect snapshot: spent dead-lettered attempts become exhausted, other native
termination becomes manual resolution, and retained acceptance facts survive.
Projection requires no write after lease loss or database failure.

The provider state transaction checks lease identity and expiry in a fresh
statement after locking, and again before commit after later lock waits. Dispatch
eligibility is checked under the effect-row lock. Known non-dispatch outcomes
atomically retain an absolute retry-not-before timestamp; native completion loss
cannot discard that committed lower bound. The private persistence method owns
both the outcome write and returned retry scheduling decision. Storage errors
retain an observed delay in the returned native failure. Process death before
the response is persisted can still lose that observation, and recovery attempts
continue to consume Runledger's attempt budget. PostgreSQL's wall clock remains
the durable timing authority; no cross-system clock or remote-effect fencing is
claimed.

The private live endpoint handoff accepts `localhost` or `127.0.0.1` with
sslmode=disable. It rejects IPv6 literals before acquisition because SQLx 0.9
retains their URL brackets during TCP lookup. Direct root construction still
supports bare native IPv6 options. The handoff serializes the validated host and
database, canonical TLS mode and explicit password choice, including empty
passwords; fixture URL parsing must not select an ambient passfile. Query spaces
retain their meaning in both native clients. Parser regressions model the
harness's normalization and database-path replacement; they do not establish
live authentication or TLS.

## Offline legacy retirement

The reference's finite retirement command verifies expected cluster/database
identity on one direct PostgreSQL session, rejects replacement of that session,
and requires no other target-database client or unknown backend and no prepared
transaction. These checks precede native definition disable and repeat before
completion. It cancels only global nonterminal legacy controls through native
Runledger policy. Applied migrations, sequence state and terminal/domain history
remain. An absent definition is not a durable disabled tombstone.

Deployment tooling must stop old producers and prevent restart before maintenance;
the database report cannot establish that external fact. Native definition disable
blocks the inspected enqueue path and is preserved by the inspected additive
catalog sync, not arbitrary SQL or administrative re-enabling. Pool closure alone
does not prove backend death. Connection loss, deadline expiry or cancellation
never proves absence of remote effects or permits automatic replay.

A returned cancellation failure is published with the owned command report before
optional readback. The separate read-only command cannot overwrite that report;
its cancellation leaves the primary cause inspectable. Scoped native cancellation
retains SQLx begin/commit sources and a separately returned rollback failure.
Reports cover returned outcomes, not every intermediate value inside a cancelled
native future. Real commit rejection and interruption during readback are covered
by PostgreSQL 18.6 probes; full redesign acceptance remains tracked separately.

The PostgreSQL verifier explicitly rejects selected temporary namespaces
(`pg_temp`, `pg_temp_*`, `pg_toast_temp_*`), their objects and routine argument
identities with Incomplete/TemporaryNamespaces before ledger locking or ACL
evaluation. This is an unsupported surface, not a claim of absent authority.
Exact PUBLIC relation declarations suppress column defaults, while explicit
column exceptions remain expressible. Whole-relation allowances still imply
column allowances. Required-policy validation and discovery share that rule.
Native privilege inquiry functions can observe newer catalog state than a
repeatable-read ACL query; required and excess checks therefore use the same
captured data and native inquiries serve as differential test references.

## Runledger invocation phases

`PhasedJob::new(handler, positive_reserve)` is the protected handler path for
work followed by outcome-aware finalization. Its `PhasedJobHandler` callbacks
borrow the same native execution services and receive only their own phase's
context. Work return, error, panic and work-budget expiry all reach finalization
while native invocation authority remains live. The adapter uses the foundation's
command-phase polling and destruction checks, retains both outcomes, and accepts
a native completion only after both phases succeed by default. A required `failed` callback
maps the complete failure to native policy and cannot return job success.
An explicit `settle_late` override may instead consume `LateJobFinalization`
and propose settlement of a retained `Ok(value)` returned after the shortened
work deadline, without cancellation or polling/destruction panic. The opaque
`LateJobSettlement` is not a runnable future or a completion proof. The adapter
drives its callback as finalization through the same kernel, accepts recovery
only after successful return and clean destruction, and rechecks the original
native deadline and invocation exit before releasing completion. Declining runs
the ordinary finalizer and keeps strict failure. Missing values, application
errors and panics never receive this capability. `PhaseOutcome::classify` stays
strict; ordinary finalizer success cannot erase a work failure. Application
settlement semantics remain application policy. Application commits and native
completion persistence remain separate; invocation death between them still
requires durable reconciliation.
The callback future reaches the polling kernel directly: its returned value or
error remains available when destroying that future separately panics. A panic
inside an application callback's own awaited subfuture can still prevent that
callback from returning a value; no unreturned result is reconstructed.
The entire adapter future belongs to the native worker; there is no independent
phase waiter to cancel. Native deadline, lease loss and task destruction still
end the invocation and may interrupt finalization. No async-drop, remote-effect
or detached-work guarantee follows. The core `operation::low_level::drive_phases`
is an adapter implementation seam with these host-ownership obligations, not a
protected borrowing bracket for application code.

The adapter privately derives `OperationPhases` from the native `JobExecution`;
the former public `job_phases` function is removed. Both phases share one root at
exactly `JobExecution::deadline()`, converted from the native std instant to
Tokio's view of the same monotonic clock; deriving late never moves it. Work
ends `reserve` before that deadline through `reserve_finalization`'s checked
arithmetic. `PhasedJob` validates a positive, clock-representable reserve of at
most one year before registration, through core `OperationPhases::validate_reserve`;
the invocation still checks that its deadline leaves positive work time. Finalization keeps the
native deadline. Cancelling or exhausting work
leaves finalization running. Derivation returns `JobPhasesRejection` without
running application work when the services make no exit claim (`Unsupported`),
the invocation already ended (`Ended`), no positive work time remains
(`Exhausted`, including a subtraction outside the clock's range) or the reserve
is not an accepted duration (`Reserve`). Later operation boundaries keep their
preflight: an expired or cancelled phase invokes no factory, and an unpolled
operation is inert.

Native ownership: Runledger ends each invocation's `JobInvocation` exactly once
when the invocation exits for any reason (success, continuation, returned
failure, panic, deadline, lease loss found by progress or heartbeat, failed lease
maintenance, task abort or destruction), after the handler future is destroyed
and before the outcome is persisted. A graceful stop request alone does not end
it. The bridge registers one exit hook per derivation; the hook owns the root's
only cancellation authority, so both phases and every context derived from them
observe the end, including observers in independently spawned tasks and contexts
derived afterwards. Handlers, observers and copies of the execution handle can
neither end the invocation nor cancel the root; child contexts cancel only
downward, and separate invocations and derivations are isolated. Continuations
and retries are new invocations with fresh signals. Waker cloning, waking and
destruction, including waiter replacement and removal, run outside the invocation
state mutex so reentrant state reads do not deadlock it. Waiter wakes and exit hooks
have independent panic boundaries: one panicking notification cannot discard
later waiters or the bridge's cancellation hook. All contained notification
panics are retained as runtime callback evidence under the existing
`job_invocation_exit_hook` category; notification still grants no cleanup proof.
The report owns payload disposal, and native diagnostics borrow its payloads.
Dropping an owner or report releases strings but intentionally retains opaque
panic allocations without executing their destructors. Raw `into_payloads()` is
a low-level transfer whose caller owns potentially panicking disposal.

**Not supplied:** joining or stopping detached work, cancellation shielding,
undoing remote effects, effect fencing, a generic `OperationError` to
`JobFailure` conversion, or any change to native outcome precedence. A result or
final-state write observed at or after the native deadline is
`job.timeout_exceeded`. The
reserve lies inside the handler deadline; native outcome persistence follows the
handler. Notification never grants dependency-cleanup authority or turns a
completed, late or fenced result into an interrupted handler.

## Shared Runledger workspace

Native PostgreSQL fixtures use Bollard directly; neither Testcontainers nor
`astral-tokio-tar` is needed by the optional facade test-support feature.
Failed starts retain label-checked, awaited removal by immutable ID, including
anonymous volumes. Successful container handles retain runtime-owned cleanup
when dropped or when delivery to a cancelled startup waiter fails. This needs a
live Tokio runtime. The optional CLI process reaper owns shared-container cleanup
on normal or forced process exit; unavailable CLI support remains a reported
limitation. `TESTCONTAINERS_COMMAND=keep` deliberately retains owned containers.

The optional adapter and native Runledger packages now resolve from one checkout.
The package ownership graph remains acyclic: native persistence consumes the
SQLx foundation, while native runtime supervision and durable policy stay in
Runledger. The old Git-revision source rejection is removed; Cargo identity and
failure-contract tests validate coordinated changes. Migration SQL and offline
metadata were imported unchanged. This source move does not migrate a deployed
database, broaden Unix support, publish packages or prove remote effects.

## Native Runlimit source identity

The shared workspace resolves one local identity for each native Runlimit package.
Graph controls reject remote/duplicate identities, publication and reversed native
ownership; imported SQL/license digests detect accidental changes. This does not
change quota semantics, authorize replay after uncertain commit, or make local
storage a fleet-wide limit. Persisted migrations and key/lock protocols remain
compatible with the imported revision. See [provenance](../runlimit/IMPORT.md).

### Protected service readiness and output

`service::start_with_readiness` owns the first-readiness deadline and typed
startup handoff. The deadline is absolute and installed before diagnostics or
initialization run. Readiness requires application approval, driver execution,
and every component acknowledgement. A transition at or after the deadline
cannot publish Ready; an owned timer requests drain and records deadline expiry.
A pre-deadline Ready transition is retained even if an observer runs later or
shutdown has begun. Current lifecycle status remains a separate read-only view.

Only consuming `StartingService::into_ready` transfers startup output. Observers
cannot claim it, and no domain Clone bound is imposed. Failed delivery returns
full service/diagnostic completion; rejected-output destruction failure has its
own retained field and does not bypass running-driver settlement. A cancelled
borrow of a retained handoff future does not cancel it. Dropping the owner or
owning handoff future requests drain, including before first poll.

This readiness deadline does not bound shutdown or diagnostic settlement; their
existing budgets remain in force. It does not prove continuing readiness, remote
effects, detached-task quiescence, runtime survival, or preemption of non-yielding
code. Existing unit-output `service::start` remains supported without an added
readiness deadline.

## Managed-object release ordering

`ManagedObjectManifest` requires a positive `ManagedRevision`. Under the ledger
lock, apply refuses older revisions and equal revisions with different content.
Revision advancement commits with the managed objects; equal revision/content
reapplication remains idempotent and can repair drift. Verification checks the
revision too. Malformed or missing release evidence in an existing versioned
ledger cannot reset the fence. Application SQL cannot alter retained revision
metadata and still obtain a successful managed receipt.

Legacy five-column ledgers require a matching recorded manifest for baseline
adoption. The transactional sixth-column upgrade causes pre-fence exact-shape
readers to refuse apply. There is no default revision or unfenced apply alias.
Applications own revision ordering, baseline selection and cross-release source
compatibility. SQLx migrations precede the gate and can already have committed;
arbitrary migrations, administrative SQL and ledger deletion are outside it.
See [ADR-013](adr/013-managed-database-objects.md#release-ordering-and-legacy-adoption--2026-11-05).

## Retained enqueue-intent reconciliation

Runledger's owned atomic scopes expose `reconcile_job_enqueue_intent` for an
exact `JobScope`. One SELECT observes the intent, its same-type/same-scope
promoted job and the current definition, including the transaction's own writes.
Opaque evidence prevents constructing an observed promoted state without job
metadata. Invalid retained links fail the query; absent definitions are explicit.
The reader takes no row locks and preserves the intent-to-queue phase transition.
Its output remains provisional inside the atomic body and follows the existing
acknowledged/uncertain transaction outcomes.

`NotFound` means no visible retained intent for that ID and scope. It does not
prove rollback: retention, another scope or concurrent transactions can explain
absence. Application authorization, original idempotency inputs and the decision
to retry remain application-owned. Evidence is a point-in-time observation, not
a reservation against future promotion or retention. The new reconciliation
grant reads only selected metadata; adding other operation grants can expose more
columns, including canonical submitted requests for intent submission.
