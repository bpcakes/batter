# batter-outbound

Send-once outbound HTTP exchange with exhaustive transport evidence. This
experimental 0.0.1 package targets Unix, including Linux and macOS; its tests
have executed on Linux only, and macOS execution is unverified.

A provider call can fail in a way that leaves the caller unsure whether the
provider received the request. This package owns the transport part of that
question and nothing above it: it builds the HTTP client itself, sends each
prepared request once under the caller's deadline, reads the response body under
a byte limit within the same deadline, and returns one exhaustive `Exchange`.
What a status or a body means stays with the application.

```rust
use batter_outbound::{
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
its `operation` variant. Through the facade, select feature `outbound` and use
`batter::outbound`; the facade's `outbound_exchange` example runs a complete
consumer against a local fixture.

## Outcomes

| Outcome | Reasons and `label()` | What this attempt established |
| --- | --- | --- |
| `NotStarted` | `reason()`: `DeadlineTooClose` (`not_started.deadline`), `Cancelled` (`not_started.cancelled`) | The request was never handed to the HTTP client. |
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
a wrapper lost. A `NotStarted` stands for the deadline or context it was given
and returns without yielding, so sending again with the same deadline or a
cancelled context returns it again at once: of the two unsent outcomes, retry
only a `NotDispatched`, with a backoff and a fresh budget check. A status line
can come from an intermediary such as a load balancer, so `Responded` does not
show that the provider processed, accepted or refused the request. A 3xx is
returned as a response, never followed.

## The fixed client

`Client::new(origin, connect_timeout)` is the only way to build a client, and a
built client cannot be reconfigured. Every client:

- follows no redirects (tested);
- offers only `http/1.1` in its TLS ALPN list, even when another crate enables
  reqwest's HTTP/2 (tested with reqwest's `http2` compiled), and sends HTTP/1.1
  requests;
- turns off reqwest's response decompression (tested for gzip, the only decoder
  compiled here; Brotli, zstd and deflate are source-inspected);
- ignores proxy environment variables (`HTTP_PROXY` and `http_proxy` are tested
  for a loopback origin; `HTTPS_PROXY` and `ALL_PROXY` for an HTTPS origin are
  not executed);
- turns off the hickory resolver (source-inspected; the feature is not compiled
  here);
- never reuses a connection idle for more than four seconds (tested), although
  hyper-util may keep such a connection open for up to about eight seconds
  before it evicts it (source-inspected).

Also set as defence in depth, not observable while redirects are off and only
HTTP/1.1 is offered: no `Referer`, reqwest's own retries disabled and
`http1_only()`. It has no client-wide total or read timeout. Cloning shares one
connection pool; `Debug` prints `Client { .. }`.

TLS uses rustls with the *ring* crypto provider passed explicitly, TLS 1.3 and
1.2 only and the operating system's certificate verifier, in a configuration the
package builds itself and reqwest's `rustls-no-provider` feature uses as given,
so a TLS or provider feature that another crate enables does not change it
(source-inspected). The ClientHello offers only `http/1.1` and TLS 1.3 or 1.2
(tested; rustls supports nothing older). The package never installs or relies on
a process-default crypto provider (tested: none is installed after building the
client and sending a ClientHello; no test completes a handshake) and adds no
aws-lc-rs. `https_only` is set for
an HTTPS origin as defence in depth, since every request URL comes from the
origin. An HTTPS client loads the host certificate store at construction; on
Linux that store follows `SSL_CERT_FILE` and `SSL_CERT_DIR`, and an empty one
makes `Client::new` return `ClientError::Build` (tested on Linux). An
`Origin::loopback` client is plain HTTP, trusts no roots and never reads that
store (tested on Linux). No test verifies a certificate. If an application's
graph also compiles aws-lc-rs into rustls, for example because another crate
enables reqwest's `rustls` feature, crates that call
`rustls::ClientConfig::builder()` without a provider panic unless the
application root installs a process default with
`rustls::crypto::CryptoProvider::install_default`; a plain
`reqwest::Client::new()` elsewhere also panics without one when the graph's
reqwest 0.13 has only `rustls-no-provider` (neither its `rustls` nor its
`native-tls` feature). This package needs neither.

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

- `send(deadline, limit)` keeps `RESERVE` (50 ms) free at the end of `deadline`:
  the exchange ends at `deadline - RESERVE`, so it returns its classification
  before the caller's deadline, apart from scheduling delay and the macOS case
  under [Limits](#limits), and an outer timer at the same deadline does not
  discard it. `RESERVE` is not a recording budget: record a durable outcome in a
  separately reserved finalization phase.
- It returns `NotStarted` with reason `DeadlineTooClose` without touching the
  HTTP client when less than `RESERVE + MINIMUM_ATTEMPT` (100 ms) remains. That
  check runs immediately before the first poll of the request, with no await
  between them; review guards that ordering, because no test would detect an
  inserted await.
- One timer bounds the response head and the body read together. Deadline
  reasons come only from this package's clock: reqwest's per-request timeout is
  never set, and `reqwest::Error::is_timeout`, which also matches an operating
  system connection timeout, only separates `ConnectFailure::TimedOut` from
  `Failed`.
- A response head, or the end of a body, that is ready together with the timer
  or a cancellation is kept, and a cancellation that is ready together with the
  expired timer is reported, both before a head and during the body. Polling a
  request that was already handed to the client cannot start another one,
  because redirects and replays are disabled. This precedence differs
  deliberately from `OperationContext::run`, which prefers cancellation and then
  the deadline over completion for arbitrary futures. Tests force each of these
  ties; a ready non-empty body chunk is read before the timer or a cancellation
  by source inspection only, so review still guards the order of the biased
  selection for that case.
- When less time remains than the connect timeout, the deadline can expire while
  a connection is being established. That is `Unknown(Deadline)`, because the
  exchange cannot tell which phase was running. Expiry during the TLS handshake
  is tested; expiry during DNS resolution or TCP setup is not exercised.
- Dropping the send future after the request was handed to the client produces
  no outcome. Treat a dropped exchange as `Unknown`, including one that an outer
  `run`, `retry`, `timeout` or `select!` dropped, so await `send_in` directly in
  the context that bounds it. No test drops an exchange.
- `send` and `send_in` must run inside a Tokio runtime with its time driver
  enabled.

## Limits

- `ResponseLimit` accepts 1 byte to 64 MiB. A declared `Content-Length` above it
  is `TooLarge` without reading; a body that grows past it stops at the first
  chunk that would exceed it. The limit counts message-body bytes as received:
  chunked framing is removed and content codings are not decoded. A response
  with neither `Content-Length` nor chunked framing ends when the peer closes the
  connection, so its truncation cannot be detected and it is `Complete` (tested).
  Preallocation from a declared length is capped at 1 MiB, the buffer never grows
  past the limit, and spare capacity is released before `Body::Complete`
  (source-inspected, not tested). There is no streaming request or response body.
- `ConnectTimeout` accepts more than zero and at most 60 seconds and covers DNS,
  TCP and TLS together; tests exercise only the TLS phase.
- `NotDispatched` relies on hyper-util assigning its connect error kind only to
  connector and pool-checkout failures, before the request is handed to a
  connection, as surfaced by reqwest's `Error::is_connect`. That is
  implementation behaviour, not a documented upstream contract: reqwest
  documents `is_connect` only as "related to connect". The tests execute a
  refused TCP connection and a failed or stalled TLS handshake; a DNS failure, an
  operating-system connect timeout and a pool-checkout failure are classified the
  same way by source inspection only. The fault tests ran against the versions
  this repository's lockfile resolves (reqwest 0.13.5, hyper 1.11.1, hyper-util
  0.1.20). The manifest requires reqwest `^0.13.5` and does not constrain hyper
  or hyper-util, so an application's lockfile may resolve releases these tests
  never ran against: keep those versions, or re-run the tests after upgrading
  them.
- On macOS, certificate verification for HTTPS calls the operating system
  synchronously inside the poll that drives the handshake, and that call may
  fetch over the network. Neither the connect timeout nor the exchange timer can
  interrupt it, so the `RESERVE` bound does not hold while it blocks. This is
  source-inspected: no test has executed on macOS, where the CI definition only
  compiles the package.
- Some requests the peer never processed are reported as `Unknown`: a deadline
  that expires during the TLS handshake (tested) and, by source inspection only,
  a request written to a keep-alive connection that the server is closing, an
  HTTP/1 handshake or readiness failure after the connector, and a request that
  hyper-util hands back unsent without resending. `Unknown(Transport)` also
  covers a malformed or truncated head (tested) and a head that the HTTP client
  rejects, such as one with too many headers or a conflicting `Content-Length`
  (source-inspected). `Unknown` never claims more than the evidence supports.
- When a reused pooled connection turns out to be closed before the request was
  written to it, hyper-util sends the request again on a new connection. reqwest
  does not expose that setting. It resends something the peer never received; it
  is not a replay, and the number of wire attempts is not observable. This is
  source-inspected; no test reuses a closed pooled connection.
- A query string still reaches the provider and every intermediary; the
  exchange only keeps it out of its own values. Prefer headers or the body for
  credentials when the provider allows it.
- No `Debug` or `Display` output of this package includes a URL, query, header
  value or body. A `Response` still holds the received headers and body:
  `Response::headers` and `Body` return them only through explicit accessors,
  with header values marked sensitive.
- The package adds no instrumentation of its own. Record `Exchange::label()`.
  Its dependencies do emit `log` and `tracing` events (source-inspected):
  reqwest logs the destination host at debug level, hyper-util logs the
  origin's authority and each socket address it connects to at debug and trace
  level, and rustls-platform-verifier logs a failed certificate verification
  at error level, including the expected and presented names for a name
  mismatch. When another crate in the application's graph enables rustls's
  `logging` feature, which rustls, tokio-rustls and hyper-rustls enable by
  default, rustls also logs the server name at debug level and the
  ClientHello, with its server name, and the server certificate chain at trace
  level. None of them carries an exchange's path, query, header value or
  body. They appear only through a logger or subscriber the application
  installs; filter the `reqwest`, `hyper_util`, `rustls_platform_verifier` and
  `rustls` targets there. The
  [references](../../docs/references.md#reqwest-0135-and-hyper-util-0120-for-the-outbound-exchange-2026-11-02)
  cite each event.
- Each exchange covers one request. Whether an earlier request of the same
  operation was sent stays application evidence. A hand-built reqwest client
  bypasses every rule above; only a consumer-side lint can forbid that.
- HTTP/2, proxies, request signing, token acquisition, certificate pinning and
  operator-supplied destinations are not supported.

## Features

`operation` is off by default. Without it, the package has no Batter dependency,
so a provider crate that cannot depend on `batter-core` can still use the
exchange. With it, `PreparedRequest::send_in(&OperationContext, ResponseLimit)`
takes the context's deadline and cancellation: a context already cancelled
returns `NotStarted` with reason `Cancelled` before the deadline is checked,
matching `batter-core`'s precedence, and cancellation after the start returns
`Unknown(Cancelled)` or `Body::Failed(Cancelled)`. Pass the work phase of a
context whose finalization is reserved, and record the outcome in the
finalization phase. A cancelled context, including one whose parent was
cancelled, runs no recording step, so the dispatch fence that the application
wrote before sending is what remains held. The facade's `outbound` feature
enables `operation`.

## Platform and verification

This crate follows the workspace's Unix-only platform policy; Windows is
unsupported and not planned. Its tests have executed on Linux only. They ran
against the versions this repository's lockfile resolves: reqwest 0.13.5
(default features off, `rustls-no-provider`), rustls 0.23.46 (`ring`),
rustls-platform-verifier 0.7.1, hyper-util 0.1.20, hyper 1.11.1 and url 2.5.8.
Test builds also compile reqwest's `gzip` and `http2` features through this
package's dev-dependencies, as an unrelated dependency could through feature
unification, so the package's own tests always run with them compiled and show
the profile overriding them. The shipped feature set is compile-checked without
them and executed by the facade example's tests in the verification matrix's
`outbound-shipped-features` batch. See `docs/references.md` at the workspace
root for the inspected sources, and `docs/guarantees.md` for the contract and
the tests that back it.

Run `cargo test -p batter-outbound --all-features --locked`, and
`cargo test -p batter-outbound --no-default-features --locked` for the build
without `batter-core`, then the workspace verification script. The tests run
against a scripted loopback peer that records every connection and raw request.
Tests that build an HTTPS client need a non-empty host certificate store, which
on Linux follows `SSL_CERT_FILE` and `SSL_CERT_DIR`; loopback clients never read
it.
