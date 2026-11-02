//! Parent-side driver: launch one child scenario with standard output and
//! standard error captured separately, wait for it to exit, and check both.

use crate::{
    capture::Capture,
    child::{SECRET, SECRET_CORE},
    evidence::Output,
    launch,
};
use std::{
    io::{self, Write},
    process::{Child, Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

/// Failure guard for the whole child run.
const GUARD: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(5);
const SITE_PREFIX: &str = "batter-fixture:site:";

/// How a scenario's child is launched.
#[derive(Clone, Copy, Default)]
pub(crate) struct Launch {
    /// `RUST_BACKTRACE` for the child; unset means `0`.
    pub(crate) backtrace: Option<&'static str>,
    /// Lower the open-descriptor limit through `/bin/sh` before `exec`.
    pub(crate) descriptor_limit: Option<u32>,
}

/// How the child must end.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Exit {
    /// The scenario's fixture test passed.
    Success,
    /// A panic escaped the libtest test function.
    UncaughtPanic,
}

pub(crate) struct Finished {
    scenario: &'static str,
    status: ExitStatus,
    stdout: Output,
    stderr: Output,
}

/// Run one scenario to completion; kill it at the guard.
pub(crate) fn run(scenario: &'static str, launch: Launch) -> Finished {
    let (input, mut authority) = io::pipe().expect("launch pipe");
    let (stdout, stdout_writer) = io::pipe().expect("stdout pipe");
    let (stderr, stderr_writer) = io::pipe().expect("stderr pipe");
    let test_binary = std::env::current_exe().expect("test binary path");
    let mut command = match launch.descriptor_limit {
        None => Command::new(test_binary),
        Some(limit) => {
            // `exec` keeps the shell's process ID, which the launch line names.
            let mut shell = Command::new("/bin/sh");
            shell
                .arg("-c")
                .arg(format!("ulimit -n {limit} && exec \"$0\" \"$@\""))
                .arg(test_binary);
            shell
        }
    };
    command
        .args(launch::CHILD_ARGS)
        .env("RUST_BACKTRACE", launch.backtrace.unwrap_or("0"))
        .stdin(input)
        .stdout(stdout_writer)
        .stderr(stderr_writer);
    let stdout = Capture::start(stdout).expect("stdout capture");
    let stderr = Capture::start(stderr).expect("stderr capture");
    let spawned = command.spawn();
    // Command retains its pipe ends after spawn; drop them so each capture
    // thread observes EOF once the child is reaped.
    drop(command);
    let mut child = Running {
        child: spawned.expect("spawn fixture child"),
        stdout,
        stderr,
        scenario,
    };
    let line = format!("batter-fixture-v1 {} {scenario}\n", child.child.id());
    if let Err(error) = authority.write_all(line.as_bytes()) {
        child.fail(&format!("launch authorization failed: {error}"));
    }
    // The child's only stdin writer stays open until it exits: its EOF would
    // tell the child that this parent is gone.
    let finished = child.finish();
    drop(authority);
    finished
}

struct Running {
    child: Child,
    stdout: Capture,
    stderr: Capture,
    scenario: &'static str,
}

impl Running {
    fn finish(mut self) -> Finished {
        let deadline = Instant::now() + GUARD;
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => self.fail("child did not exit"),
                Ok(None) => thread::sleep(POLL),
                Err(error) => self.fail(&format!("child status unavailable: {error}")),
            }
        };
        let stdout = match self.stdout.finish() {
            Ok(output) => output,
            Err(error) => self.fail(&format!("stdout capture failed: {error}")),
        };
        let stderr = match self.stderr.finish() {
            Ok(output) => output,
            Err(error) => self.fail(&format!("stderr capture failed: {error}")),
        };
        Finished {
            scenario: self.scenario,
            status,
            stdout,
            stderr,
        }
    }

    fn fail(&mut self, reason: &str) -> ! {
        let status = self.reap();
        panic!(
            "{}: {reason}; status={status:?}; stdout={:?}; stderr={:?}",
            self.scenario,
            self.stdout.finish(),
            self.stderr.finish()
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

impl Drop for Running {
    fn drop(&mut self) {
        // Reap before each Capture joins its reader, which needs the child's EOF.
        let _ = self.reap();
    }
}

impl Finished {
    pub(crate) fn stderr(&self) -> &Output {
        &self.stderr
    }

    pub(crate) fn status(&self) -> ExitStatus {
        self.status
    }

    pub(crate) fn fail(&self, reason: &str) -> ! {
        panic!(
            "{}: {reason}; status={:?}; stdout={:?}; stderr={:?}",
            self.scenario, self.status, self.stdout, self.stderr
        );
    }

    /// Check the exit, then the stdout protocol records. Standard output never
    /// receives hook output, so its shared panic detector stays meaningful.
    pub(crate) fn expect(&self, exit: Exit, required: &[&str]) -> &Self {
        let exited_as_expected = match exit {
            Exit::Success => self.status.success(),
            // libtest's failure status, which is also std's status for a
            // panic escaping `main`.
            Exit::UncaughtPanic => self.status.code() == Some(101),
        };
        if !exited_as_expected {
            self.fail(&format!("expected {exit:?}"));
        }
        if let Err(reason) = self.stdout.validate(required, &[]) {
            self.fail(&reason);
        }
        self
    }

    /// The panic sites the child recorded on stdout, in order, with repeats.
    pub(crate) fn sites(&self) -> Vec<String> {
        self.stdout
            .text()
            .lines()
            .filter_map(|line| line.strip_prefix(SITE_PREFIX))
            .map(str::to_owned)
            .collect()
    }

    /// Standard error is exactly one redacted line per recorded panic, in
    /// order, so nothing else (a payload, thread name or backtrace) appears.
    pub(crate) fn expect_redacted(&self, panics: usize) -> &Self {
        let sites = self.sites();
        if sites.len() != panics {
            self.fail(&format!("expected {panics} recorded panic sites"));
        }
        let expected: String = sites
            .iter()
            .map(|site| format!("\na thread panicked at {site}; panic payload redacted\n"))
            .collect();
        if self.stderr.text() != expected {
            self.fail(&format!("standard error differs from {expected:?}"));
        }
        self.expect_no_secret()
    }

    /// Standard error is empty although the child recorded `panics` panics.
    pub(crate) fn expect_silent(&self, panics: usize) -> &Self {
        if self.sites().len() != panics {
            self.fail(&format!("expected {panics} recorded panic sites"));
        }
        if !self.stderr.text().is_empty() {
            self.fail("standard error is not empty");
        }
        self.expect_no_secret()
    }

    /// Neither stream contains the secret or its distinctive core.
    pub(crate) fn expect_no_secret(&self) -> &Self {
        for (stream, output) in [("stdout", &self.stdout), ("stderr", &self.stderr)] {
            let text = output.text();
            if text.contains(SECRET) || text.contains(SECRET_CORE) {
                self.fail(&format!("{stream} disclosed the panic payload"));
            }
        }
        self
    }
}
