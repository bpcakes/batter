use crate::{
    PanicPayload,
    lifecycle::{ShutdownFailure, ShutdownSuccess},
    startup::{InitializationError, StartupError},
};
use std::{fmt, sync::Arc};
use tokio::task::JoinError;

/// Original service outcome, independent of diagnostic execution.
pub enum ServiceOutcome<E> {
    /// Protected initialization failed after its owned cleanup path settled.
    StartupFailed(StartupError<InitializationError<E>>),
    /// Checked running-driver completion, retaining the native report.
    Shutdown(Result<ShutdownSuccess, ShutdownFailure>),
    /// The outer service driver terminated without publishing an outcome.
    Coordinator(Arc<JoinError>),
}

impl<E> fmt::Debug for ServiceOutcome<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::StartupFailed(_) => "ServiceOutcome::StartupFailed",
            Self::Shutdown(Ok(_)) => "ServiceOutcome::Shutdown(success)",
            Self::Shutdown(Err(_)) => "ServiceOutcome::Shutdown(failure)",
            Self::Coordinator(_) => "ServiceOutcome::Coordinator",
        })
    }
}

/// Diagnostic result; failures never replace the original service outcome.
pub enum DiagnosticOutcome<R> {
    /// Adapter completed. Inspect its report for individual export failures.
    Completed(R),
    /// Synchronous adapter installation unwound; payload is retained explicitly.
    InstallationPanicked(PanicPayload),
    /// Diagnostic future panicked or was cancelled; join error is retained.
    TaskFailed(JoinError),
}

impl<R> fmt::Debug for DiagnosticOutcome<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Completed(_) => "DiagnosticOutcome::Completed",
            Self::InstallationPanicked(_) => "DiagnosticOutcome::InstallationPanicked",
            Self::TaskFailed(_) => "DiagnosticOutcome::TaskFailed",
        })
    }
}

pub(super) struct Completed<E, R> {
    pub(super) service: ServiceOutcome<E>,
    pub(super) diagnostics: DiagnosticOutcome<R>,
}

/// Shared retained service and diagnostic results. Neither can be substituted
/// for the other; application exit policy should inspect [`Self::service`].
#[must_use = "inspect the service result separately from diagnostic outcomes"]
pub struct ServiceCompletion<E, R>(pub(super) Arc<Completed<E, R>>);

impl<E, R> Clone for ServiceCompletion<E, R> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<E, R> ServiceCompletion<E, R> {
    /// Original protected startup/shutdown outcome.
    pub fn service(&self) -> &ServiceOutcome<E> {
        &self.0.service
    }
    /// Separately retained diagnostics, including adapter panics.
    pub fn diagnostics(&self) -> &DiagnosticOutcome<R> {
        &self.0.diagnostics
    }
}

impl<E, R> fmt::Debug for ServiceCompletion<E, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceCompletion")
            .field("service", self.service())
            .field("diagnostics", self.diagnostics())
            .finish()
    }
}
