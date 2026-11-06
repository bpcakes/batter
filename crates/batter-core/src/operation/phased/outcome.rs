use crate::{PanicPayload, command::CommandCause, operation::Interruption};
use std::fmt;

/// A fully classified phase. A returned value alone is not success evidence.
/// The library checks interruption and destruction before constructing this value.
#[must_use = "classify the complete phase evidence"]
pub struct PhaseOutcome<T, E>(pub(super) Result<T, PhaseFailure<T, E>>);

impl<T, E> PhaseOutcome<T, E> {
    pub(super) fn new(
        result: Result<T, CommandCause<E>>,
        interruption: Option<Interruption>,
        destruction: Option<PanicPayload>,
    ) -> Self {
        Self(match (result, interruption, destruction) {
            (Ok(value), None, None) => Ok(value),
            (result, interruption, destruction) => Err(PhaseFailure {
                result,
                interruption,
                destruction,
            }),
        })
    }

    /// Accepted success or the complete failed-phase evidence.
    ///
    /// Migration: `PhaseOutcome::result` is replaced by `classify`; its `Ok`
    /// now proves no late interruption or destruction panic occurred. Inspect
    /// [`PhaseFailure::returned_result`] explicitly for a value retained on failure.
    pub fn classify(&self) -> Result<&T, &PhaseFailure<T, E>> {
        self.0.as_ref()
    }

    /// True only for an accepted return without interruption or destruction panic.
    pub fn is_success(&self) -> bool {
        self.0.is_ok()
    }
}

impl<T, E> fmt::Debug for PhaseOutcome<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Ok(_) => f.write_str("phase succeeded"),
            Err(failure) => fmt::Debug::fmt(failure, f),
        }
    }
}

/// Evidence of an unsuccessful phase; only the library can construct it.
/// A late returned value is retained without being classified as success.
///
/// ```compile_fail,E0451
/// use batter_core::operation::PhaseFailure;
/// let failure = PhaseFailure::<(), ()> { result: Ok(()), interruption: None, destruction: None };
/// ```
pub struct PhaseFailure<T, E> {
    result: Result<T, CommandCause<E>>,
    interruption: Option<Interruption>,
    destruction: Option<PanicPayload>,
}

impl<T, E> PhaseFailure<T, E> {
    /// Raw returned evidence. `Ok` here does NOT mean the phase succeeded.
    pub fn returned_result(&self) -> Result<&T, &CommandCause<E>> {
        self.result.as_ref()
    }

    /// Interruption observed at the return boundary, before future destruction.
    /// Migration: `PhaseOutcome::interruption_after_return` now belongs to the
    /// failed evidence returned by [`PhaseOutcome::classify`].
    pub fn interruption_after_return(&self) -> Option<Interruption> {
        self.interruption
    }

    /// Independent panic while destroying the future or uncalled factory.
    /// Migration: `PhaseOutcome::destruction_panic` now belongs to the failed
    /// evidence returned by [`PhaseOutcome::classify`].
    pub fn destruction_panic(&self) -> Option<&PanicPayload> {
        self.destruction.as_ref()
    }
}

impl<T, E> fmt::Debug for PhaseFailure<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let outcome = match &self.result {
            Ok(_) => "returned",
            Err(CommandCause::Failed(_)) => "failed",
            Err(CommandCause::Interrupted(_)) => "interrupted",
            Err(CommandCause::Panicked(_)) => "panicked",
        };
        f.debug_struct("PhaseFailure")
            .field("outcome", &outcome)
            .field("interruption", &self.interruption)
            .field("destruction_panic", &self.destruction.is_some())
            .finish()
    }
}
