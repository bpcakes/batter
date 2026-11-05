# batter-outbound-http

Send-once outbound HTTP exchange with exhaustive transport evidence. This
experimental 0.0.1 package targets Unix, including Linux and macOS; its tests
had Linux and local macOS evidence in `batter-8e8r`; the owned-connection
implementation is verified separately in `batter-u387` and has no new macOS evidence.

A provider call can fail in a way that leaves the caller unsure whether the
provider received the request. This package owns the transport part of that
question and nothing above it: it builds the HTTP client itself, sends each
prepared request once under the caller's deadline, reads the response body under
a byte limit within the same deadline, and returns one exhaustive `Exchange`.
What a status or a body means stays with the application.

```rust
use batter_outbound_http::{
    Body, Client, ConnectTimeout, Exchange, Origin, ResponseLimit,
    http::{HeaderName, HeaderValue, Method, StatusCode},
};
use std::time::Duration;

async fn example(token: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(
        Origin::https("https://provider.example")?,
        ConnectTimeout::new(Duration::from_secs(5))?,
    )?;
    let request = client
        .request(Method::POST, ["v1", "effects"])
        .header(HeaderName::from_static("authorization"), HeaderValue::from_str(token)?)
        .json(&serde_json::json!({ "effect": "example" }))
        .prepare()?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    match request.send(deadline, ResponseLimit::new(64 * 1024)?).await {
        // Provably unsent: the application may release its own dispatch fence.
        Exchange::NotStarted(_) | Exchange::NotDispatched(_) => {}
        // The provider may have acted: keep the fence and reconcile later.
        Exchange::Unknown(_) => {}
        // What a status and body mean is provider policy, decided here.
        Exchange::Responded(response) => match (response.status(), response.body()) {
            (StatusCode::OK, Body::Complete(_bytes)) => {}
            _ => {}
        },
    }
    Ok(())
}
```

The crate-level rustdoc compiles this example, and the `send_in` rustdoc compiles
its `operation` variant. Through the facade, select feature `outbound-http` and use
`batter::outbound_http`; the facade's `outbound_exchange` example runs a complete
consumer against a local fixture.

## Outcomes

| Outcome | Reasons and `label()` | What this attempt established |
| --- | --- | --- |
| `NotStarted` | `reason()`: `DeadlineTooClose` (`not_started.deadline`), `Cancelled` (`not_started.cancelled`) | The request was never handed to an HTTP connection. |
| `NotDispatched` | `failure()`: `TimedOut` (`not_dispatched.connect_timeout`), `Failed` (`not_dispatched.connect_failed`) | Connection establishment failed, so no request bytes were written. |
| `Unknown` | `Deadline` (`unknown.deadline`), `Cancelled` (`unknown.cancelled`), `Transport` (`unknown.transport`) | The request may have reached the peer, and no response head that the HTTP client accepted arrived. The request may also still reach the peer after this outcome is returned. |
| `Responded` | `Body::Complete` (`responded`), `TooLarge` (`responded.body_too_large`), `Failed(Deadline)` (`responded.body_deadline`), `Failed(Cancelled)` (`responded.body_cancelled`), `Failed(Transport)` (`responded.body_transport`) | A response head arrived. Its status is kept even when the body then fails. |

`Exchange` is exhaustive and `#[must_use]`, so a `match` needs no wildcard arm
that could absorb a later variant; a wildcard arm still compiles, and only a
consumer-side lint such as `clippy::wildcard_enum_match_arm` forbids one.
`proves_unsent()` is true only for `NotStarted` and `NotDispatched`, and only for
this attempt: it says nothing about earlier attempts or other requests in the
same operation. Only this package produces those two outcomes: their payloads
have no public constructor and are not `Clone` (a `compile_fail` doctest rejects
a hand-built `NotStarted`), so code that wraps an exchange cannot turn an
interruption into proof that nothing was sent. Read them through
`NotStarted::reason()` and `NotDispatched::failure()`, for example
`Exchange::NotDispatched(failure) if failure.failure() == ConnectFailure::Failed`.
`Unknown` stays constructible: it is the conservative value for an exchange that
a wrapper lost. Retrying with the same expired deadline or cancelled context
returns `NotStarted` again at once: of the two unsent outcomes, retry
only a `NotDispatched`, with a backoff and a fresh budget check. A status line
can come from an intermediary such as a load balancer, so `Responded` does not
show that the provider processed, accepted or refused the request. A 3xx is
returned as a response, never followed.

## The fixed client

`Client::new(origin, connect_timeout)` is the only constructor. A client fixes
one validated origin and shares only its TLS configuration when cloned. Every
exchange opens a fresh TCP connection, performs TLS for HTTPS, and uses hyper's
low-level HTTP/1 connection API. The exchange future drives that connection
itself; it never spawns a connection driver or writer. There is no pool,
connection reuse, resend, redirect, proxy lookup or content decoder. Every call
therefore pays connection and TLS setup. Feature unification with another
client's HTTP/2 or gzip support cannot change this transport.

Hostname setup resolves addresses explicitly. The first resolved family starts
first; the other family starts after at most 300 ms, or half the remaining setup
budget when shorter. An outright family failure starts the alternative at once.
Within each family the remaining budget is shared across its addresses. At most
two TCP attempts run together, both inside the exchange future, and losing
sockets are dropped before TLS or HTTP handoff. This is address fallback, not
HTTP replay. Controlled connection futures cover stalled addresses and families;
live dual-stack routing has not been exercised.

TLS keeps the explicit *ring* provider, TLS 1.3 and 1.2 only, ALPN `http/1.1`,
and the operating system verifier for HTTPS. Loopback HTTP clients use an empty
root store and never read the host certificate store. On Linux, HTTPS client
construction loads the host store (`SSL_CERT_FILE`/`SSL_CERT_DIR`); an empty
store returns `ClientError::Build`. On macOS, the platform verifier runs during
the handshake. No process-default provider is installed or needed here. Other
crates using default-provider rustls builders still need application-owned
configuration when their graph enables both providers.

`Origin::https` accepts `https://host` or `https://host:port` with an optional
trailing slash, and refuses credentials, a path, a query, a fragment, port zero
and a host that `http::Uri` rejects (`OriginError::Unparseable`).
`Origin::loopback` accepts plain HTTP only to a literal loopback socket address
with a nonzero port, for local fixtures and tests.

`client.request(method, segments)` starts a `RequestDraft` below the origin.
Each segment is percent-encoded, so `/`, `?`, `#` and `%` stay inside it: pass
`["v1", "effects"]`, not `["v1/effects"]`, which is sent as one encoded segment.
`header` marks every value it is given sensitive and refuses `Host`,
`Content-Length`, `Transfer-Encoding`, `Connection`, `Upgrade`, `TE`, `Trailer`,
`Expect`, `Keep-Alive`, `Proxy-Connection`, `Proxy-Authorization` and
`Content-Type`; the content type travels with `body` or `json`. `prepare()`
supplies `Accept: */*` only when the caller supplied no `Accept`; explicit values
keep their order and sensitivity. It
reports the first problem as a value-free `PrepareError` before an exchange
exists: an empty segment, or one whose part before a `;` is empty
(`EmptySegment`); a `.` or `..` segment, which URL normalization would drop
silently, or one whose part before a `;` is `.` or `..`, which servers that strip
`;` path parameters would read the same way (`DotSegment`); a segment containing
a tab, line feed or carriage return, which the URL parser deletes before it
applies its dot-segment rules, so that `.\t.` would become `..`
(`ControlCharacter`; other control characters are percent-encoded inside the
segment); a URL that is not a valid `http::Uri`, for example one longer than the
`http` crate accepts (`InvalidUri`); a reserved header; a second body; a JSON
serialization failure; the `CONNECT` method; or a builder failure (`Build`).
`Build` also covers a check that every segment survives URL normalization as
exactly one segment, which is defence in depth: no known input reaches it after
the refusals above. Nothing connects before `send`. `PreparedRequest` is not
`Clone`, and `send` consumes it, both checked by `compile_fail` doctests, so a
second attempt is a new preparation in the application's own code.

## What stays with the application

Call the exchange from the crate that owns the provider protocol. That crate
decides what a status and body mean, whether and when a request may be replayed,
idempotency keys, Retry-After, dispatch fences and their release, reconciliation
of `Unknown`, token acquisition, request signing, webhook ingress, which
destination an operator may configure, and what to record. Only an outcome for
which `proves_unsent()` holds can justify releasing a fence or refunding a
reservation without reconciliation. `Unknown` and a dropped send future keep the
reconciliation path, and reconciliation must tolerate a request that reaches the
provider after the exchange has returned. `Responded` needs provider-specific
interpretation before it means accepted, refused or retryable.

## Deadlines

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

## Limits

- `ResponseLimit` accepts 1 byte to 64 MiB. A declared or streamed length above
  it returns `TooLarge`; chunked framing is removed and content codings remain
  encoded. Preallocation is capped at 1 MiB and buffer growth stays within the
  limit. A body delimited only by connection close cannot reveal truncation.
- Deadlines are cooperative. Scheduling delay or synchronous certificate
  verification on macOS can delay return beyond `RESERVE`. A gate cannot
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

## Features

`operation` is off by default. Without it, the package has no Batter dependency,
so a provider crate that cannot depend on `batter-core` can still use the
exchange. With it, `PreparedRequest::send_in(&OperationContext, ResponseLimit)`
takes the context's deadline and cancellation: a context already cancelled
returns `NotStarted` with reason `Cancelled` before the deadline is checked,
matching `batter-core`'s precedence. Cancellation during connection setup also
returns `NotStarted(Cancelled)`; cancellation after handoff returns
`Unknown(Cancelled)` or `Body::Failed(Cancelled)`. Pass the work phase of a
context whose finalization is reserved, and record the outcome in the
finalization phase. A cancelled context, including one whose parent was
cancelled, runs no recording step, so the dispatch fence that the application
wrote before sending is what remains held. The facade's `outbound-http` feature
enables `operation`.

## Platform and verification

This crate follows the workspace's Unix-only platform policy; Windows is
unsupported and not planned. Earlier macOS results in `batter-8e8r` predate the
owned-connection transport. Current executed checks belong to `batter-u387`;
no new macOS or hosted evidence is claimed. The current lockfile resolves
hyper 1.11.1, hyper-util 0.1.20, tokio-rustls 0.26.6, rustls 0.23.46,
rustls-platform-verifier 0.7.1 and Tokio 1.54.1. reqwest 0.13.5 is dev-only.
The package's own tests compile its gzip/HTTP2 features as unification controls;
the facade example executes the owned transport without that control client.
Tests also exercise real TLS 1.2/1.3 with untrusted certificate rejection and a
private explicitly trusting verifier, including a readable response at expiry.
See `docs/references.md` at the workspace root for the inspected sources and
`docs/guarantees.md` for the contract and evidence.

Run `cargo test -p batter-outbound-http --all-features --locked`, and
`cargo test -p batter-outbound-http --no-default-features --locked` for the build
without `batter-core`, then the workspace verification script. The tests run
against a scripted loopback peer that records every connection and raw request.
Tests that build an HTTPS client need a non-empty host certificate store, which
on Linux follows `SSL_CERT_FILE` and `SSL_CERT_DIR`; loopback clients never read
it.
