use super::*;
use batter::{
    cleanup::CleanupOutcome,
    command::{CommandSuccess, SharedCommandReport},
    operation::Interruption,
};
use std::{
    io::{BufRead, BufReader, Read},
    process::{Child, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
    time::Instant,
};

type Finished = Result<CommandSuccess<EchoReply>, CommandFailure<EchoReply, CommandError>>;

const SIGNAL_TEST: &str = "tests::executable_root_finalizes_after_sigint_or_sigterm";
const CHILD_ARGS: [&str; 4] = ["--exact", SIGNAL_TEST, "--nocapture", "--test-threads=1"];
const CHILD_MODE: &str = "BATTER_FINITE_COMMAND_SIGNAL_CHILD";
const CHILD_FINISHED: &str = "signal-child-finished";
const WAITING: &str = "finite_command: waiting for SIGINT or SIGTERM";
const GUARD: Duration = Duration::from_secs(5);

fn context() -> OperationContext {
    OperationOwner::new(Duration::from_secs(5))
        .unwrap()
        .into_context()
}

/// In-process tests never install signal listeners: they start the same inert
/// command that the executable root starts with signals.
async fn finish(context: OperationContext, work: Work, cleanup: CleanupResult) -> Finished {
    command(context, work, cleanup)
        .unwrap()
        .start_without_signals()
        .finish()
        .await
}

fn failed(finished: Finished) -> SharedCommandReport<EchoReply, CommandError> {
    match finished {
        Err(CommandFailure::Report(report)) => report,
        Err(CommandFailure::Coordinator(_)) => panic!("command coordinator failed"),
        Ok(success) => panic!("command unexpectedly succeeded: {success}"),
    }
}

fn assert_closed(cleanup: &CleanupReport) {
    assert_eq!(cleanup.records.len(), 1);
    assert_eq!(cleanup.records[0].name, "socket");
    assert_eq!(cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    assert!(cleanup.skipped.is_empty());
}

#[tokio::test]
async fn successful_command_moves_the_native_reply_out_with_cleanup_evidence() {
    let success = finish(context(), Work::Echo, CleanupResult::Success)
        .await
        .unwrap();
    assert_eq!(success.stage(), "command.echo");
    assert_closed(success.cleanup());
    let reply: EchoReply = success.into_value();
    assert_eq!(reply.bytes, b"prepare");
}

#[tokio::test]
async fn work_and_cleanup_failures_both_remain_inspectable() {
    let report = failed(finish(context(), Work::Fail, CleanupResult::Failure).await);
    assert!(!report.is_success());
    assert!(matches!(
        report.work,
        Err(CommandCause::Failed(CommandError::Work))
    ));
    let cleanup = report.cleanup.as_ref().unwrap();
    assert_eq!(cleanup.records.len(), 1);
    let record = &cleanup.records[0];
    assert_eq!(record.outcome, CleanupOutcome::Failed);
    assert!(matches!(
        record.error.as_ref().unwrap().downcast_ref(),
        Some(CommandError::Cleanup)
    ));
    assert!(cleanup.skipped.is_empty());
}

#[tokio::test]
async fn cleanup_failure_keeps_the_successful_work_value_by_reference() {
    let report = failed(finish(context(), Work::Echo, CleanupResult::Failure).await);
    assert!(!report.is_success());
    let Ok(reply) = &report.work else {
        panic!("work value was not retained")
    };
    assert_eq!(reply.bytes, b"prepare");
    assert_eq!(
        report.cleanup.as_ref().unwrap().records[0].outcome,
        CleanupOutcome::Failed
    );
}

#[tokio::test]
async fn work_failure_still_awaits_successful_cleanup() {
    let report = failed(finish(context(), Work::Fail, CleanupResult::Success).await);
    assert!(matches!(
        report.work,
        Err(CommandCause::Failed(CommandError::Work))
    ));
    assert_closed(report.cleanup.as_ref().unwrap());
}

#[tokio::test]
async fn cancelled_command_still_awaits_successful_cleanup() {
    let context = context();
    let report = failed(finish(context.clone(), Work::Cancel, CleanupResult::Success).await);
    assert!(
        context.check().is_ok(),
        "command cancellation cannot reach its parent"
    );
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
    assert_closed(report.cleanup.as_ref().unwrap());
}

// Paused time: the idle work lets the clock advance to its own deadline.
#[tokio::test(start_paused = true)]
async fn expired_command_still_has_a_separate_cleanup_budget() {
    let context = context();
    let report = failed(finish(context.clone(), Work::Deadline, CleanupResult::Success).await);
    assert_eq!(context.check(), Err(Interruption::DeadlineExceeded));
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::DeadlineExceeded))
    ));
    assert_closed(report.cleanup.as_ref().unwrap());
}

/// Re-executes this test binary as the executable root. Only the exact child
/// arguments together with the mode variable select the child branch.
#[test]
fn executable_root_finalizes_after_sigint_or_sigterm() {
    if std::env::var_os(CHILD_MODE).is_some_and(|mode| mode == "root")
        && std::env::args().skip(1).eq(CHILD_ARGS)
    {
        run_root_child();
        return;
    }
    for signal in ["INT", "TERM"] {
        let output = signal_root_child(signal);
        for fact in [
            "work succeeded: false",
            "work interrupted: true",
            "cleanup succeeded: true",
            "command succeeded: false",
            CHILD_FINISHED,
        ] {
            assert!(output.contains(fact), "SIG{signal}: {fact}: {output:?}");
        }
    }
}

/// The real root path on the runtime `#[tokio::main]` builds for it.
fn run_root_child() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let code = runtime
        .block_on(root(&[OsString::from("--until-signal")]))
        .unwrap();
    // Interrupted work is not a success, even though cleanup completed.
    assert_eq!(code, ExitCode::FAILURE);
    println!("{CHILD_FINISHED}");
}

fn signal_root_child(signal: &str) -> String {
    let mut child = SignalChild::spawn();
    child.wait_for_line(WAITING);
    child.send(signal);
    let output = child.finish(Instant::now() + GUARD);
    assert!(
        !output.timed_out && output.status.success(),
        "SIG{signal}: {}; timed out: {}; stdout={:?}; stderr={:?}",
        output.status,
        output.timed_out,
        output.stdout,
        output.stderr
    );
    output.stdout
}

struct SignalChild {
    child: Child,
    lines: Receiver<String>,
    stdout: Option<JoinHandle<io::Result<String>>>,
    stderr: Option<JoinHandle<io::Result<String>>>,
}

struct ChildOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

impl SignalChild {
    fn spawn() -> Self {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(CHILD_ARGS)
            .env(CHILD_MODE, "root")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (line_tx, lines) = mpsc::channel();
        let stdout = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut output = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line)? == 0 {
                    return Ok(output);
                }
                output.push_str(&line);
                // The parent may stop listening; keep draining regardless.
                let _ = line_tx.send(line);
            }
        });
        // Drain stderr so the child can never block writing diagnostics.
        let stderr = thread::spawn(move || {
            let mut output = String::new();
            stderr.read_to_string(&mut output)?;
            Ok(output)
        });
        Self {
            child,
            lines,
            stdout: Some(stdout),
            stderr: Some(stderr),
        }
    }

    fn wait_for_line(&mut self, expected: &str) {
        let deadline = Instant::now() + GUARD;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(remaining) {
                Ok(line) if line.contains(expected) => return,
                Ok(_) => {}
                Err(_) => {
                    let output = self.finish(Instant::now());
                    panic!(
                        "child did not print {expected:?}; status={}; stdout={:?}; stderr={:?}",
                        output.status, output.stdout, output.stderr
                    );
                }
            }
        }
    }

    fn send(&mut self, signal: &str) {
        let sent = std::process::Command::new("/bin/kill")
            .args([format!("-{signal}"), self.child.id().to_string()])
            .status();
        if !matches!(sent, Ok(status) if status.success()) {
            let output = self.finish(Instant::now());
            panic!(
                "failed to send SIG{signal}: {sent:?}; status={}; stdout={:?}; stderr={:?}",
                output.status, output.stdout, output.stderr
            );
        }
    }

    /// Wait until `deadline`, then kill and reap; join both drained pipes.
    fn finish(&mut self, deadline: Instant) -> ChildOutput {
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                timed_out = true;
                break self.kill_and_reap();
            }
            thread::sleep(Duration::from_millis(10));
        };
        ChildOutput {
            status,
            stdout: join(self.stdout.take()),
            stderr: join(self.stderr.take()),
            timed_out,
        }
    }

    fn kill_and_reap(&mut self) -> ExitStatus {
        // A kill can race with natural exit; the reap reports either.
        let _ = self.child.kill();
        self.child.wait().unwrap()
    }
}

impl Drop for SignalChild {
    fn drop(&mut self) {
        // A failed assertion must not leave the child running or unreaped.
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn join(capture: Option<JoinHandle<io::Result<String>>>) -> String {
    match capture.map(JoinHandle::join) {
        Some(Ok(Ok(output))) => output,
        Some(Ok(Err(error))) => format!("<capture failed: {error}>"),
        Some(Err(_)) => "<capture panicked>".to_owned(),
        None => "<already joined>".to_owned(),
    }
}
