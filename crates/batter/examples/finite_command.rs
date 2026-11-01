//! A finite native operation whose executable root owns SIGTERM and SIGINT.
//! Run with --fail-work, --fail-cleanup, --fail-both, --cancel, --deadline or
//! --until-signal. Every mode still awaits the registered socket cleanup; only
//! the complete command contract moves the reply out and exits successfully.
//! `--until-signal` waits after registration until Ctrl-C or `kill -TERM`.

#[cfg(test)]
#[path = "finite_command/tests.rs"]
mod tests;

use batter::{
    BoxError, ConfigurationError, RegistrationError,
    cleanup::{CleanupBudget, CleanupReport},
    command::{Command, CommandCause, CommandFailure, CommandFuture, CommandScope},
    operation::{OperationContext, OperationOwner},
};
use std::{
    ffi::OsString,
    future::pending,
    io::{self, Write},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};
use tokio::net::UdpSocket;

/// The native reply. It derives no `Clone`: the command owner moves it out.
struct EchoReply {
    bytes: Vec<u8>,
}

#[derive(Clone, Copy)]
enum Work {
    Echo,
    Fail,
    Cancel,
    Deadline,
    UntilSignal,
}

impl Work {
    /// Waiting for an operator's signal needs a longer work budget.
    fn budget(self) -> Duration {
        match self {
            Self::UntilSignal => Duration::from_secs(30),
            Self::Echo | Self::Fail | Self::Cancel | Self::Deadline => Duration::from_secs(1),
        }
    }
}

#[derive(Clone, Copy)]
enum CleanupResult {
    Success,
    Failure,
}

#[derive(Debug, thiserror::Error)]
enum CommandError {
    #[error("resource registration failed")]
    Registration(#[from] RegistrationError),
    #[error("native socket operation failed")]
    Io(#[from] io::Error),
    #[error("command work failed")]
    Work,
    #[error("resource cleanup failed")]
    Cleanup,
}

/// Build the inert command. Nothing binds, runs or spawns until it is started:
/// the executable root starts it with signals, helpers and tests without.
fn command(
    context: OperationContext,
    work: Work,
    cleanup_result: CleanupResult,
) -> Result<
    Command<
        impl for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, EchoReply, CommandError>
        + Send
        + 'static,
    >,
    ConfigurationError,
> {
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second, second, second)?;
    Ok(Command::new(context, cleanup, move |scope| {
        Box::pin(async move {
            scope.stage("command.echo")?;
            let slot = scope.reserve_cleanup("socket")?;
            let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
            let closing = socket.clone();
            // Register immediately after acquisition, before another await.
            slot.register(move || async move {
                // Native pool/exporter close or flush would be awaited here.
                drop(closing);
                match cleanup_result {
                    CleanupResult::Success => Ok(()),
                    CleanupResult::Failure => Err(CommandError::Cleanup.into()),
                }
            });
            match work {
                Work::Fail => return Err(CommandError::Work),
                Work::Cancel => {
                    scope.cancel_work();
                    pending::<()>().await;
                }
                Work::Deadline => pending::<()>().await,
                Work::UntilSignal => {
                    writeln!(
                        io::stdout(),
                        "finite_command: waiting for SIGINT or SIGTERM"
                    )?;
                    io::stdout().flush()?;
                    pending::<()>().await;
                }
                Work::Echo => {}
            }
            socket.send_to(b"prepare", socket.local_addr()?).await?;
            let mut buffer = [0; 32];
            let (length, _) = socket.recv_from(&mut buffer).await?;
            Ok(EchoReply {
                bytes: buffer[..length].to_vec(),
            })
        })
    }))
}

fn parse(arguments: &[OsString]) -> Option<(Work, CleanupResult)> {
    match arguments {
        [] => Some((Work::Echo, CleanupResult::Success)),
        [arg] if arg == "--fail-work" => Some((Work::Fail, CleanupResult::Success)),
        [arg] if arg == "--fail-cleanup" => Some((Work::Echo, CleanupResult::Failure)),
        [arg] if arg == "--fail-both" => Some((Work::Fail, CleanupResult::Failure)),
        [arg] if arg == "--cancel" => Some((Work::Cancel, CleanupResult::Success)),
        [arg] if arg == "--deadline" => Some((Work::Deadline, CleanupResult::Success)),
        [arg] if arg == "--until-signal" => Some((Work::UntilSignal, CleanupResult::Success)),
        _ => None,
    }
}

/// The executable root, the only place that selects Unix signals: SIGINT or
/// SIGTERM cancels work, and the registered cleanup still runs to completion.
async fn root(arguments: &[OsString]) -> Result<ExitCode, BoxError> {
    let Some((work, cleanup_result)) = parse(arguments) else {
        eprintln!(
            "usage: finite_command \
             [--fail-work|--fail-cleanup|--fail-both|--cancel|--deadline|--until-signal]"
        );
        return Ok(ExitCode::FAILURE);
    };
    let context = OperationOwner::new(work.budget())?.into_context();
    let owner = command(context, work, cleanup_result)?.start_with_unix_signals()?;
    // Default command diagnostics retain causes without formatting their contents.
    Ok(match owner.finish().await {
        Ok(success) => {
            let cleanup_succeeded = success.cleanup().is_success();
            // Only the complete contract moves the reply out: no expect, no Clone.
            let reply = success.into_value();
            println!(
                "work succeeded: true; work interrupted: false; \
                 cleanup succeeded: {cleanup_succeeded}; command succeeded: true"
            );
            println!("echoed {} bytes", reply.bytes.len());
            ExitCode::SUCCESS
        }
        Err(CommandFailure::Report(report)) => {
            let interrupted = matches!(report.work, Err(CommandCause::Interrupted(_)))
                || report.interruption_after_work.is_some();
            println!(
                "work succeeded: {}; work interrupted: {interrupted}; \
                 cleanup succeeded: {}; command succeeded: {}",
                report.work.is_ok(),
                report.cleanup.as_ref().is_ok_and(CleanupReport::is_success),
                report.is_success()
            );
            ExitCode::FAILURE
        }
        Err(CommandFailure::Coordinator(_)) => {
            eprintln!("command coordinator failed");
            ExitCode::FAILURE
        }
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<ExitCode, BoxError> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    root(&arguments).await
}
