//! The closed outcome of one exchange.

use bytes::Bytes;
use http::{HeaderMap, StatusCode};
use std::fmt;

/// What one send established. Every variant is local evidence about this
/// attempt, never a statement about what the provider did.
///
/// The enum is exhaustive: a new variant would be a breaking change, so a
/// `match` over it needs no wildcard arm that could silently absorb one.
///
/// Only this package produces the two outcomes that prove nothing was sent:
/// [`NotStarted`] and [`NotDispatched`] have no public constructor, so code
/// that wraps an exchange cannot turn an interruption into proof that a request
/// was unsent. A wrapper that loses an exchange may build
/// `Exchange::Unknown(..)`, the conservative outcome.
///
/// ```compile_fail,E0451
/// use batter_outbound_http::{Exchange, NotStarted, StartRefusal};
/// let forged = Exchange::NotStarted(NotStarted { reason: StartRefusal::Cancelled });
/// ```
#[must_use = "an exchange records whether the peer may have received the request"]
#[derive(Debug)]
pub enum Exchange {
    /// The request was never handed to an HTTP connection.
    NotStarted(NotStarted),
    /// Connection establishment failed, so this attempt wrote no request bytes.
    NotDispatched(NotDispatched),
    /// The request may have reached the peer, and no response head that the
    /// HTTP client accepted arrived. The request may also still reach the peer
    /// after this outcome is returned, so reconciliation must tolerate a late
    /// arrival.
    Unknown(UnknownReason),
    /// A response head arrived. Its status may come from an intermediary, so
    /// this does not show that the provider processed the request.
    Responded(Response),
}

impl Exchange {
    /// `true` only for [`Exchange::NotStarted`] and [`Exchange::NotDispatched`],
    /// the outcomes proving that this attempt wrote no request bytes.
    ///
    /// It says nothing about earlier attempts or other requests in the same
    /// operation. A [`StartRefusal`] stands for the deadline or context it was
    /// given and returns without yielding, so sending again with the same
    /// deadline or a cancelled context returns it again at once: retry a
    /// [`ConnectFailure`] only, with a backoff and a fresh budget check.
    pub fn proves_unsent(&self) -> bool {
        matches!(self, Self::NotStarted(_) | Self::NotDispatched(_))
    }

    /// A stable, value-free label for logs and metrics, such as
    /// `"not_dispatched.connect_failed"` or `"responded.body_too_large"`.
    pub fn label(&self) -> &'static str {
        match self {
            Self::NotStarted(refusal) => match refusal.reason {
                StartRefusal::DeadlineTooClose => "not_started.deadline",
                StartRefusal::Cancelled => "not_started.cancelled",
            },
            Self::NotDispatched(failure) => match failure.failure {
                ConnectFailure::TimedOut => "not_dispatched.connect_timeout",
                ConnectFailure::Failed => "not_dispatched.connect_failed",
            },
            Self::Unknown(UnknownReason::Deadline) => "unknown.deadline",
            Self::Unknown(UnknownReason::Cancelled) => "unknown.cancelled",
            Self::Unknown(UnknownReason::Transport) => "unknown.transport",
            Self::Responded(response) => match response.body {
                Body::Complete(_) => "responded",
                Body::TooLarge => "responded.body_too_large",
                Body::Failed(BodyFailure::Deadline) => "responded.body_deadline",
                Body::Failed(BodyFailure::Cancelled) => "responded.body_cancelled",
                Body::Failed(BodyFailure::Transport) => "responded.body_transport",
            },
        }
    }
}

/// Evidence that the request was never handed to an HTTP connection.
///
/// Only this package constructs it, and it is not `Clone`.
#[derive(Debug, PartialEq, Eq)]
pub struct NotStarted {
    reason: StartRefusal,
}

impl NotStarted {
    pub(crate) fn new(reason: StartRefusal) -> Self {
        Self { reason }
    }

    /// Why the exchange did not start.
    pub fn reason(&self) -> StartRefusal {
        self.reason
    }
}

/// Evidence that connection establishment failed before any request byte of
/// this attempt was written.
///
/// Only this package constructs it, and it is not `Clone`.
#[derive(Debug, PartialEq, Eq)]
pub struct NotDispatched {
    failure: ConnectFailure,
}

impl NotDispatched {
    pub(crate) fn new(failure: ConnectFailure) -> Self {
        Self { failure }
    }

    /// How connection establishment failed.
    pub fn failure(&self) -> ConnectFailure {
        self.failure
    }
}

/// Why an exchange did not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartRefusal {
    /// Less than `RESERVE + MINIMUM_ATTEMPT` remained at entry, or the
    /// exchange deadline expired before handoff to the connection.
    DeadlineTooClose,
    /// The operation context was cancelled before handoff (`operation` feature).
    Cancelled,
}

/// How connection establishment failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectFailure {
    /// The [`crate::ConnectTimeout`] elapsed (tested for a stalled TLS
    /// handshake). An operating-system connect timeout is classified the same
    /// way by source inspection, not by a test.
    TimedOut,
    /// The TCP connection or the TLS handshake failed (both tested). A DNS
    /// resolution failure and an HTTP/1 setup failure are classified the same
    /// way by source inspection, not by a test.
    Failed,
}

/// Why a possibly sent request has no response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnknownReason {
    /// The exchange deadline passed after handoff, before a response head arrived.
    Deadline,
    /// The operation context was cancelled after handoff (`operation` feature).
    Cancelled,
    /// The connection failed after the request was handed to it, the peer
    /// closed it without a complete response head, or the HTTP client rejected
    /// the head that arrived (for example a malformed head, which is tested, or
    /// too many headers or a conflicting `Content-Length`, source-inspected). By
    /// source inspection, even an unwritten request that fails after handoff
    /// is conservatively unknown. No error-kind inference grants unsent proof.
    Transport,
}

/// A received response head and the result of reading its body.
pub struct Response {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Body,
}

impl Response {
    /// The received status code.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// The received headers. Every value is marked sensitive, so `Debug` output
    /// of the map hides it.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// The result of reading the body.
    pub fn body(&self) -> &Body {
        &self.body
    }

    /// Take the status, headers and body.
    pub fn into_parts(self) -> (StatusCode, HeaderMap, Body) {
        (self.status, self.headers, self.body)
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Response")
            .field("status", &self.status.as_u16())
            .field("headers", &self.headers.len())
            .field("body", &self.body)
            .finish()
    }
}

/// The result of reading a response body under the exchange's limit and
/// deadline.
pub enum Body {
    /// The body as received, within the [`crate::ResponseLimit`], after
    /// HTTP/1.1 chunked framing is removed. A response with neither
    /// `Content-Length` nor chunked framing ends when the peer closes the
    /// connection, so its truncation cannot be detected.
    Complete(Bytes),
    /// The declared or streamed length exceeded the [`crate::ResponseLimit`].
    TooLarge,
    /// Reading stopped before the end of the body.
    Failed(BodyFailure),
}

impl Body {
    /// The bytes of a complete body.
    pub fn complete(&self) -> Option<&Bytes> {
        match self {
            Self::Complete(bytes) => Some(bytes),
            Self::TooLarge | Self::Failed(_) => None,
        }
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Complete(bytes) => write!(formatter, "Complete({} bytes)", bytes.len()),
            Self::TooLarge => formatter.write_str("TooLarge"),
            Self::Failed(failure) => formatter.debug_tuple("Failed").field(failure).finish(),
        }
    }
}

/// Why reading a body stopped early.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyFailure {
    /// The exchange deadline passed while reading.
    Deadline,
    /// The operation context was cancelled while reading (`operation` feature).
    Cancelled,
    /// The connection failed, or the body ended before its declared length.
    Transport,
}
