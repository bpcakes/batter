// Suite-local composition: shared mechanics, foundation-owned controls.
include!("../../../../test-support/process/watchdog.rs");

use super::timing::{EXIT_LIMIT, OBSERVATION_LIMIT};

impl ChildRun {
    pub fn run(scenario: &str) -> Self {
        Self::run_with_policy(scenario, WaitPolicy::for_scenario(scenario))
    }
}

impl FixtureChild {
    fn wait_for_scenario(&mut self) -> io::Result<ChildRun> {
        let limit = self.resolve_deadline(WaitPolicy::for_scenario(&self.scenario));
        self.wait(limit)
    }

    fn wait_for_event(&mut self, event: &'static str) {
        self.resolve_deadline(WaitPolicy::until_event(event));
    }
}

#[path = "watchdog_tests.rs"]
mod tests;
