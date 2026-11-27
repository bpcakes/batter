//! The one storage-independent attempt lease transition, for backend authors.
//! [`AttemptState`] documents the contract; the sibling `evaluation` module
//! holds what its transitions return.

use std::{num::NonZeroU32, time::Duration};

use super::{
    AdmitEvaluation, AppliedCompletion, AttemptCompletion, AttemptOutcome, AttemptPolicy,
    CompleteEvaluation, DeniedAdmission, HeldAttempt, PendingLease, ReleasedAttempt,
};
use crate::Delay;

/// Everything a backend retains about one subject under one attempt policy.
///
/// `T` is the backend's lease token, and equality with the live lease's token
/// is the only fence [`Self::complete`] checks. A backend that fences
/// completion outside this type, such as a database statement comparing its
/// own token, decodes with `T = `[`StorageFenced`], which admits and settles
/// but cannot complete, and owes the same token rule there.
///
/// # Backend obligations
///
/// A backend decodes its storage into this state, calls [`Self::admit`] or
/// [`Self::complete`], and persists what the evaluation returns. It never
/// re-derives expiry, the quiet-period reset, a denial or a delay. The
/// transition is pure, so these remain the backend's and no type checks them:
///
/// - Time is a [`Duration`] since a backend-owned epoch, from one clock
///   authority. The backend chooses what a regressing clock means: clamp its
///   samples, as the memory backend does, or accept that an earlier sample
///   re-evaluates the stored state at that earlier instant, as the PostgreSQL
///   backend does with its server clock. The second choice has a visible
///   cost wherever a denial or a stale completion left the stored state
///   alone: a lease reported expired is live again before its deadline.
///   Samples stay below [`Duration::MAX`]. No later deadline is representable
///   at that last instant, so a lease granted there is already expired.
/// - A token is never issued again for a subject while a receipt that carries
///   it can still be presented, including after the subject's stored state
///   was removed and created again. Telling a lease from the one before it is
///   not enough: a counter that restarts with the stored state issues 1, 2, 1,
///   and a late completion of the first lease then applies to the third. The
///   memory backend numbers leases from one counter for the whole store that
///   never restarts; the PostgreSQL backend generates a random token for each.
/// - A completion is evaluated against the state its receipt was issued for.
///   The rule above is per subject, so two subjects may hold equal tokens:
///   the store, policy and subject come from the receipt itself, never from
///   the caller beside it, unless the token alone tells all of them apart.
/// - Transitions for one subject are serialized.
/// - A state is evaluated only with the policy whose fingerprint keys it.
///   Another policy would reinterpret the stored failure count and instants.
/// - A granted lease or an applied completion is stored before its result is
///   reported.
///
/// ```
/// use std::time::Duration;
///
/// use runlimit_core::{
///     PolicyId, QuotaPeriod, ScopeId,
///     attempts::{
///         AdmitEvaluation, AttemptOutcome, AttemptPolicy, AttemptState, CompleteEvaluation,
///         LeaseDenial, ReleasedAttempt,
///     },
/// };
///
/// let seconds = |value| QuotaPeriod::new(Duration::from_secs(value)).unwrap();
/// let policy = AttemptPolicy::new(
///     PolicyId::new("sign-in").unwrap(),
///     ScopeId::new("account").unwrap(),
///     seconds(10),
///     seconds(40),
///     seconds(100),
///     seconds(20),
/// )
/// .unwrap();
///
/// // A subject without a row is `Clear`. The backend issues its own token.
/// let now = Duration::from_secs(1_000);
/// let AdmitEvaluation::Grant(pending) = AttemptState::<u64>::Clear.admit(&policy, now) else {
///     unreachable!("a clear subject is always admitted");
/// };
/// let leased = AttemptState::Leased(pending.issue(7));
///
/// // A second admission is denied and leaves the stored state alone.
/// let AdmitEvaluation::Denied(denied) = leased.admit(&policy, now) else {
///     unreachable!("one lease is live");
/// };
/// assert!(matches!(denied.denial(), LeaseDenial::Busy { .. }));
/// assert_eq!(AttemptState::from(denied.held()), leased);
///
/// // Only the token that holds the live lease completes it.
/// let CompleteEvaluation::Applied(applied) =
///     leased.complete(&policy, now, &7, AttemptOutcome::Failure)
/// else {
///     unreachable!("the token holds the live lease");
/// };
/// assert_eq!(applied.completion().retry_after().duration(), Duration::from_secs(10));
/// assert!(matches!(applied.released(), ReleasedAttempt::Failed(_)));
/// ```
///
/// The variants are the whole lifecycle. A lease has no retry time and a retry
/// time has no lease, so neither can be stored beside the other, and a failure
/// run cannot be empty:
///
/// ```compile_fail,E0308
/// # use std::time::Duration;
/// use runlimit_core::attempts::FailedAttempts;
/// let no_failures: u32 = 0;
/// FailedAttempts::new(no_failures, Duration::ZERO, Duration::ZERO);
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptState<T> {
    /// No reservation and no retained failure. A subject the backend has never
    /// stored is in this state, as is one whose last attempt succeeded.
    Clear,
    /// One attempt is reserved. When its lease ends uncompleted it counts as a
    /// failure at the lease deadline, not at the time it is noticed.
    Leased(AttemptLease<T>),
    /// No reservation; consecutive failures delay the next admission.
    Failed(FailedAttempts),
}

/// One reservation and the failures that preceded it.
///
/// `Debug` omits the token: it is the value [`AttemptState::complete`] accepts,
/// and a lease is printed whenever a state or a busy denial is.
///
/// ```
/// # use std::time::Duration;
/// use runlimit_core::attempts::AttemptLease;
/// let lease = AttemptLease::new("fence-9f2c", 0, Duration::from_secs(5));
/// assert!(!format!("{lease:?}").contains("fence-9f2c"));
/// ```
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AttemptLease<T> {
    token: T,
    failures: u32,
    until: Duration,
}

impl<T> std::fmt::Debug for AttemptLease<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttemptLease")
            .field("failures", &self.failures)
            .field("until", &self.until)
            .finish_non_exhaustive()
    }
}

impl<T> AttemptLease<T> {
    /// Rebuilds a lease from backend storage. A new lease comes from
    /// [`PendingLease::issue`] instead.
    pub const fn new(token: T, failures: u32, until: Duration) -> Self {
        Self {
            token,
            failures,
            until,
        }
    }
    /// The backend token that completes this reservation.
    pub const fn token(&self) -> &T {
        &self.token
    }
    /// Consecutive failures before this reservation was admitted.
    pub const fn failures(&self) -> u32 {
        self.failures
    }
    /// The instant from which the reservation is expired.
    pub const fn until(&self) -> Duration {
        self.until
    }
}

/// Stands where a lease token would in a state decoded without one, because
/// the backend's storage compares the token itself.
///
/// It is deliberately not [`Eq`], which [`AttemptState::complete`] requires.
/// A unit token would equal itself and complete any live lease, a stale
/// receipt's replacement included, so a completion that skipped the storage
/// fence does not compile:
///
/// ```compile_fail,E0277
/// # use std::time::Duration;
/// use runlimit_core::attempts::{AttemptOutcome, AttemptPolicy, AttemptState, StorageFenced};
/// fn complete(policy: &AttemptPolicy, decoded: AttemptState<StorageFenced>) {
///     let _ = decoded.complete(policy, Duration::ZERO, &StorageFenced, AttemptOutcome::Success);
/// }
/// ```
///
/// Decoded states still compare, and [`AttemptState::admit`] and
/// [`AttemptState::settle`] evaluate them as they do any other.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StorageFenced;

/// A nonzero run of consecutive failures and the delay it imposes.
///
/// `retry_at` is stored rather than derived because a backend persists it and
/// an operator may move it. An instant at or before `last_failure` releases the
/// subject without forgetting its failures. An instant after the quiet period
/// ends cannot hold the subject longer: the failures clear there first, and a
/// backoff denial reports the wait until then.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FailedAttempts {
    failures: NonZeroU32,
    last_failure: Duration,
    retry_at: Duration,
}

impl FailedAttempts {
    /// Rebuilds failure state from backend storage. Transitions derive it from
    /// the policy; nothing else should compute `retry_at`.
    pub const fn new(failures: NonZeroU32, last_failure: Duration, retry_at: Duration) -> Self {
        Self {
            failures,
            last_failure,
            retry_at,
        }
    }
    /// Consecutive failures, saturating at `u32::MAX`.
    pub const fn failures(&self) -> NonZeroU32 {
        self.failures
    }
    /// When the latest failure counted. The quiet period runs from here.
    pub const fn last_failure(&self) -> Duration {
        self.last_failure
    }
    /// The stored end of the delay. Another attempt may be admitted from this
    /// instant, or from the end of the quiet period when that comes first.
    pub const fn retry_at(&self) -> Duration {
        self.retry_at
    }
    /// The end of the quiet period: the instant these failures clear.
    fn clears_at(&self, policy: &AttemptPolicy) -> Duration {
        self.last_failure
            .saturating_add(policy.quiet_period().duration())
    }
    /// The wait at `now` until another attempt may be admitted: until the
    /// stored retry time or until the failures clear, whichever comes first.
    /// A backoff denial and a failed completion both report it from here.
    fn retry_after(&self, policy: &AttemptPolicy, now: Duration) -> Delay {
        let admits_at = self.retry_at.min(self.clears_at(policy));
        Delay::new(admits_at.saturating_sub(now))
    }
}

impl<T> AttemptState<T> {
    /// Applies every transition that the passage of time alone causes, in the
    /// one order that is correct: an expired lease first counts as a failure
    /// at its deadline, and only then can the quiet period clear the failures.
    ///
    /// Settling is idempotent, and settling at an earlier instant first never
    /// changes the result at a later one. The reverse does not hold: a state
    /// settled past a deadline stays failed at an instant before it.
    #[must_use]
    pub fn settle(self, policy: &AttemptPolicy, now: Duration) -> Self {
        let failed = match self {
            Self::Clear => return Self::Clear,
            Self::Leased(lease) if now < lease.until => return Self::Leased(lease),
            Self::Leased(lease) => fail(policy, lease.failures, lease.until),
            Self::Failed(failed) => failed,
        };
        if now >= failed.clears_at(policy) {
            Self::Clear
        } else {
            Self::Failed(failed)
        }
    }

    /// Evaluates one admission at `now` without changing anything.
    ///
    /// A granted lease is live at `now` for every instant below
    /// [`Duration::MAX`], which the backend obligations exclude.
    pub fn admit(self, policy: &AttemptPolicy, now: Duration) -> AdmitEvaluation<T> {
        let failures = match self.settle(policy, now) {
            Self::Clear => 0,
            Self::Leased(lease) => {
                return AdmitEvaluation::Denied(DeniedAdmission {
                    retry_after: Delay::new(lease.until.saturating_sub(now)),
                    held: HeldAttempt::Leased(lease),
                });
            }
            Self::Failed(failed) if now < failed.retry_at => {
                // A restored retry time may lie past the end of the quiet
                // period, and admission resumes at whichever comes first.
                return AdmitEvaluation::Denied(DeniedAdmission {
                    retry_after: failed.retry_after(policy, now),
                    held: HeldAttempt::Failed(failed),
                });
            }
            Self::Failed(failed) => failed.failures.get(),
        };
        AdmitEvaluation::Grant(PendingLease {
            failures,
            until: now.saturating_add(policy.lease().duration()),
        })
    }

    /// Evaluates the completion presented by `token` at `now`.
    ///
    /// The outcome applies only while `token` holds the live lease. A lease
    /// that has expired by `now` has already failed, so its own token is stale.
    ///
    /// A token type is [`Eq`]: a token that could differ from itself, as a
    /// floating-point `NaN` does, would leave its own live lease to expire as
    /// a failure. [`StorageFenced`] is no token and cannot complete.
    pub fn complete(
        self,
        policy: &AttemptPolicy,
        now: Duration,
        token: &T,
        outcome: AttemptOutcome,
    ) -> CompleteEvaluation<T>
    where
        T: Eq,
    {
        let lease = match self.settle(policy, now) {
            Self::Leased(lease) if lease.token == *token => lease,
            settled => return CompleteEvaluation::Stale { settled },
        };
        match outcome {
            AttemptOutcome::Success => CompleteEvaluation::Applied(AppliedCompletion {
                released: ReleasedAttempt::Clear,
                completion: AttemptCompletion {
                    outcome,
                    consecutive_failures: 0,
                    retry_after: Delay::new(Duration::ZERO),
                },
            }),
            AttemptOutcome::Failure | AttemptOutcome::Abandoned => {
                let failed = fail(policy, lease.failures, now);
                CompleteEvaluation::Applied(AppliedCompletion {
                    released: ReleasedAttempt::Failed(failed),
                    completion: AttemptCompletion {
                        outcome,
                        consecutive_failures: failed.failures.get(),
                        // The policy delay, unless the stored deadline
                        // saturated at the end of representable time.
                        retry_after: failed.retry_after(policy, now),
                    },
                })
            }
        }
    }
}

/// Counts one more failure at `at`. Every failure, whether completed or
/// expired, takes its count and its delay from here.
fn fail(policy: &AttemptPolicy, prior: u32, at: Duration) -> FailedAttempts {
    let failures = NonZeroU32::MIN.saturating_add(prior);
    FailedAttempts {
        failures,
        last_failure: at,
        retry_at: at.saturating_add(policy.delay(failures.get())),
    }
}
