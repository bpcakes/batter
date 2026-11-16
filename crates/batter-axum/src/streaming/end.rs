//! How an owned body ends: its outcome, the error that tells the transport,
//! and the wakeups a pending body waits on.

use batter_core::operation::Interruption;
use std::{fmt, future::Future, pin::Pin};
use tokio::time::{Instant, Sleep};

/// How an owned body ended, as its completion event names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BodyEnd {
    Completed,
    Failed,
    DeadlineExceeded,
    IdleTimeout,
    Cancelled,
    Panicked,
    Dropped,
}

impl BodyEnd {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::IdleTimeout => "idle_timeout",
            Self::Cancelled => "cancelled",
            Self::Panicked => "panicked",
            Self::Dropped => "dropped",
        }
    }

    pub(super) const fn interrupted(reason: Interruption) -> Self {
        match reason {
            Interruption::DeadlineExceeded => Self::DeadlineExceeded,
            Interruption::Cancelled => Self::Cancelled,
        }
    }
}

/// The error a library-ended body yields, so the transport truncates the
/// message. It names the bound and nothing from the application.
#[derive(Debug)]
pub(super) struct BodyInterrupted(pub(super) BodyEnd);

impl fmt::Display for BodyInterrupted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "response body ended: {}", self.0.label())
    }
}

impl std::error::Error for BodyInterrupted {}

/// The wakeups a pending body needs, allocated on its first pending poll.
pub(super) struct Wake {
    pub(super) timer: Pin<Box<Sleep>>,
    pub(super) cancelled: Pin<Box<dyn Future<Output = ()> + Send>>,
    /// Whether the timer currently describes this wait.
    pub(super) armed: bool,
    /// Whether the armed instant is the idle bound rather than the total.
    pub(super) idle_first: bool,
}

impl Wake {
    /// The armed bound's outcome once its instant has passed, so a source
    /// that becomes ready only after its wait expired does not extend it.
    pub(super) fn due(&self) -> Option<BodyEnd> {
        (self.armed && self.timer.deadline() <= Instant::now()).then_some(if self.idle_first {
            BodyEnd::IdleTimeout
        } else {
            BodyEnd::DeadlineExceeded
        })
    }
}
