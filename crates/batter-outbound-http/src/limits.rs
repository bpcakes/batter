//! Validated transport limits.

use std::time::Duration;

/// The largest response body one exchange buffers.
///
/// A declared `Content-Length` above the limit is refused before reading, and
/// a body that grows past it while streaming stops at the first chunk that
/// would exceed it. Both report [`crate::Body::TooLarge`] with the status kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResponseLimit(usize);

impl ResponseLimit {
    /// The largest accepted limit, 64 MiB.
    pub const MAX_BYTES: usize = 64 * 1024 * 1024;

    /// Accept a limit from one byte to [`ResponseLimit::MAX_BYTES`].
    ///
    /// ```
    /// use batter_outbound_http::{LimitError, ResponseLimit};
    ///
    /// assert_eq!(ResponseLimit::new(64 * 1024).unwrap().bytes(), 65_536);
    /// assert_eq!(ResponseLimit::new(0), Err(LimitError::Zero));
    /// ```
    pub fn new(bytes: usize) -> Result<Self, LimitError> {
        if bytes == 0 {
            Err(LimitError::Zero)
        } else if bytes > Self::MAX_BYTES {
            Err(LimitError::AboveMaximum)
        } else {
            Ok(Self(bytes))
        }
    }

    /// The limit in bytes.
    pub fn bytes(self) -> usize {
        self.0
    }
}

/// How long connection setup may take: DNS, TCP, TLS and HTTP/1 together.
///
/// One timeout covers the exchange-owned setup future, including address
/// fallback, within the exchange's absolute deadline.
///
/// A setup timeout returns [`crate::Exchange::NotDispatched`] with
/// [`crate::ConnectFailure::TimedOut`], proving no request bytes were written.
/// Deadline expiry or cancellation before the HTTP request handoff instead
/// returns [`crate::Exchange::NotStarted`]. Successful setup is checked against
/// both clocks before the HTTP request handoff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectTimeout(Duration);

impl ConnectTimeout {
    /// The longest accepted connect timeout, 60 seconds.
    pub const MAX: Duration = Duration::from_secs(60);

    /// Accept a timeout above zero and at most [`ConnectTimeout::MAX`].
    ///
    /// ```
    /// use batter_outbound_http::{ConnectTimeout, LimitError};
    /// use std::time::Duration;
    ///
    /// assert!(ConnectTimeout::new(Duration::from_secs(5)).is_ok());
    /// assert_eq!(ConnectTimeout::new(Duration::ZERO), Err(LimitError::Zero));
    /// ```
    pub fn new(timeout: Duration) -> Result<Self, LimitError> {
        if timeout.is_zero() {
            Err(LimitError::Zero)
        } else if timeout > Self::MAX {
            Err(LimitError::AboveMaximum)
        } else {
            Ok(Self(timeout))
        }
    }

    /// The timeout.
    pub fn get(self) -> Duration {
        self.0
    }
}

/// Why a limit was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LimitError {
    /// A zero limit would refuse every exchange.
    #[error("the limit must be greater than zero")]
    Zero,
    /// The limit is above its documented maximum.
    #[error("the limit is above its maximum")]
    AboveMaximum,
}
