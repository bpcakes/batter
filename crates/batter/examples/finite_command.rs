//! A finite native operation with independently owned work and cleanup.
//! Run with --fail-work, --fail-cleanup, --fail-both, --cancel, or --deadline.
//! The command owner cancels work on drop; its live-runtime coordinator still
//! finalizes registered resources. Borrowed waiter cancellation changes no ownership.

#[cfg(test)]
#[path = "finite_command/tests.rs"]
mod tests;

use batter::{
    RegistrationError,
    cleanup::CleanupBudget,
    command::{Command, RunningCommand},
    operation::OperationContext,
};
use std::{future::pending, io, process::ExitCode, sync::Arc, time::Duration};
use tokio::net::UdpSocket;

#[derive(Clone, Copy)]
enum Work {
    Echo,
    Fail,
    Cancel,
    Deadline,
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

fn start(
    context: OperationContext,
    work: Work,
    fail_cleanup: bool,
) -> RunningCommand<Vec<u8>, CommandError> {
    let second = Duration::from_secs(1);
    let cleanup =
        CleanupBudget::new(second, second, second).expect("constant cleanup budget is valid");
    Command::new(context, cleanup, move |scope| {
        Box::pin(async move {
            scope.stage("command.echo")?;
            let slot = scope.reserve_cleanup("socket")?;
            let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
            let closing = socket.clone();
            // Register immediately after acquisition, before another await.
            slot.register(move || async move {
                // Native pool/exporter close or flush would be awaited here.
                drop(closing);
                if fail_cleanup {
                    return Err(CommandError::Cleanup.into());
                }
                Ok(())
            });
            match work {
                Work::Fail => return Err(CommandError::Work),
                Work::Cancel => {
                    scope.context().cancel();
                    pending::<()>().await;
                }
                Work::Deadline => pending::<()>().await,
                Work::Echo => {}
            }
            socket.send_to(b"prepare", socket.local_addr()?).await?;
            let mut buffer = [0; 32];
            let (length, _) = socket.recv_from(&mut buffer).await?;
            Ok(buffer[..length].to_vec())
        })
    })
    .start()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let (work, fail_cleanup) = match arguments.as_slice() {
        [] => (Work::Echo, false),
        [arg] if arg == "--fail-work" => (Work::Fail, false),
        [arg] if arg == "--fail-cleanup" => (Work::Echo, true),
        [arg] if arg == "--fail-both" => (Work::Fail, true),
        [arg] if arg == "--cancel" => (Work::Cancel, false),
        [arg] if arg == "--deadline" => (Work::Deadline, false),
        _ => {
            eprintln!(
                "usage: finite_command [--fail-work|--fail-cleanup|--fail-both|--cancel|--deadline]"
            );
            return ExitCode::FAILURE;
        }
    };
    let context =
        OperationContext::new(Duration::from_secs(1)).expect("constant command budget is valid");
    let command = start(context, work, fail_cleanup);
    let outcome = command.wait().await;
    // Default command diagnostics retain causes without formatting their contents.
    match outcome {
        Ok(report) => {
            println!(
                "work succeeded: {}; cleanup succeeded: {}; command succeeded: {}",
                report.work.is_ok(),
                report
                    .cleanup
                    .as_ref()
                    .is_ok_and(batter::cleanup::CleanupReport::is_success),
                report.is_success()
            );
            if report.is_success() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(_) => {
            eprintln!("command coordinator failed");
            ExitCode::FAILURE
        }
    }
}
