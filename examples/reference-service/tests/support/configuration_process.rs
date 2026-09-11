// Reuse repository-owned Unix kill/reap/capture machinery; these controls own
// only the configuration child environment and its explicit launch protocol.
#[allow(dead_code)]
#[path = "../../../../test-support/process/capture.rs"]
mod capture;
#[allow(dead_code)]
#[path = "../../../../test-support/process/evidence.rs"]
mod evidence;
#[allow(dead_code)]
#[path = "../../../../test-support/process/launch.rs"]
pub(crate) mod launch;
#[allow(dead_code)]
#[path = "../../../../test-support/process/timing.rs"]
mod timing;
#[allow(dead_code)]
mod watchdog {
    include!("../../../../test-support/process/watchdog.rs");

    impl ChildRun {
        pub fn validate_text(
            &self,
            expected: ExpectedExit,
            required: &[&str],
            forbidden: &[&str],
        ) -> Result<(), String> {
            self.validate(expected, &[], &[])?;
            let text = self.output.text();
            if required.iter().any(|part| !text.contains(part))
                || forbidden.iter().any(|part| text.contains(part))
            {
                return Err(format!("configuration diagnostic mismatch: {self:?}"));
            }
            Ok(())
        }
    }

    pub fn run(scenario: &str, env: &[(&str, std::ffi::OsString)]) -> ChildRun {
        let (input, parent_pipe) = io::pipe().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(launch::CHILD_ARGS).stdin(input).env_clear();
        command.envs(env.iter().map(|(key, value)| (key, value)));
        let mut child = FixtureChild::spawn_command(scenario, command, Some(parent_pipe)).unwrap();
        writeln!(
            child.parent_pipe.as_mut().unwrap(),
            "batter-fixture-v1 {} {scenario}",
            child.child.id()
        )
        .unwrap();
        child.wait(Duration::from_secs(5)).unwrap()
    }
}
pub(crate) use watchdog::{ExpectedExit, run};
