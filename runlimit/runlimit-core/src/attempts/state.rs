//! The one storage-independent attempt lease transition, for backend authors.
//! [`AttemptState`] documents the contract.

use std::{num::NonZeroU32, time::Duration};

use super::{AttemptCompletion, AttemptDenial, AttemptOutcome, AttemptPolicy};
use crate::Delay;

/// Everything a backend retains about one subject under one attempt policy.
///
/// `T` is the backend's lease token: whatever distinguishes one reservation
/// from the next for the same subject. A backend that fences completion
/// outside this type, such as a database statement comparing its own token,
/// decodes with `T = ()`.
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
///         LeaseDenial,
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
/// let AdmitEvaluation::Denied { settled, denial } = leased.admit(&policy, now) else {
///     unreachable!("one lease is live");
/// };
/// assert_eq!(settled, leased);
/// assert!(matches!(denial, LeaseDenial::Busy { .. }));
///
/// // Only the token that holds the live lease completes it.
/// let CompleteEvaluation::Applied { state, completion } =
///     leased.complete(&policy, now, &7, AttemptOutcome::Failure)
/// else {
///     unreachable!("the token holds the live lease");
/// };
/// assert_eq!(completion.retry_after().duration(), Duration::from_secs(10));
/// assert!(matches!(state, AttemptState::Failed(_)));
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

/// A nonzero run of consecutive failures and the delay it imposes.
///
/// `retry_at` is stored rather than derived because a backend persists it and
/// an operator may move it. An instant at or before `last_failure` releases the
/// subject without forgetting its failures.
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
    /// The first instant at which another attempt may be admitted.
    pub const fn retry_at(&self) -> Duration {
        self.retry_at
    }
}

/// An evaluated reservation that the backend has not persisted yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingLease {
    failures: u32,
    until: Duration,
}

impl PendingLease {
    /// Consecutive failures the new reservation carries forward.
    pub const fn failures(&self) -> u32 {
        self.failures
    }
    /// The instant from which the new reservation is expired.
    pub const fn until(&self) -> Duration {
        self.until
    }
    /// Binds the backend's token, giving the lease to store.
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
    Denied {
        /// The state at the evaluation instant. A backend may store it or
        /// leave the stored state alone. While evaluation instants never
        /// decrease the two give the same answers later. They differ after a
        /// clock regression, because an unsettled expired lease is live again
        /// at an instant before its deadline and a stored failure is not, so
        /// a backend that leaves it has chosen re-evaluation at the earlier
        /// instant, as [`AttemptState`]'s backend obligations describe.
        settled: AttemptState<T>,
        /// Why admission was refused.
        denial: LeaseDenial,
    },
}

/// The provisional result of evaluating one completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use = "an evaluation changes nothing until the backend stores its state"]
pub enum CompleteEvaluation<T> {
    /// The token held the live lease. The backend must store `state` before it
    /// reports `completion`.
    Applied {
        /// [`AttemptState::Clear`] after success, otherwise
        /// [`AttemptState::Failed`].
        state: AttemptState<T>,
        /// Metadata for the applied outcome.
        completion: AttemptCompletion,
    },
    /// The token holds no live lease: the lease expired, was completed, or was
    /// replaced. The outcome was not applied.
    Stale {
        /// The state at the evaluation instant, with an expired lease already
        /// counted as a failure. Storing it is the same choice as for
        /// [`AdmitEvaluation::Denied`]'s settled state.
        settled: AttemptState<T>,
    },
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
        let quiet_from = failed
            .last_failure
            .saturating_add(policy.quiet_period().duration());
        if now >= quiet_from {
            Self::Clear
        } else {
            Self::Failed(failed)
        }
    }

    /// Evaluates one admission at `now` without changing anything.
    pub fn admit(self, policy: &AttemptPolicy, now: Duration) -> AdmitEvaluation<T> {
        let failures = match self.settle(policy, now) {
            Self::Clear => 0,
            Self::Leased(lease) => {
                return AdmitEvaluation::Denied {
                    denial: LeaseDenial::Busy {
                        retry_after: Delay::new(lease.until.saturating_sub(now)),
                    },
                    settled: Self::Leased(lease),
                };
            }
            Self::Failed(failed) if now < failed.retry_at => {
                return AdmitEvaluation::Denied {
                    denial: LeaseDenial::Backoff {
                        retry_after: Delay::new(failed.retry_at.saturating_sub(now)),
                    },
                    settled: Self::Failed(failed),
                };
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
    pub fn complete(
        self,
        policy: &AttemptPolicy,
        now: Duration,
        token: &T,
        outcome: AttemptOutcome,
    ) -> CompleteEvaluation<T>
    where
        T: PartialEq,
    {
        let lease = match self.settle(policy, now) {
            Self::Leased(lease) if lease.token == *token => lease,
            settled => return CompleteEvaluation::Stale { settled },
        };
        match outcome {
            AttemptOutcome::Success => CompleteEvaluation::Applied {
                state: Self::Clear,
                completion: AttemptCompletion {
                    outcome,
                    consecutive_failures: 0,
                    retry_after: Delay::new(Duration::ZERO),
                },
            },
            AttemptOutcome::Failure | AttemptOutcome::Abandoned => {
                let failed = fail(policy, lease.failures, now);
                CompleteEvaluation::Applied {
                    state: Self::Failed(failed),
                    completion: AttemptCompletion {
                        outcome,
                        consecutive_failures: failed.failures.get(),
                        retry_after: Delay::new(policy.delay(failed.failures.get())),
                    },
                }
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
