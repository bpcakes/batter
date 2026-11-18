//! The validated bounds of one owned response body.

use batter_core::ConfigurationError;
use std::time::Duration;
use tokio::time::Instant;

/// Validated bounds for one owned response body.
///
/// `total` bounds the whole body from the moment the handler hands it over.
/// `idle` bounds each wait for the body's next frame, from the poll that first
/// asks for it, so a source that stops producing, or spends that long inside
/// one poll, is ended long before a generous total. Both are required: a body
/// that outlives its handler has no other bound.
///
/// A stream meant to run indefinitely still needs a total: choose the longest
/// connection the service accepts, at most one year. Reaching it ends the
/// body with an error at its next poll and is logged at WARN. What a peer
/// then sees is the transport's to decide; the cases exercised are listed at
/// [`AdmittedRequest::into_streaming_response`](crate::AdmittedRequest::into_streaming_response).
/// A stream whose peer should instead see a clean end finishes by itself
/// shortly before, for example when [`BodyScope::context`](crate::BodyScope::context)'s `remaining()`
/// runs low.
///
/// ```
/// use batter_axum::BodyBudget;
/// use std::time::Duration;
///
/// let budget = BodyBudget::new(Duration::from_secs(3600), Duration::from_secs(30))?;
/// assert_eq!(budget.total(), Duration::from_secs(3600));
/// assert_eq!(budget.idle(), Duration::from_secs(30));
/// // A wait that could never expire before the total is a configuration error.
/// assert!(BodyBudget::new(Duration::from_secs(1), Duration::from_secs(2)).is_err());
/// assert!(BodyBudget::new(Duration::from_secs(1), Duration::from_secs(1)).is_err());
/// assert!(BodyBudget::new(Duration::ZERO, Duration::ZERO).is_err());
/// # Ok::<(), batter_core::ConfigurationError>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyBudget {
    total: Duration,
    idle: Duration,
}

impl BodyBudget {
    /// Validate a positive total of at most one year (365 days) that Tokio can
    /// represent as a deadline, and a positive idle bound shorter than it.
    /// A longer total is rejected as [`ConfigurationError::TooLarge`].
    pub fn new(total: Duration, idle: Duration) -> Result<Self, ConfigurationError> {
        const TOTAL: &str = "HTTP body total budget";
        const IDLE: &str = "HTTP body idle budget";
        if total.is_zero() {
            return Err(ConfigurationError::Zero(TOTAL));
        }
        if idle.is_zero() {
            return Err(ConfigurationError::Zero(IDLE));
        }
        if total > Duration::from_secs(365 * 24 * 60 * 60)
            || Instant::now().checked_add(total).is_none()
        {
            return Err(ConfigurationError::TooLarge(TOTAL));
        }
        if idle >= total {
            return Err(ConfigurationError::TooLarge(IDLE));
        }
        Ok(Self { total, idle })
    }

    /// The validated bound on the whole body.
    pub const fn total(self) -> Duration {
        self.total
    }

    /// The validated bound on each wait for the next frame.
    pub const fn idle(self) -> Duration {
        self.idle
    }
}
