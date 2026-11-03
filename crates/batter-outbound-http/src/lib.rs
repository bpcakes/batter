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
//! | [`Exchange::NotStarted`] | The request was never handed to the HTTP client. |
//! | [`Exchange::NotDispatched`] | Connection establishment failed (TCP, TLS or the connect timeout, which tests execute; DNS and pool checkout by source inspection), so no request bytes were written. |
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
//! # Deadlines
//!
//! The exchange keeps [`RESERVE`] free at the end of the caller's deadline, so it
//! returns its classification before that deadline and an outer timer at the
//! same deadline does not discard it. `RESERVE` is not a recording budget:
//! record a durable outcome in a separately reserved finalization phase. The
//! exchange refuses to start when less than `RESERVE + MINIMUM_ATTEMPT`
//! remains. One timer bounds the response head and the bounded body read. When
//! a response or body chunk is ready at the same time as the timer or
//! cancellation, the ready result is kept: polling an already dispatched request
//! cannot start another one, because redirects and replays are disabled. Tests
//! force that tie for a response head and for the end of an empty body; a tie
//! with a non-empty body chunk follows the same select order but is not
//! forced by a test.
//! Sending requires a Tokio runtime with its time driver enabled.
//!
//! With the `operation` feature, `PreparedRequest::send_in` takes the deadline
//! and cancellation of a `batter_core::operation::OperationContext`. A context
//! cancelled before the start returns [`StartRefusal::Cancelled`]; cancellation
//! after the start returns [`UnknownReason::Cancelled`] or
//! [`BodyFailure::Cancelled`]. A cancelled context cannot run a recording step,
//! so the dispatch fence an application writes before sending is what remains
//! held.
//!
//! # Limits
//!
//! - [`Exchange::NotDispatched`] relies on hyper-util assigning its connect error
//!   kind only to connector and pool-checkout failures, before a request is
//!   handed to a connection. That is implementation behavior, not a documented
//!   upstream contract. The fault tests ran against the versions this
//!   repository's lockfile resolves (reqwest 0.13.5, hyper 1.11.1, hyper-util
//!   0.1.20). The manifest requires reqwest `^0.13.5` and does not constrain
//!   hyper or hyper-util, so an application's lockfile may resolve releases
//!   these tests never ran against: keep those versions, or re-run the tests
//!   after upgrading them.
//! - On macOS, certificate verification for HTTPS calls the operating system
//!   synchronously inside the poll that drives the handshake, and that call may
//!   fetch over the network. Neither the connect timeout nor the exchange timer
//!   can interrupt it, so the `RESERVE` bound does not hold while it blocks.
//!   This is source-inspected; no test has executed on macOS.
//! - Some attempts that wrote nothing, or that the server never processed, are
//!   reported as [`Exchange::Unknown`]. A deadline that expires during the TLS
//!   handshake is tested; expiry during DNS or TCP setup is not exercised. A
//!   keep-alive connection that
//!   the server closes just as the request is written, and an HTTP/1 handshake
//!   failure, are source-inspected. Unknown never claims more than the
//!   evidence supports.
//! - When a pooled connection turns out to be closed before a request was
//!   written to it, hyper-util sends that request again on a new connection
//!   (source-inspected, not observable through reqwest). This resends something
//!   the peer never received; it is not a replay.
//! - Dropping the send future after the request was handed to the client
//!   produces no outcome (no test drops one). Treat a dropped exchange as
//!   [`Exchange::Unknown`].
//! - The package adds no instrumentation of its own; [`Exchange::label`]
//!   returns a value-free label to record. Its dependencies emit `log` and
//!   `tracing` events: reqwest logs the destination host at debug level,
//!   hyper-util logs the origin's authority and each socket address it
//!   connects to at debug and trace level, and rustls-platform-verifier logs a
//!   failed certificate verification at error level, with the expected and
//!   presented names for a name mismatch. When another crate in the
//!   application enables rustls's `logging` feature, a default of rustls,
//!   tokio-rustls and hyper-rustls, rustls also logs the server name at debug
//!   level and the ClientHello and the server certificate chain at trace
//!   level. None of them carries an exchange's path, query, header value or
//!   body (source-inspected). They appear only through a logger or subscriber
//!   the application installs, whose configuration owns their filtering.
//! - HTTP/2, request signing, token acquisition, proxies and operator-supplied
//!   destinations are not supported. Unix only.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod client;
mod exchange;
mod limits;
#[cfg(feature = "operation")]
mod operation;
mod origin;
mod request;
mod send;

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
