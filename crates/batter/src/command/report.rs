use crate::{PanicPayload, cleanup::CleanupReport, operation::Interruption};
use std::{error::Error, fmt, ops::Deref, sync::Arc};
use tokio::task::JoinError;

/// Finite work failure without replacing concrete application causes.
pub enum CommandCause<E> {
    /// Original application error returned from the callback.
    Failed(E),
    /// Work deadline or cancellation won; external effects may be unknown.
    Interrupted(Interruption),
    /// Factory construction or work polling unwound.
    Panicked(PanicPayload),
}

impl<E> fmt::Debug for CommandCause<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Failed(_) => "command work failed",
            Self::Interrupted(_) => "command work interrupted",
            Self::Panicked(_) => "command work panicked",
        })
    }
}

/// Work and cleanup retained together. Successful work may coexist with failed
/// finalization; neither result replaces the other. Default formatting omits
/// result/error/panic contents. The runtime must survive to publish this report.
#[must_use = "inspect work, interruption and cleanup outcomes"]
pub struct CommandReport<T, E> {
    /// Last validated callback stage.
    pub stage: &'static str,
    /// The actual work value or failure, including an original returned error.
    pub work: Result<T, CommandCause<E>>,
    /// A synchronous final poll returned after cancellation/deadline was already
    /// effective. Retaining its result does not turn that overrun into success.
    /// Captured before destroying the work future: cancellation/deadline during
    /// destruction cannot reclassify its completed result.
    pub interruption_after_work: Option<Interruption>,
    /// Independent work-future destruction panic, including after a returned error.
    pub destruction_panic: Option<PanicPayload>,
    /// All cleanup observations, or failure of its own coordinator. A coordinator
    /// failure makes no cleanup completion claim and cannot discard the work result.
    pub cleanup: Result<CleanupReport, Arc<JoinError>>,
}

impl<T, E> CommandReport<T, E> {
    /// Success requires normal work, no late interruption/destruction panic and
    /// complete successful cleanup. Awaiting the owner alone does not prove this.
    pub fn is_success(&self) -> bool {
        self.work.is_ok()
            && self.interruption_after_work.is_none()
            && self.destruction_panic.is_none()
            && self.cleanup.as_ref().is_ok_and(CleanupReport::is_success)
    }
}

impl<T, E> fmt::Display for CommandReport<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "command at {}: work succeeded: {}, late interruption: {}, destruction panic: {}; ",
            self.stage,
            self.work.is_ok(),
            self.interruption_after_work.is_some(),
            self.destruction_panic.is_some()
        )?;
        match &self.cleanup {
            Ok(report) => fmt::Display::fmt(report, f),
            Err(_) => f.write_str("cleanup coordinator terminated without a report"),
        }
    }
}

impl<T, E> fmt::Debug for CommandReport<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl<T, E: Error + 'static> Error for CommandReport<T, E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.work {
            Err(CommandCause::Failed(error)) => Some(error),
            _ => None,
        }
    }
}

/// Shared complete command report. All clones retain identical values and errors;
/// observations never prolong command ownership. Binding a must-use value does
/// not prove inspection; [`check_command`] checks its complete success contract.
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter::command::RunningCommand;
/// async fn ignored(command: RunningCommand<(), std::io::Error>) {
///     command.wait().await.unwrap();
/// }
/// ```
#[must_use = "inspect the command report, or use check_command"]
pub struct SharedCommandReport<T, E>(pub(super) Arc<CommandReport<T, E>>);

impl<T, E> Clone for SharedCommandReport<T, E> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T, E> Deref for SharedCommandReport<T, E> {
    type Target = CommandReport<T, E>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T, E> fmt::Debug for SharedCommandReport<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

/// Retained complete report, or a command coordinator that terminated without it.
pub type CommandOutcome<T, E> = Result<SharedCommandReport<T, E>, Arc<JoinError>>;

/// Failure to complete the full command contract. Both work and cleanup evidence
/// remain accessible; formatting does not inspect application error contents.
pub enum CommandFailure<T, E> {
    /// Completed command whose work or cleanup was unsuccessful.
    Report(SharedCommandReport<T, E>),
    /// No complete report could be produced by the command coordinator.
    Coordinator(Arc<JoinError>),
}

impl<T, E> fmt::Display for CommandFailure<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Report(report) => fmt::Display::fmt(&**report, f),
            Self::Coordinator(_) => f.write_str("command coordinator terminated without a report"),
        }
    }
}
impl<T, E> fmt::Debug for CommandFailure<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl<T: 'static, E: Error + 'static> Error for CommandFailure<T, E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Report(report) => Some(&**report),
            Self::Coordinator(error) => Some(&**error),
        }
    }
}

/// Check work and all cleanup before returning the retained successful report.
/// See [`super::Command`] for a complete finite command using this boundary.
pub fn check_command<T, E>(
    outcome: CommandOutcome<T, E>,
) -> Result<SharedCommandReport<T, E>, CommandFailure<T, E>> {
    match outcome {
        Ok(report) if report.is_success() => Ok(report),
        Ok(report) => Err(CommandFailure::Report(report)),
        Err(error) => Err(CommandFailure::Coordinator(error)),
    }
}
