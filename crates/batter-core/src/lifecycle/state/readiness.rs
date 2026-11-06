use super::{Readiness, Shared};
use tokio::time::Instant;

/// First-readiness evidence, retained independently of later drain/stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadinessProgress {
    Pending(Option<Instant>),
    Ready,
    Stopped,
    DeadlineExceeded,
}

impl ReadinessProgress {
    pub(super) fn may_become_ready(self) -> bool {
        matches!(self, Self::Pending(deadline) if deadline.is_none_or(|end| Instant::now() < end))
    }

    pub(super) fn stop(&mut self) {
        if let Self::Pending(deadline) = *self {
            *self = if deadline.is_some_and(|end| Instant::now() >= end) {
                Self::DeadlineExceeded
            } else {
                Self::Stopped
            };
        }
    }
}

impl Shared {
    pub(crate) fn set_readiness_deadline(&self, deadline: Instant) {
        {
            let mut state = self.lock();
            if matches!(state.progress, ReadinessProgress::Pending(_)) {
                state.progress = ReadinessProgress::Pending(Some(deadline));
            }
        }
        self.expire_readiness();
    }

    pub(crate) fn expire_readiness(&self) {
        {
            let mut state = self.lock();
            if !matches!(state.progress, ReadinessProgress::Pending(Some(end)) if Instant::now() >= end)
            {
                return;
            }
            state.progress = ReadinessProgress::DeadlineExceeded;
            state.request_drain(Instant::now());
            self.publish_readiness(&state);
        }
        self.drain.cancel();
        self.changed.notify_waiters();
    }

    pub(crate) fn startup_progress(&self) -> ReadinessProgress {
        self.lock().progress
    }

    pub(crate) async fn wait_startup(&self) -> ReadinessProgress {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let progress = self.startup_progress();
            if !matches!(progress, ReadinessProgress::Pending(_)) {
                return progress;
            }
            // An abandoned supervisor publishes Draining before notification.
            if matches!(self.readiness(), Readiness::Draining | Readiness::Stopped) {
                return ReadinessProgress::Stopped;
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::state::Supervision;
    use std::time::Duration;

    #[tokio::test(start_paused = true)]
    async fn transition_refuses_late_readiness_before_timer_poll() {
        let shared = Shared::new(Supervision::Driven);
        shared.set_readiness_deadline(Instant::now() + Duration::from_secs(1));
        shared.start_driver();
        tokio::time::advance(Duration::from_secs(1)).await;
        shared.mark_ready();
        assert_eq!(shared.readiness(), Readiness::Starting);
        shared.expire_readiness();
        assert_eq!(shared.readiness(), Readiness::Draining);
        assert_eq!(
            shared.startup_progress(),
            ReadinessProgress::DeadlineExceeded
        );
    }
}
