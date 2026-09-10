//! A finite native operation followed by separately awaited cleanup.
//! Run with --fail-work, --fail-cleanup, --fail-both, --cancel, or --deadline
//! to inspect unsuccessful outcomes. No service supervisor is needed.
//!
//! The caller must keep driving this future through cleanup. Outer-future loss,
//! unwinding and runtime/process death can abandon finalization. A command
//! signal handler should cancel its OperationContext and continue awaiting this
//! composition; dropping it in select! would skip the subsequent cleanup.

#[cfg(test)]
#[path = "finite_command/tests.rs"]
mod tests;

use batter::{
    RegistrationError,
    cleanup::{CleanupBudget, CleanupReport, CleanupStack},
    operation::{OperationContext, OperationError},
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

// Application-owned result: preserve the concrete work error AND every cleanup
// record. A first `?` must not discard either outcome or bypass cleanup.
struct CommandReport {
    work: Result<Vec<u8>, OperationError<CommandError>>,
    cleanup: CleanupReport,
}

impl CommandReport {
    fn is_success(&self) -> bool {
        self.work.is_ok() && self.cleanup.is_success()
    }
}

async fn run(context: &OperationContext, work: Work, fail_cleanup: bool) -> CommandReport {
    let mut cleanup = CleanupStack::new();
    let work = context
        .run("command.echo", |_| async {
            let slot = cleanup.reserve("socket")?;
            let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
            let closing = socket.clone();
            // Register immediately after native acquisition, before another await.
            slot.register(move || async move {
                // Socket close is synchronous; a pool/exporter would await its
                // native close/flush here. No operation cancellation token is used.
                drop(closing);
                if fail_cleanup {
                    return Err(CommandError::Cleanup.into());
                }
                Ok(())
            });
            match work {
                Work::Fail => return Err(CommandError::Work),
                Work::Cancel => {
                    // Demonstrate cancellation AFTER acquiring/registering the resource.
                    context.cancel();
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
        .await;
    // This budget starts AFTER work stops. Even a cancelled/expired operation
    // must not cancel its own finalization. The outer future is still caller-owned.
    let budget = CleanupBudget::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .expect("constant cleanup budget is valid");
    let cleanup = cleanup.close(budget).await;
    CommandReport { work, cleanup }
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
    let report = run(&context, work, fail_cleanup).await;
    // Rich causes remain in report for a trusted application sink. Result-returning
    // main would automatically print Debug errors; this boundary prints only facts.
    println!(
        "work succeeded: {}; {}",
        report.work.is_ok(),
        report.cleanup
    );
    if report.is_success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
