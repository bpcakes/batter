use std::{error::Error, fmt};

/// A periodic run's failure, separating ordinary recurrence from escalation.
///
/// Run futures return `Result<(), PeriodicFailure<E>>`. `From<E>` produces
/// [`Self::Recoverable`], so `?` on an ordinary application error yields a
/// retained, non-escalating failure and the next scheduled invocation still
/// happens. Escalation is deliberately asymmetric: [`Self::Fatal`] has no
/// conversion and must be written out, so an ordinary error cannot implicitly
/// become a process drain. Debug and Display never format the cause.
///
/// A plain application error propagates with `?` and keeps the schedule:
///
/// ```
/// use batter_core::periodic::PeriodicFailure;
///
/// fn prune() -> Result<(), PeriodicFailure<std::io::Error>> {
///     Err(std::io::Error::other("storage unavailable"))?;
///     Ok(())
/// }
/// assert!(!prune().unwrap_err().is_fatal());
/// ```
///
/// Escalation must be written out:
///
/// ```
/// use batter_core::periodic::PeriodicFailure;
///
/// fn witness() -> Result<(), PeriodicFailure<std::io::Error>> {
///     Err(PeriodicFailure::Fatal(std::io::Error::other("witness state is unusable")))
/// }
/// assert!(witness().unwrap_err().is_fatal());
/// ```
#[must_use = "return the failure from the periodic run"]
pub enum PeriodicFailure<E> {
    /// Retained in bounded history; the next scheduled invocation still happens
    /// and no extra immediate retry is performed.
    Recoverable(E),
    /// Explicitly terminal. It initiates the existing drain sequence and the
    /// original error is retained in the process report's task records.
    Fatal(E),
}

/// One periodic run's result, for naming a run function's own signature.
///
/// ```
/// use batter_core::{operation::OperationContext, periodic::PeriodicRun};
///
/// async fn prune(scope: OperationContext) -> PeriodicRun<std::io::Error> {
///     scope.check().map_err(std::io::Error::other)?;
///     Ok(())
/// }
/// ```
pub type PeriodicRun<E> = Result<(), PeriodicFailure<E>>;

impl<E> PeriodicFailure<E> {
    /// Mark one run's failure as recoverable without relying on inference.
    pub fn recoverable(error: E) -> Self {
        Self::Recoverable(error)
    }

    /// Mark one run's failure as terminal for the process.
    pub fn fatal(error: E) -> Self {
        Self::Fatal(error)
    }

    /// Whether this failure escalates to a process drain.
    pub fn is_fatal(&self) -> bool {
        matches!(self, Self::Fatal(_))
    }

    /// Recover the original error, discarding the escalation decision.
    pub fn into_inner(self) -> E {
        match self {
            Self::Recoverable(error) | Self::Fatal(error) => error,
        }
    }

    fn cause(&self) -> &E {
        match self {
            Self::Recoverable(error) | Self::Fatal(error) => error,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Recoverable(_) => "recoverable periodic run failure",
            Self::Fatal(_) => "fatal periodic run failure",
        }
    }
}

impl<E> From<E> for PeriodicFailure<E> {
    /// Propagating an ordinary application error keeps recurrence, never escalates.
    fn from(error: E) -> Self {
        Self::Recoverable(error)
    }
}

impl<E> fmt::Debug for PeriodicFailure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl<E> fmt::Display for PeriodicFailure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl<E: Error + 'static> Error for PeriodicFailure<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause())
    }
}

/// The selected first-success initialization allowance expired before any run
/// succeeded. This is a library-owned initialization failure, distinct from an
/// application run failure, and it initiates the existing drain sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "periodic component {name} did not complete a successful run within its initialization allowance"
)]
pub struct PeriodicInitializationExpired {
    /// Validated registration name of the component that did not initialize.
    pub name: &'static str,
}
