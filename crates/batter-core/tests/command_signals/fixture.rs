//! Parent-side driver: launch one child, wait for its printed markers, send
//! real signals with `/bin/kill`, and check how it exits. Every wait has a
//! failure guard; printed markers alone order the signals.

use crate::{capture::Capture, launch};
use std::{
    io::{self, Write},
    os::unix::process::ExitStatusExt,
    process::{Child, Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

/// Failure guard for each marker and for the final exit.
const GUARD: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(5);

#[derive(Clone, Copy, Debug)]
pub(crate) enum StopSignal {
    Term,
    Int,
}

impl StopSignal {
    fn flag(self) -> &'static str {
        match self {
            Self::Term => "-TERM",
            Self::Int => "-INT",
        }
    }

    /// The same numbers on Linux and macOS.
    fn number(self) -> i32 {
        match self {
            Self::Term => 15,
            Self::Int => 2,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Exit {
    /// The child survived every signal and its fixture test passed.
    Success,
    /// The signal's default disposition terminated the child.
    KilledBy(StopSignal),
}

pub(crate) struct SignalledChild {
    child: Child,
    capture: Capture,
    /// The child's only stdin writer. Launch authorization is its sole
    /// input; its EOF tells the child that this parent is gone.
    authority: io::PipeWriter,
    scenario: &'static str,
    sent: Vec<StopSignal>,
}

impl SignalledChild {
    pub(crate) fn spawn(scenario: &'static str) -> Self {
        let (input, authority) = io::pipe().expect("launch pipe");
        let (reader, writer) = io::pipe().expect("output pipe");
        let mut command = Command::new(std::env::current_exe().expect("test binary path"));
        command
            .args(launch::CHILD_ARGS)
            .env("RUST_BACKTRACE", "0")
            .stdin(input)
            .stdout(writer.try_clone().expect("output pipe"))
            .stderr(writer);
        let capture = Capture::start(reader).expect("output capture");
        let spawned = command.spawn();
        // Command retains its pipe ends after spawn; drop them so the capture
        // thread observes EOF once the child is reaped.
        drop(command);
        let mut fixture = Self {
            child: spawned.expect("spawn fixture child"),
            capture,
            authority,
            scenario,
            sent: Vec::new(),
        };
        let line = format!("batter-fixture-v1 {} {scenario}\n", fixture.child.id());
        if let Err(error) = fixture.authority.write_all(line.as_bytes()) {
            fixture.fail(&format!("launch authorization failed: {error}"));
        }
        fixture
    }

    /// Wait until the child has printed `event`, failing on exit, panic or guard.
    pub(crate) fn await_event(&mut self, event: &str) {
        let deadline = Instant::now() + GUARD;
        let mut exited = false;
        loop {
            match self
                .capture
                .inspect(|output, now| output.event_by(event, deadline, now))
            {
                Ok(Some(_)) => return,
                Ok(None) if exited => self.fail(&format!("child exited before {event}")),
                Ok(None) => {}
                Err(reason) => self.fail(&reason),
            }
            match self.child.try_wait() {
                Ok(None) => thread::sleep(POLL),
                // Exit and output publication are independent: join the
                // reader before deciding that the event is missing.
                Ok(Some(_)) => match self.capture.finish() {
                    Ok(_) => exited = true,
                    Err(error) => self.fail(&format!("output capture failed: {error}")),
                },
                Err(error) => self.fail(&format!("child status unavailable: {error}")),
            }
        }
    }

    /// Send one real signal to the still-running child.
    pub(crate) fn signal(&mut self, signal: StopSignal) {
        self.sent.push(signal);
        // An unreaped child keeps its pid, so the signal cannot reach a reused one.
        if !matches!(self.child.try_wait(), Ok(None)) {
            self.fail("child was not running when the signal was due");
        }
        let sent = Command::new("/bin/kill")
            .args([signal.flag(), &self.child.id().to_string()])
            .status();
        if !matches!(sent, Ok(status) if status.success()) {
            self.fail(&format!("kill {} failed: {sent:?}", signal.flag()));
        }
    }

    pub(crate) fn expect_exit(mut self, expected: Exit, required: &[&str], forbidden: &[&str]) {
        let deadline = Instant::now() + GUARD;
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => self.fail("child did not exit"),
                Ok(None) => thread::sleep(POLL),
                Err(error) => self.fail(&format!("child status unavailable: {error}")),
            }
        };
        let output = match self.capture.finish() {
            Ok(output) => output,
            Err(error) => self.fail(&format!("output capture failed: {error}")),
        };
        let exited_as_expected = match expected {
            Exit::Success => status.success(),
            Exit::KilledBy(signal) => status.signal() == Some(signal.number()),
        };
        if !exited_as_expected {
            self.fail(&format!("expected {expected:?}"));
        }
        if let Err(reason) = output.validate(required, forbidden) {
            self.fail(&reason);
        }
    }

    fn fail(&mut self, reason: &str) -> ! {
        let status = self.reap();
        let output = self.capture.finish();
        panic!(
            "{} after {:?}: {reason}; status={status:?}; output={output:?}",
            self.scenario, self.sent
        );
    }

    /// Kill unless the child already exited, then reap. Repeatable.
    fn reap(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.child.try_wait()? {
            return Ok(status);
        }
        if let Err(error) = self.child.kill() {
            // A kill can race natural exit; keep that exit when it is visible.
            return self.child.try_wait()?.ok_or(error);
        }
        self.child.wait()
    }
}

impl Drop for SignalledChild {
    fn drop(&mut self) {
        // Reap before Capture joins its reader, which needs the child's EOF.
        let _ = self.reap();
    }
}
