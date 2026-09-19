use crate::{
    PanicPayload, cleanup::CleanupReport, lifecycle::SignalRegistrationError,
    operation::Interruption,
};
use std::{error::Error, fmt, sync::Arc};
use tokio::task::JoinError;

/// Stable failure envelope for the protected startup path.
///
/// Formatting is deliberately redacted and never requires the application
/// error to implement `Debug` or `Display`. Trusted callers can match the
/// concrete application or signal cause directly.
#[non_exhaustive]
pub enum InitializationError<E> {
    /// Original application initialization failure.
    Application(E),
    /// Unix signal installation or reserved-component registration failed.
    Signals(SignalRegistrationError),
    /// More than one signal policy was selected for one startup specification.
    SignalPolicyAlreadySelected,
}

impl<E> fmt::Display for InitializationError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Application(_) => "application initialization failed",
            Self::Signals(_) => "startup signal initialization failed",
            Self::SignalPolicyAlreadySelected => "startup signal policy was already selected",
        })
    }
}
impl<E> fmt::Debug for InitializationError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl<E: Error + 'static> Error for InitializationError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Application(error) => Some(error),
            Self::Signals(error) => Some(error),
            Self::SignalPolicyAlreadySelected => None,
        }
    }
}

/// Why owned initialization did not reach its running-driver handoff.
pub enum StartupCause<E> {
    /// Original application failure.
    Failed(E),
    /// Initialization deadline or cancellation; external effects may be unknown.
    Interrupted(Interruption),
    /// Process drain was requested before startup completed.
    Draining,
    /// An initializer construction or polling panic, with its original payload.
    Panicked(PanicPayload),
    /// The initializer returned successfully but its future destructor panicked.
    /// The original payload is in `StartupFailure::destruction_panic`.
    DestructionPanicked,
}

impl<E> fmt::Debug for StartupCause<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Failed(_) => "initializer failed",
            Self::Interrupted(_) => "initializer interrupted",
            Self::Draining => "startup drained",
            Self::Panicked(_) => "initializer panicked",
            Self::DestructionPanicked => "initializer destruction panicked",
        })
    }
}

/// Failed startup and all cleanup outcomes, retained independently of waiters.
/// Debug/Display omit application, cleanup and panic contents. Trusted source
/// inspection can disclose them. Panic hooks and arbitrary destructors remain
/// application-owned; aborting panics cannot be caught.
pub struct StartupFailure<E> {
    /// Last validated application stage.
    pub stage: &'static str,
    /// Original application failure or explicit lifecycle interruption.
    pub cause: StartupCause<E>,
    /// An independent panic while destroying the initializer future, if any.
    /// It never replaces a returned application failure or polling panic.
    pub destruction_panic: Option<PanicPayload>,
    /// All explicitly driven finalizer outcomes.
    pub cleanup: CleanupReport,
}

impl<E> fmt::Display for StartupFailure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "startup failed at {}; {}", self.stage, self.cleanup)
    }
}
impl<E> fmt::Debug for StartupFailure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl<E: Error + 'static> Error for StartupFailure<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.cause {
            StartupCause::Failed(error) => Some(error),
            _ => None,
        }
    }
}

/// Startup failure versus an unexpected coordinator failure without a report.
pub enum StartupError<E> {
    /// Explicitly driven startup cleanup and retained original causes.
    Failed(Arc<StartupFailure<E>>),
    /// The coordinator terminated without publishing a cleanup report.
    Coordinator(Arc<JoinError>),
}

impl<E> Clone for StartupError<E> {
    fn clone(&self) -> Self {
        match self {
            Self::Failed(error) => Self::Failed(error.clone()),
            Self::Coordinator(error) => Self::Coordinator(error.clone()),
        }
    }
}
impl<E> fmt::Display for StartupError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(error) => fmt::Display::fmt(error, f),
            Self::Coordinator(_) => f.write_str("startup coordinator terminated without a report"),
        }
    }
}
impl<E> fmt::Debug for StartupError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl<E: Error + 'static> Error for StartupError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Failed(error) => Some(&**error),
            Self::Coordinator(error) => Some(&**error),
        }
    }
}
