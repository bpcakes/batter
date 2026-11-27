//! What one evaluation of [`AttemptState`] returns. Only a transition builds
//! these values, so the state a result stores and the metadata it reports
//! cannot disagree and a backend checks neither against the other.

use std::time::Duration;

use super::{AttemptCompletion, AttemptDenial, AttemptLease, AttemptState, FailedAttempts};
use crate::Delay;

/// An evaluated reservation that the backend has not persisted yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingLease {
    pub(super) failures: u32,
    pub(super) until: Duration,
}

impl PendingLease {
    /// Consecutive failures the new reservation carries forward.
    pub const fn failures(&self) -> u32 {
        self.failures
    }
    /// The instant from which the new reservation is expired: the evaluation
    /// instant plus the whole policy lease.
    pub const fn until(&self) -> Duration {
        self.until
    }
    /// Binds the backend's token, giving the lease to store. No receipt still
    /// outstanding for the subject may carry that token: see the obligations
    /// on [`AttemptState`].
    pub const fn issue<T>(self, token: T) -> AttemptLease<T> {
        AttemptLease::new(token, self.failures, self.until)
    }
}

/// The denials lease state alone can produce.
///
/// [`AttemptDenial::StorageCapacity`] is absent because only a backend knows
/// its capacity. Convert with `AttemptDenial::from`.
///
/// ```compile_fail,E0599
/// let _ = runlimit_core::attempts::LeaseDenial::StorageCapacity;
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeaseDenial {
    /// Another attempt holds the lease.
    Busy {
        /// Time until that lease expires.
        retry_after: Delay,
    },
    /// Consecutive failures require waiting.
    Backoff {
        /// Remaining delay.
        retry_after: Delay,
    },
}

impl From<LeaseDenial> for AttemptDenial {
    fn from(denial: LeaseDenial) -> Self {
        match denial {
            LeaseDenial::Busy { retry_after } => Self::Busy { retry_after },
            LeaseDenial::Backoff { retry_after } => Self::Backoff { retry_after },
        }
    }
}

/// The provisional result of evaluating one admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use = "an evaluation changes nothing until the backend stores its state"]
pub enum AdmitEvaluation<T> {
    /// Admission is possible. Nothing is reserved until the backend stores
    /// [`AttemptState::Leased`] with the issued lease.
    Grant(PendingLease),
    /// No reservation was made.
    Denied(DeniedAdmission<T>),
    /// The subject would be admitted, but its lease deadline lies past the end
    /// of representable time, so no lease can be held. Nothing is reserved; a
    /// backend reports its own clock or storage range failure.
    ///
    /// ```
    /// # use std::time::Duration;
    /// # use runlimit_core::{PolicyId, QuotaPeriod, ScopeId};
    /// use runlimit_core::attempts::{AdmitEvaluation, AttemptPolicy, AttemptState};
    /// # let seconds = |value| QuotaPeriod::new(Duration::from_secs(value)).unwrap();
    /// # let policy = AttemptPolicy::new(
    /// #     PolicyId::new("sign-in").unwrap(),
    /// #     ScopeId::new("account").unwrap(),
    /// #     seconds(10),
    /// #     seconds(40),
    /// #     seconds(100),
    /// #     seconds(20),
    /// # )
    /// # .unwrap();
    /// // Twenty seconds of lease do not fit after this instant.
    /// let late = Duration::MAX - Duration::from_secs(19);
    /// assert_eq!(
    ///     AttemptState::<u64>::Clear.admit(&policy, late),
    ///     AdmitEvaluation::DeadlineOverflow
    /// );
    /// ```
    DeadlineOverflow,
}

/// The state of a subject that cannot be admitted. [`AttemptState::Clear`]
/// always admits, so it is not among them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeldAttempt<T> {
    /// A live lease reserves the subject.
    Leased(AttemptLease<T>),
    /// Consecutive failures delay the next admission.
    Failed(FailedAttempts),
}

impl<T> From<HeldAttempt<T>> for AttemptState<T> {
    fn from(held: HeldAttempt<T>) -> Self {
        match held {
            HeldAttempt::Leased(lease) => Self::Leased(lease),
            HeldAttempt::Failed(failed) => Self::Failed(failed),
        }
    }
}

/// A refused admission: the state holding the subject back, and the denial
/// that state gives at the evaluation instant.
///
/// Only [`AttemptState::admit`] builds one, so a live lease is always `Busy`
/// until its deadline and failures are always `Backoff`:
///
/// ```compile_fail,E0451
/// use runlimit_core::{Delay, attempts::{DeniedAdmission, FailedAttempts, HeldAttempt}};
/// fn mismatched(failed: FailedAttempts, retry_after: Delay) -> DeniedAdmission<u64> {
///     DeniedAdmission { held: HeldAttempt::Failed(failed), retry_after }
/// }
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use = "an evaluation changes nothing until the backend stores its state"]
pub struct DeniedAdmission<T> {
    pub(super) held: HeldAttempt<T>,
    pub(super) retry_after: Delay,
}

impl<T> DeniedAdmission<T> {
    /// The state at the evaluation instant, with an expired lease already
    /// counted as a failure. A backend may store it or leave the stored state
    /// alone. While evaluation instants never decrease the two give the same
    /// answers later. They differ after a clock regression, because an
    /// unsettled expired lease is live again at an instant before its deadline
    /// and a stored failure is not, so a backend that leaves it has chosen
    /// re-evaluation at the earlier instant, as [`AttemptState`]'s backend
    /// obligations describe.
    pub fn held(self) -> HeldAttempt<T> {
        self.held
    }
    /// Why admission was refused, and for how long.
    pub const fn denial(&self) -> LeaseDenial {
        let retry_after = self.retry_after;
        match self.held {
            HeldAttempt::Leased(_) => LeaseDenial::Busy { retry_after },
            HeldAttempt::Failed(_) => LeaseDenial::Backoff { retry_after },
        }
    }
}

/// The provisional result of evaluating one completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use = "an evaluation changes nothing until the backend stores its state"]
pub enum CompleteEvaluation<T> {
    /// The token held the live lease.
    Applied(AppliedCompletion),
    /// The token holds no live lease: the lease expired, was completed, or was
    /// replaced. The outcome was not applied.
    Stale {
        /// The state at the evaluation instant, with an expired lease already
        /// counted as a failure. Storing it is the same choice as for
        /// [`DeniedAdmission::held`].
        settled: AttemptState<T>,
    },
}

/// The state an applied completion leaves. The lease is released either way,
/// so [`AttemptState::Leased`] is not among them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleasedAttempt {
    /// The attempt succeeded and no failure is retained.
    Clear,
    /// The attempt failed or was abandoned.
    Failed(FailedAttempts),
}

impl<T> From<ReleasedAttempt> for AttemptState<T> {
    fn from(released: ReleasedAttempt) -> Self {
        match released {
            ReleasedAttempt::Clear => Self::Clear,
            ReleasedAttempt::Failed(failed) => Self::Failed(failed),
        }
    }
}

/// An applied completion: the state to store, and the metadata that state
/// gives at the evaluation instant.
///
/// Only [`AttemptState::complete`] builds one, so a success is always stored
/// as [`ReleasedAttempt::Clear`] and a failure with its own count and delay:
///
/// ```compile_fail,E0451
/// use runlimit_core::attempts::{AppliedCompletion, AttemptCompletion, FailedAttempts, ReleasedAttempt};
/// fn mismatched(success: AttemptCompletion, failed: FailedAttempts) -> AppliedCompletion {
///     AppliedCompletion { released: ReleasedAttempt::Failed(failed), completion: success }
/// }
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use = "an evaluation changes nothing until the backend stores its state"]
pub struct AppliedCompletion {
    pub(super) released: ReleasedAttempt,
    pub(super) completion: AttemptCompletion,
}

impl AppliedCompletion {
    /// The state the backend must store before it reports
    /// [`Self::completion`].
    pub const fn released(&self) -> ReleasedAttempt {
        self.released
    }
    /// Metadata for the applied outcome.
    pub const fn completion(&self) -> AttemptCompletion {
        self.completion
    }
}
