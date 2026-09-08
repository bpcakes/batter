use super::{evidence::Output, fixture, launch};
use std::time::{Duration, Instant};

pub const EXIT_LIMIT: Duration = Duration::from_secs(5);
pub const STARTUP_LIMIT: Duration = Duration::from_secs(5);
pub const OBSERVATION_LIMIT: Duration =
    fixture::BLOCKED_OBSERVATION.saturating_add(Duration::from_secs(1));

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
    pub fn for_scenario(scenario: &str) -> Self {
        if matches!(
            scenario,
            "blocked-current-thread" | "blocked-request-failure"
        ) {
            let policy = Self::AfterEvent {
                event: "drain-requested",
                startup: STARTUP_LIMIT,
                observation: OBSERVATION_LIMIT,
            };
            assert!(policy.maximum_wait() < launch::EMERGENCY_LIMIT);
            policy
        } else {
            Self::ExitAfter(EXIT_LIMIT)
        }
    }

    pub fn until_event(event: &'static str) -> Self {
        Self::AfterEvent {
            event,
            startup: STARTUP_LIMIT,
            observation: Duration::ZERO,
        }
    }

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

#[path = "timing_tests.rs"]
mod tests;
