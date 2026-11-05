use crate::{PanicPayload, command::CommandCause, operation::Interruption};
use std::{fmt, ops::Deref, sync::Arc};

/// One completed phase, including a result that raced interruption or destruction.
/// Formatting never visits application values, errors or panic payloads.
#[must_use = "inspect the result and independent interruption/destruction evidence"]
pub struct PhaseOutcome<T, E> {
    pub(super) result: Result<T, CommandCause<E>>,
    pub(super) interruption: Option<Interruption>,
    pub(super) destruction: Option<PanicPayload>,
}

impl<T, E> PhaseOutcome<T, E> {
    /// The returned value, application error, interruption or polling panic.
    pub fn result(&self) -> Result<&T, &CommandCause<E>> {
        self.result.as_ref()
    }

    /// Interruption observed at the same boundary as a returned result.
    pub fn interruption_after_return(&self) -> Option<Interruption> {
        self.interruption
    }

    /// Independent panic while destroying the phase future or uncalled factory.
    pub fn destruction_panic(&self) -> Option<&PanicPayload> {
        self.destruction.as_ref()
    }

    /// True only for a normal return without late interruption or destruction panic.
    pub fn is_success(&self) -> bool {
        self.result.is_ok() && self.interruption.is_none() && self.destruction.is_none()
    }
}

impl<T, E> fmt::Debug for PhaseOutcome<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let outcome = match &self.result {
            Ok(_) => "returned",
            Err(CommandCause::Failed(_)) => "failed",
            Err(CommandCause::Interrupted(_)) => "interrupted",
            Err(CommandCause::Panicked(_)) => "panicked",
        };
        f.debug_struct("PhaseOutcome")
            .field("outcome", &outcome)
            .field("interruption", &self.interruption)
            .field("destruction_panic", &self.destruction.is_some())
            .finish()
    }
}

/// Work and finalization outcomes retained independently by one coordinator.
/// A successful finalization does not make failed work successful, or conversely.
#[must_use = "inspect both work and finalization"]
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

    /// True only when both phases completed successfully.
    pub fn is_success(&self) -> bool {
        self.work.is_success() && self.finalization.is_success()
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

/// Shared retained report; observation does not keep work alive.
#[must_use = "inspect both phases; waiting alone does not establish success"]
pub struct SharedPhasedReport<T, E, U, F>(pub(super) Arc<PhasedReport<T, E, U, F>>);

impl<T, E, U, F> Clone for SharedPhasedReport<T, E, U, F> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T, E, U, F> Deref for SharedPhasedReport<T, E, U, F> {
    type Target = PhasedReport<T, E, U, F>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T, E, U, F> fmt::Debug for SharedPhasedReport<T, E, U, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

/// Failure of the complete two-phase contract, with both outcomes retained.
/// Default formatting omits all application values, errors and panic payloads.
pub enum PhasedFailure<T, E, U, F> {
    /// At least one phase failed, was interrupted, or panicked during destruction.
    Report(SharedPhasedReport<T, E, U, F>),
    /// The coordinator terminated without a complete report.
    Coordinator(Arc<tokio::task::JoinError>),
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

type Checked<T, E, U, F> = Result<SharedPhasedReport<T, E, U, F>, PhasedFailure<T, E, U, F>>;

pub(super) fn checked<T, E, U, F>(
    outcome: Result<SharedPhasedReport<T, E, U, F>, Arc<tokio::task::JoinError>>,
) -> Checked<T, E, U, F> {
    match outcome {
        Ok(report) if report.is_success() => Ok(report),
        Ok(report) => Err(PhasedFailure::Report(report)),
        Err(error) => Err(PhasedFailure::Coordinator(error)),
    }
}
