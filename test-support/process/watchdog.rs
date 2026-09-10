use super::{capture::Capture, evidence::Output, launch, timing::WaitPolicy};
use std::{
    io::{self, Write},
    os::unix::process::ExitStatusExt,
    process::{Child, Command, ExitStatus},
    time::{Duration, Instant},
};

const POLL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug)]
pub enum ExpectedExit {
    Success,
    Code(i32),
    WatchdogKill,
}

#[derive(Debug)]
pub struct ChildRun {
    pub status: ExitStatus,
    kill_requested_at: Option<Duration>,
    scenario: String,
    elapsed: Duration,
    deadline_after_start: Duration,
    output: Output,
}

impl ChildRun {
    pub fn kill_requested(&self) -> bool {
        self.kill_requested_at.is_some()
    }

    pub fn run_with_policy(scenario: &str, policy: WaitPolicy) -> Self {
        let mut child = FixtureChild::spawn(scenario).unwrap();
        let limit = child.resolve_deadline(policy);
        child.wait(limit).unwrap()
    }

    pub fn validate(
        &self,
        expected: ExpectedExit,
        required: &[&str],
        forbidden: &[&str],
    ) -> Result<(), String> {
        // Truncated diagnostics cannot prove absence of a panic or forbidden
        // event. Overflow fails closed even if earlier milestones all exist.
        // A panic can unwind into a Runtime::drop that blocks forever. Keep
        // Rust's default hook; these are fixed fixture diagnostics only.
        self.output.check()?;
        let expected_status = match expected {
            ExpectedExit::Success => !self.kill_requested() && self.status.success(),
            ExpectedExit::Code(code) => !self.kill_requested() && self.status.code() == Some(code),
            ExpectedExit::WatchdogKill => {
                self.kill_requested_at
                    .is_some_and(|at| at >= self.deadline_after_start)
                    && killed_status(self.status)
            }
        };
        if !expected_status {
            return Err(format!(
                "unexpected {} child termination: {self:?}",
                self.scenario
            ));
        }
        self.output.validate(required, forbidden)
    }
}

fn killed_status(status: ExitStatus) -> bool {
    status.signal() == Some(9) // SIGKILL, not merely any unsuccessful exit.
}

struct FixtureChild {
    child: Child,
    capture: Capture,
    parent_pipe: Option<io::PipeWriter>,
    started: Instant,
    scenario: String,
    kill_requested_at: Option<Duration>,
}

impl FixtureChild {
    fn spawn(scenario: &str) -> io::Result<Self> {
        let (input, parent_pipe) = io::pipe()?;
        let mut command = Command::new(std::env::current_exe()?);
        command.args(launch::CHILD_ARGS).stdin(input);
        let mut child = Self::spawn_command(scenario, command, Some(parent_pipe))?;
        writeln!(
            child.parent_pipe.as_mut().unwrap(),
            "batter-fixture-v1 {} {scenario}",
            child.child.id()
        )?;
        Ok(child)
    }

    fn spawn_command(
        scenario: &str,
        mut command: Command,
        parent_pipe: Option<io::PipeWriter>,
    ) -> io::Result<Self> {
        let (reader, writer) = io::pipe()?;
        command
            .env("RUST_BACKTRACE", "0")
            .stdout(writer.try_clone()?)
            .stderr(writer);
        let capture = Capture::start(reader)?;
        let started = Instant::now();
        let spawned = command.spawn();
        // Command retains its pipe writers after spawn, including on errors.
        // Drop it before joining the capture thread, or EOF cannot arrive.
        drop(command);
        let child = spawned?;
        Ok(Self {
            child,
            capture,
            parent_pipe,
            started,
            scenario: scenario.to_owned(),
            kill_requested_at: None,
        })
    }

    fn kill_and_reap(&mut self) -> io::Result<ExitStatus> {
        // Preserve the first request's time even if cleanup retries a kill.
        self.kill_requested_at.get_or_insert(self.started.elapsed());
        if let Err(error) = self.child.kill() {
            // Some platforms reject a kill racing with natural exit. Preserve
            // the observed exit if available; otherwise retain the I/O error.
            return self.child.try_wait()?.ok_or(error);
        }
        self.child.wait()
    }

    fn wait(&mut self, limit: Duration) -> io::Result<ChildRun> {
        let status = loop {
            if let Some(status) = self.child.try_wait()? {
                break status;
            }
            if self.started.elapsed() >= limit {
                break self.kill_and_reap()?;
            }
            std::thread::sleep(POLL);
        };
        let output = self.capture.finish()?;
        Ok(ChildRun {
            status,
            kill_requested_at: self.kill_requested_at,
            scenario: self.scenario.clone(),
            elapsed: self.started.elapsed(),
            deadline_after_start: limit,
            output,
        })
    }

    fn resolve_deadline(&mut self, policy: WaitPolicy) -> Duration {
        loop {
            let exited = self.child.try_wait().unwrap().is_some();
            if exited {
                // Exit and capture publication are independent. Join the reader
                // before deciding whether the exited child supplied its event.
                self.capture.finish().unwrap();
                if let WaitPolicy::AfterEvent { event, .. } = policy {
                    // Missing final evidence is terminal regardless of when the
                    // parent resumed. Do not turn an observed exit into waiting
                    // for (or merely reporting) a startup deadline.
                    self.capture.snapshot().require_final_event(event).unwrap();
                }
            }
            if let Some(deadline) = self
                .capture
                .inspect(|output, now| policy.deadline(output, self.started, now))
                .unwrap()
            {
                return deadline;
            }
            std::thread::sleep(POLL);
        }
    }
}

impl Drop for FixtureChild {
    fn drop(&mut self) {
        // Reap before Capture joins its reader. The parent pipe stays open
        // through wait, so only actual parent disappearance triggers EOF.
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.kill_and_reap();
        }
    }
}
