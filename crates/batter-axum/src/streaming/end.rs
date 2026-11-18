//! How an owned body ends: its outcome, the error that tells the transport,
//! and the wakeups a pending body waits on.

use batter_core::operation::Interruption;
use std::{
    fmt,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    pin::Pin,
};
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
/// Both hold the waker of the task that polled last.
pub(super) struct Wake {
    pub(super) timer: Pin<Box<Sleep>>,
    pub(super) cancelled: Pin<Box<dyn Future<Output = ()> + Send>>,
    /// Whether the timer currently describes this wait.
    pub(super) armed: bool,
    /// Whether the armed instant is the idle bound rather than the total.
    pub(super) idle_first: bool,
}

impl Wake {
    pub(super) fn new(timer: Sleep, cancelled: impl Future<Output = ()> + Send + 'static) -> Self {
        Self {
            timer: Box::pin(timer),
            cancelled: Box::pin(cancelled),
            armed: false,
            idle_first: false,
        }
    }

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

/// What is left of a body as it ends, torn down in stages. The stages that
/// destroy application values may unwind.
///
/// `source` is the application's body, `rejected` an answer it gave too
/// late, and `kept` what it yielded that the poll has yet to return. All
/// three are the application's, and each is destroyed on its own, under the
/// dispatcher `under` and inside the request span. If one of those
/// destructions unwinds, the others still happen, the same way and with
/// that unwind caught, and `kept` is destroyed instead of returned: a later
/// destructor that panics then replaces the earlier panic, where one
/// running inside that unwind would abort the process. `wait` is the body's
/// wait, which holds the polling task's waker and is released on every
/// path, under `under` as well, where it was created and polled.
pub(super) struct Ending<S, R, K> {
    source: Option<S>,
    rejected: Option<R>,
    kept: Option<K>,
    wait: Option<Wake>,
    under: Option<tracing::Dispatch>,
    /// Declared last: on an unwind the span is left, and its handle
    /// released, after the values were destroyed.
    entered: Option<tracing::span::EnteredSpan>,
}

/// What a destructor that unwound was carrying.
type Unwound = Box<dyn std::any::Any + Send>;

impl<S, R, K> Ending<S, R, K> {
    pub(super) fn armed(
        (source, rejected, kept): (S, R, K),
        wait: Option<Wake>,
        under: Option<tracing::Dispatch>,
    ) -> Self {
        Self {
            source: Some(source),
            rejected: Some(rejected),
            kept: Some(kept),
            wait,
            under,
            entered: None,
        }
    }

    /// Enter the request span and own it.
    pub(super) fn enter(&mut self, span: Option<tracing::Span>) {
        self.entered = span.map(tracing::Span::entered);
    }

    /// Destroy the source and any rejected answer, inside the span. If
    /// either unwinds, the poll returns nothing, so `kept` is destroyed as
    /// well before the unwind continues.
    pub(super) fn destroy_source(&mut self) {
        let source = self.source.take();
        let rejected = self.rejected.take();
        let first = self.destroy(source);
        let second = self.destroy(rejected);
        if let Some(unwound) = self.latest(first, second) {
            let kept = self.kept.take();
            let third = self.destroy(kept);
            if let Some(unwound) = self.latest(Some(unwound), third) {
                resume_unwind(unwound);
            }
        }
    }

    /// Destroy one application value, catching an unwind from it.
    fn destroy<T>(&self, value: T) -> Option<Unwound> {
        catch_unwind(AssertUnwindSafe(|| self.under(|| drop(value)))).err()
    }

    /// The later of two unwinds. A panic's payload is the application's
    /// too, so one that is superseded is destroyed like the values were.
    /// What its own destructor does if it panics is not followed further.
    fn latest(&self, earlier: Option<Unwound>, later: Option<Unwound>) -> Option<Unwound> {
        let Some(later) = later else { return earlier };
        drop(self.destroy(earlier));
        Some(later)
    }

    /// Release the wait, and with it the polling task's waker, under the
    /// dispatcher the wait was created and polled under.
    pub(super) fn release_wait(&mut self) {
        let wait = self.wait.take();
        self.under(|| drop(wait));
    }

    /// Leave the span and release its handle, which closes it if this was
    /// the last, under the dispatcher current here.
    pub(super) fn leave_span(&mut self) {
        if let Some(entered) = self.entered.take() {
            drop(entered.exit());
        }
    }

    /// Release the hand-over dispatcher and hand `kept` on.
    pub(super) fn release(mut self) -> Option<K> {
        let kept = self.kept.take();
        drop(self.under.take());
        kept
    }

    fn under(&self, destroy: impl FnOnce()) {
        match &self.under {
            Some(under) => tracing::dispatcher::with_default(under, destroy),
            None => destroy(),
        }
    }
}

impl<S, R, K> Drop for Ending<S, R, K> {
    fn drop(&mut self) {
        let left = (self.source.take(), self.rejected.take(), self.kept.take());
        let wait = self.wait.take();
        if left.0.is_some() || left.1.is_some() || left.2.is_some() || wait.is_some() {
            self.under(|| drop((left, wait)));
        }
    }
}
