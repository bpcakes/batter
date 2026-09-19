// Suite-local composition: shared mechanics, foundation-owned controls.
include!("../../../../test-support/process/timing.rs");

use super::{fixture, launch};

pub const EXIT_LIMIT: Duration = Duration::from_secs(5);
pub const STARTUP_LIMIT: Duration = Duration::from_secs(5);
pub const OBSERVATION_LIMIT: Duration =
    fixture::BLOCKED_OBSERVATION.saturating_add(Duration::from_secs(1));

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
}

#[path = "timing_tests.rs"]
mod tests;
