use super::evidence::Output;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub enum WaitPolicy {
    ExitAfter(Duration),
    AfterEvent {
        event: &'static str,
        startup: Duration,
        observation: Duration,
    },
}

impl WaitPolicy {
    pub fn maximum_wait(self) -> Duration {
        match self {
            Self::ExitAfter(limit) => limit,
            Self::AfterEvent {
                startup,
                observation,
                ..
            } => startup + observation,
        }
    }

    pub fn deadline(
        self,
        output: &Output,
        started: Instant,
        now: Instant,
    ) -> Result<Option<Duration>, String> {
        match self {
            Self::ExitAfter(limit) => Ok(Some(limit)),
            Self::AfterEvent {
                event,
                startup,
                observation,
            } => output
                .event_by(event, started + startup, now)
                .map(|at| at.map(|at| at.duration_since(started) + observation)),
        }
    }
}
