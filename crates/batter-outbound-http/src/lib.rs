//! Send-once outbound HTTP exchange with exhaustive transport evidence.
//!
//! A provider call can fail in a way that leaves the caller unsure whether the
//! provider received the request. This package owns the transport part of that
//! question and nothing above it.
//!
//! - [`Client::new`] is the only way to build a client. Every client follows no
//!   redirects, offers HTTP/1.1 only, ignores proxy environment variables,
//!   requires a [`ConnectTimeout`] and sends only to its one validated
//!   [`Origin`]; [`Client`] marks which of its settings tests execute and
//!   which are defence in depth.
//! - [`RequestDraft::prepare`] reports every construction failure before an
//!   exchange exists. Header values are marked sensitive, framing headers are
//!   refused, and path segments cannot leave the origin.
//! - [`PreparedRequest::send`] consumes the request. It refuses to start when the
//!   caller's deadline is too close, reads the response body under a
//!   [`ResponseLimit`] within the same deadline, and returns one [`Exchange`].
//!
//! | Outcome | What this attempt established |
//! | --- | --- |
//! | [`Exchange::NotStarted`] | The request was never handed to an HTTP connection. |
//! | [`Exchange::NotDispatched`] | Connection setup failed or timed out before handoff, so no request bytes were written. |
//! | [`Exchange::Unknown`] | The request may have reached the peer, and no response head that the HTTP client accepted arrived. |
//! | [`Exchange::Responded`] | A response head arrived. Its status is kept even when the body then fails. |
//!
//! Only the first two prove that nothing was sent; see
//! [`Exchange::proves_unsent`]. A status line may come from an intermediary such
//! as a load balancer, so [`Exchange::Responded`] does not show that the
//! provider processed, accepted or refused the request. What a status or body
//! means, whether a request may be replayed, idempotency keys, dispatch fences
//! and reconciliation all stay with the application.
//!
//! ```no_run
//! use batter_outbound_http::{
//!     Body, Client, ConnectTimeout, Exchange, Origin, ResponseLimit,
//!     http::{HeaderName, HeaderValue, Method, StatusCode},
//! };
//! use std::time::Duration;
//!
//! # async fn example(token: &str) -> Result<(), Box<dyn std::error::Error>> {
//! let client = Client::new(
//!     Origin::https("https://provider.example")?,
//!     ConnectTimeout::new(Duration::from_secs(5))?,
//! )?;
//! let request = client
//!     .request(Method::POST, ["v1", "effects"])
//!     .header(HeaderName::from_static("authorization"), HeaderValue::from_str(token)?)
//!     .json(&serde_json::json!({ "effect": "example" }))
//!     .prepare()?;
//! let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
//! match request.send(deadline, ResponseLimit::new(64 * 1024)?).await {
//!     // Provably unsent: the application may release its own dispatch fence.
//!     Exchange::NotStarted(_) | Exchange::NotDispatched(_) => {}
//!     // The provider may have acted: keep the fence and reconcile later.
//!     Exchange::Unknown(_) => {}
//!     // What a status and body mean is provider policy, decided here.
//!     Exchange::Responded(response) => match (response.status(), response.body()) {
//!         (StatusCode::OK, Body::Complete(_bytes)) => {}
//!         _ => {}
//!     },
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [`Origin::public_https`] additionally rejects non-public literal addresses
//! and filters the exact DNS-resolved dial candidates before TCP setup. The
//! existing [`Origin::https`] permits deliberately private HTTPS services.
//!
//! # Owned connection
//!
//! `Client::new(origin, connect_timeout)` is the only constructor. A client fixes
//! one validated origin and shares only its TLS configuration when cloned. Every
//! exchange opens a fresh TCP connection, performs TLS for HTTPS, and uses hyper's
//! low-level HTTP/1 connection API. The exchange future drives that connection
//! itself; it never spawns a connection driver or writer. There is no pool,
//! connection reuse, resend, redirect, proxy lookup or content decoder. Every call
//! therefore pays connection and TLS setup. Feature unification with another
//! client's HTTP/2 or gzip support cannot change this transport.
//!
//! Hostname setup retains bounded address fallback inside this future: the
//! other address family starts after at most 300 ms or half the remaining setup
//! budget, and addresses within each family share its remaining time. At most
//! two TCP candidates run together; losers drop before TLS or HTTP handoff.
//! Preparation keeps the default `Accept: */*` when callers omit `Accept`, and
//! preserves their explicit values and sensitivity.
//!
//! TLS keeps the explicit *ring* provider, TLS 1.3 and 1.2 only, ALPN `http/1.1`,
//! and the operating system verifier for HTTPS. Loopback HTTP clients use an empty
//! root store and never read the host certificate store. On Linux, HTTPS client
//! construction loads the host store (`SSL_CERT_FILE`/`SSL_CERT_DIR`); an empty
//! store returns `ClientError::Build`. On macOS, the platform verifier runs during
//! the handshake. No process-default provider is installed or needed here. Other
//! crates using default-provider rustls builders still need application-owned
//! configuration when their graph enables both providers.
//!
//! # Deadlines
//!
//! The exchange deadline is the caller's deadline minus `RESERVE` (50 ms).
//! Connection setup begins only with at least `MINIMUM_ATTEMPT` (50 ms) left before
//! that exchange deadline. DNS, TCP, TLS and HTTP/1 setup have one connect timeout
//! of more than zero and at most 60 seconds, inside the same exchange deadline.
//!
//! Before handoff, cancellation wins over the exchange timer, which wins over
//! connection progress. The final clock/cancellation check sits immediately before
//! `send_request`, with no await between them. An expired or cancelled exchange
//! that has not handed its request to the connection returns `NotStarted`, even
//! when TCP or TLS setup already began. A setup error or connect timeout returns
//! `NotDispatched`. These outcomes follow owned phases; they do not infer dispatch
//! from reqwest or hyper-util error kinds.
//!
//! After handoff, an available response or body frame wins over cancellation and
//! the timer, and cancellation wins over the timer. Each write, vectored write,
//! flush and shutdown checks the gate before polling the underlying transport.
//! The TCP stream under TLS is guarded too, because a TLS read can write pending
//! records. After stopping, the connection can still read an available reply but
//! cannot advance its writer. A queued or partially written request therefore
//! cannot resume writing when a stalled exchange is polled after its deadline or
//! cancellation. No detached connection task can progress while the exchange is
//! unpolled. Without a response head, interruption after handoff is `Unknown`;
//! a received status survives every body failure.
//!
//! With `operation`, await `send_in(context, limit)` directly in the work phase of
//! `reserve_finalization`, then record the outcome in the finalization phase.
//! `RESERVE` lets classification precede an outer deadline; it is not a recording
//! budget. Dropping a send future drops its connection and produces no outcome:
//! treat a lost exchange as `Unknown`. An outer `run`, `retry`, `timeout` or
//! `select!` can still discard a ready exchange according to its own precedence.
//! Sending requires a Tokio runtime with its time driver enabled.
//!
//! # Response-body storage
//!
//! A response body can hold credentials or personal data, so this package
//! overwrites the heap storage it allocates for a body with zeros before
//! releasing that storage:
//!
//! - when the read stops early, with [`Body::TooLarge`] or any [`Body::Failed`];
//! - when the exchange future is dropped while reading the body;
//! - when the buffer grows, for the allocation being replaced; and
//! - for [`Body::Complete`], when the last [`Bytes`] handle is released. Clones
//!   and slices, on any thread, share one owner and stay intact until then.
//!
//! The guarantee stops at that storage. It does not reach hyper's read buffer,
//! rustls or kernel buffers, response headers or request bodies. A copy is the
//! caller's to wipe, including `Bytes::to_vec`, `Vec::from(bytes)`,
//! `BytesMut::from(bytes)` and parsed values. A returned body is never uniquely
//! owned, so `Bytes::try_into_mut` returns `Err`, and `Vec::from` and
//! `BytesMut::from` always copy instead of reclaiming the allocation.
//!
//! The wipe runs in a destructor, so a leaked or forgotten handle, an abort and
//! process exit skip it. Live bytes are not locked in memory, so swap, hibernation
//! or a core dump can capture them. Wiping takes time proportional to the
//! allocation and runs without yielding on the thread that releases it: about
//! 18 ms for 64 MiB in one release-build measurement.
//!
//! # Limits
//!
//! - `ResponseLimit` accepts 1 byte to 64 MiB. A declared or streamed length above
//!   it returns `TooLarge`; chunked framing is removed and content codings remain
//!   encoded. Preallocation is capped at 1 MiB and buffer growth stays within the
//!   limit. Growth copies into a new allocation before releasing the old one, so two
//!   allocations briefly coexist, together below twice the limit. A completed body
//!   keeps the allocation it grew into, at most twice its length and never above the
//!   limit, until its last handle is released. A body delimited only by connection
//!   close cannot reveal truncation.
//! - Deadlines are cooperative. Scheduling delay, synchronous certificate
//!   verification on macOS or growing a large body's buffer can delay return beyond
//!   `RESERVE`. One growth step copies and wipes the old allocation without
//!   yielding. Separate release-build measurements of copying and wiping 64 MiB
//!   give an estimated combined cost of about 56 ms (38 ms + 18 ms). A gate cannot
//!   interrupt an already-entered poll or eliminate thread preemption between a
//!   clock check and a socket operation. The handoff and write checks have no
//!   intervening await. Bytes already accepted by the kernel can arrive later,
//!   including through TCP retransmission. `Unknown` remains necessary.
//! - A request that fails after handoff remains conservatively `Unknown`, even if
//!   no byte was actually written. Only owned pre-handoff phases prove unsent.
//! - No formatted value from this package includes URLs, queries, header values or
//!   bodies. Response accessors retain the actual headers and body; header values
//!   are sensitive. A query still reaches the provider and intermediaries.
//! - The package adds no instrumentation. `label()` provides twelve fixed labels.
//!   rustls-platform-verifier can log certificate failures and names; rustls can
//!   log names and certificates when another dependency enables `logging`.
//!   Application logger configuration owns filtering. The outbound transport no
//!   longer invokes reqwest or hyper-util's legacy client, so their destination
//!   and pool logs are absent from this path.
//! - Provider semantics, replay authorization, idempotency keys, Retry-After,
//!   dispatch fences and reconciliation remain application-owned. HTTP/2, proxies,
//!   signing, token acquisition, certificate pinning and operator-supplied
//!   destinations remain unsupported.

#![forbid(unsafe_code)]

mod client;
mod exchange;
mod limits;
#[cfg(feature = "operation")]
mod operation;
mod origin;
mod request;
mod send;
mod transport;

pub use bytes::Bytes;
/// The `http` 1.x types used in this package's public API.
pub use http;

pub use client::{Client, ClientError};
pub use exchange::{
    Body, BodyFailure, ConnectFailure, Exchange, NotDispatched, NotStarted, Response, StartRefusal,
    UnknownReason,
};
pub use limits::{ConnectTimeout, LimitError, ResponseLimit};
pub use origin::{Origin, OriginError};
pub use request::{PrepareError, PreparedRequest, RequestDraft};
pub use send::{MINIMUM_ATTEMPT, RESERVE};
