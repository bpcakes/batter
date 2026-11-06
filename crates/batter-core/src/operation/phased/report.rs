use super::outcome::PhaseOutcome;
use std::{fmt, ops::Deref, sync::Arc};

/// Independently retained work and finalization evidence.
/// Only a failed complete operation exposes this diagnostic report.
pub struct PhasedReport<T, E, U, F> {
    pub(super) work: PhaseOutcome<T, E>,
    pub(super) finalization: PhaseOutcome<U, F>,
}

impl<T, E, U, F> PhasedReport<T, E, U, F> {
    /// Work evidence retained before finalization starts.
    pub fn work(&self) -> &PhaseOutcome<T, E> {
        &self.work
    }
    /// Finalization evidence, without replacing work evidence.
    pub fn finalization(&self) -> &PhaseOutcome<U, F> {
        &self.finalization
    }

    pub(super) fn into_checked(self) -> Checked<T, E, U, F> {
        match self {
            Self {
                work: PhaseOutcome(Ok(work)),
                finalization: PhaseOutcome(Ok(finalization)),
            } => Ok(PhasedSuccess(Arc::new((work, finalization)))),
            failed => Err(PhasedFailure::Report(FailedPhasedReport(Arc::new(failed)))),
        }
    }

    pub(super) fn into_owned_checked(self) -> Result<(T, U), PhasedFailure<T, E, U, F>> {
        match self {
            Self {
                work: PhaseOutcome(Ok(work)),
                finalization: PhaseOutcome(Ok(finalization)),
            } => Ok((work, finalization)),
            failed => Err(PhasedFailure::Report(FailedPhasedReport(Arc::new(failed)))),
        }
    }
}

impl<T, E, U, F> fmt::Debug for PhasedReport<T, E, U, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PhasedReport")
            .field("work", &self.work)
            .field("finalization", &self.finalization)
            .finish()
    }
}

/// Proof that both phases succeeded, with no late interruption/destruction panic.
/// Values are shared without requiring `Clone`; formatting never visits them.
///
/// Migration: `RunningPhasedOperation::wait` and `PhasedObserver::wait` now return
/// `PhasedSuccess` instead of `SharedPhasedReport` on success. Access `work()` and
/// `finalization()` directly.
/// Migration: `PhasedReport::is_success` is removed; the result types now
/// distinguish accepted values from failed evidence.
///
/// ```compile_fail,E0423
/// use batter_core::operation::PhasedSuccess;
/// let forged = PhasedSuccess(std::sync::Arc::new((1, 2)));
/// ```
/// ```compile_fail,E0308
/// use batter_core::operation::{PhasedSuccess, PhasedFailure};
/// fn forge(success: PhasedSuccess<(), ()>) -> PhasedFailure<(), (), (), ()> {
///     PhasedFailure::Report(success)
/// }
/// ```
pub struct PhasedSuccess<T, U>(Arc<(T, U)>);
impl<T, U> PhasedSuccess<T, U> {
    /// The accepted work value.
    pub fn work(&self) -> &T {
        &self.0.0
    }
    /// The accepted finalization value.
    pub fn finalization(&self) -> &U {
        &self.0.1
    }
}
impl<T, U> Clone for PhasedSuccess<T, U> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T, U> fmt::Debug for PhasedSuccess<T, U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("phased operation succeeded")
    }
}

/// Shared proof that at least one phase failed. Both outcomes remain inspectable.
/// Migration: `SharedPhasedReport` on the error path is now `FailedPhasedReport`;
/// callers cannot construct it from a successful report or successful values.
///
/// ```compile_fail,E0423
/// use batter_core::operation::{FailedPhasedReport, PhasedReport};
/// fn forge(report: PhasedReport<(), (), (), ()>) {
///     let _ = FailedPhasedReport(std::sync::Arc::new(report));
/// }
/// ```
#[must_use = "inspect both retained phase outcomes"]
pub struct FailedPhasedReport<T, E, U, F>(Arc<PhasedReport<T, E, U, F>>);
impl<T, E, U, F> Clone for FailedPhasedReport<T, E, U, F> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T, E, U, F> Deref for FailedPhasedReport<T, E, U, F> {
    type Target = PhasedReport<T, E, U, F>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T, E, U, F> fmt::Debug for FailedPhasedReport<T, E, U, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

/// Failure of the complete contract, retaining both outcomes or coordinator failure.
/// Migration: `PhasedFailure::Report` accepts only `FailedPhasedReport`, not a
/// general `SharedPhasedReport`. Success cannot be relabeled as failed evidence.
pub enum PhasedFailure<T, E, U, F> {
    /// At least one phase failed, was interrupted, or panicked during destruction.
    Report(FailedPhasedReport<T, E, U, F>),
    /// The coordinator terminated without a complete report.
    Coordinator(Arc<tokio::task::JoinError>),
}
impl<T, E, U, F> Clone for PhasedFailure<T, E, U, F> {
    fn clone(&self) -> Self {
        match self {
            Self::Report(report) => Self::Report(report.clone()),
            Self::Coordinator(error) => Self::Coordinator(error.clone()),
        }
    }
}
impl<T, E, U, F> fmt::Display for PhasedFailure<T, E, U, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Report(report) => write!(f, "phased operation failed: {report:?}"),
            Self::Coordinator(_) => f.write_str("phase coordinator terminated without a report"),
        }
    }
}
impl<T, E, U, F> fmt::Debug for PhasedFailure<T, E, U, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl<T: 'static, E: 'static, U: 'static, F: 'static> std::error::Error
    for PhasedFailure<T, E, U, F>
{
}

pub(super) type Checked<T, E, U, F> = Result<PhasedSuccess<T, U>, PhasedFailure<T, E, U, F>>;
