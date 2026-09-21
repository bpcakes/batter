use std::{num::NonZeroUsize, time::Duration};

use thiserror::Error;

use crate::Capacity;

/// A backend-reported duration until a quota event.
///
/// A quota denial reports the delay until the rejected cost can be retried, and
/// an allowance reports the delay until full capacity is next available. Both
/// feed the same whole-second header fields, so both use this type.
///
/// The exact duration remains available through [`Delay::duration`].
/// [`Delay::seconds`] rounds it up to the whole seconds that HTTP `Retry-After`
/// and `RateLimit` fields require, so a header never invites a retry or
/// advertises a reset before the backend would honor it.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Delay {
    duration: Duration,
}

impl Delay {
    /// Wraps a backend-measured duration.
    pub const fn new(duration: Duration) -> Self {
        Self { duration }
    }

    /// Returns the exact backend-measured duration.
    pub const fn duration(self) -> Duration {
        self.duration
    }

    /// Returns the duration rounded up to whole seconds.
    ///
    /// Any fractional second rounds up. Values beyond the representable range
    /// saturate at [`u64::MAX`].
    pub const fn seconds(self) -> u64 {
        let seconds = self.duration.as_secs();
        if self.duration.subsec_nanos() == 0 {
            seconds
        } else {
            seconds.saturating_add(1)
        }
    }
}

impl From<Duration> for Delay {
    fn from(duration: Duration) -> Self {
        Self::new(duration)
    }
}

/// An invalid decision or batch construction.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum DecisionError {
    /// An allowance reported more available quota than its capacity.
    #[error("available quota {available} exceeds decision capacity {capacity}")]
    AvailableExceedsCapacity {
        /// Decision capacity.
        capacity: Capacity,
        /// Invalid available quota.
        available: u64,
    },
    /// A batch denial named an input index at or beyond its batch size.
    #[error("denied batch index {index} is out of range for a batch of {batch_size} checks")]
    DeniedIndexOutOfRange {
        /// Index of the denied input supplied by the caller.
        index: usize,
        /// Batch size supplied by the caller.
        batch_size: usize,
    },
    /// An allowed batch carried no allowances.
    ///
    /// Backends reject empty batches before evaluation, so an allowed batch
    /// always describes at least one consumed check.
    #[error("an allowed batch must carry at least one allowance")]
    EmptyBatch,
}

/// Validated details for quota exhaustion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuotaDenial {
    capacity: Capacity,
    retry_after: Delay,
}

impl QuotaDenial {
    /// Constructs quota-denial details.
    ///
    /// The capacity is already validated, so construction cannot fail.
    pub const fn new(capacity: Capacity, retry_after: Duration) -> Self {
        Self {
            capacity,
            retry_after: Delay::new(retry_after),
        }
    }

    /// Returns the maximum immediately available policy allowance.
    pub const fn capacity(self) -> Capacity {
        self.capacity
    }

    /// Returns the delay until the rejected cost can be retried.
    pub const fn retry_after(self) -> Delay {
        self.retry_after
    }
}

/// An enforced denial, discriminated by reason.
///
/// Each variant names one denial reason and carries exactly the metadata that
/// reason provides. Every payload is validated on its own, so any `Denial`
/// value is reportable and serializable. There is no reason-agnostic
/// accessor: consumers match every variant, so a new reason is a breaking
/// change that fails to compile in every consumer instead of landing in a
/// fallback arm.
///
/// A quota denial always contains its policy capacity and the delay until the
/// requested cost can be retried. A storage-capacity denial may contain the
/// delay until the backend's earliest known expiry, when one is available.
///
/// Process-local backends can measure the delay at evaluation time exactly.
/// Distributed backends may return a safe upper bound measured with their
/// authoritative clock, which can overstate the delay at the caller by commit
/// and transport time.
///
/// With the `serde` feature, this is an object tagged by `reason`. Durations
/// use Serde's exact `{ "secs": ..., "nanos": ... }` representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Denial {
    /// Consuming the requested cost would exceed the configured quota.
    QuotaExceeded(QuotaDenial),
    /// A bounded backend could not safely allocate storage for a new key.
    StorageCapacity {
        /// Delay until the backend's earliest known expiry, when known.
        retry_after: Option<Delay>,
    },
}

impl From<QuotaDenial> for Denial {
    fn from(denial: QuotaDenial) -> Self {
        Self::QuotaExceeded(denial)
    }
}

/// Validated allowance metadata for an allowed check.
///
/// An allowance reports the policy's maximum immediately available capacity,
/// the allowance still available after this check consumed its cost, and the
/// backend-reported delay until full capacity is next available. `available`
/// never exceeds `capacity`, so every constructible allowance is valid.
///
/// With the `serde` feature, this is an object with `capacity`, `available`,
/// and `replenishes_after` fields. Deserialization applies the same
/// validation as [`Allowance::new`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Allowance {
    capacity: Capacity,
    available: u64,
    replenishes_after: Delay,
}

impl Allowance {
    /// Constructs a validated allowance.
    ///
    /// Storage backends should pass the allowance available after consuming
    /// the check's cost. The capacity is already validated, so the only
    /// remaining invariant is that `available` does not exceed it.
    ///
    /// # Errors
    ///
    /// Returns [`DecisionError::AvailableExceedsCapacity`] when `available`
    /// exceeds `capacity`.
    pub const fn new(
        capacity: Capacity,
        available: u64,
        replenishes_after: Duration,
    ) -> Result<Self, DecisionError> {
        if available > capacity.get() {
            return Err(DecisionError::AvailableExceedsCapacity {
                capacity,
                available,
            });
        }
        Ok(Self {
            capacity,
            available,
            replenishes_after: Delay::new(replenishes_after),
        })
    }

    /// Returns the maximum immediately available policy allowance.
    pub const fn capacity(self) -> Capacity {
        self.capacity
    }

    /// Returns the allowance available after consuming this check.
    pub const fn available(self) -> u64 {
        self.available
    }

    /// Returns the delay until the policy's full capacity is next available.
    pub const fn replenishes_after(self) -> Delay {
        self.replenishes_after
    }
}

/// A read-only, discriminated view of an [`Admitted`] decision.
///
/// Both variants permit the request. Only a quota denial can be shadowed, so
/// the shadow variant carries a [`QuotaDenial`] rather than a [`Denial`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmittedView {
    /// Quota was consumed.
    Allowed {
        /// Validated allowance metadata.
        allowance: Allowance,
    },
    /// Quota was exceeded in shadow mode and nothing was consumed.
    ShadowDenied {
        /// Validated quota-exhaustion details.
        denial: QuotaDenial,
    },
}

/// A decision that permits the request.
///
/// Every admitted decision is either a consumed [`Allowance`] or a quota
/// denial observed under a shadow policy. An enforced denial is not
/// representable, so code that receives an `Admitted` value, such as a
/// request handler behind admission middleware, never re-checks enforcement.
/// [`Decision::admit`] produces one from a backend decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Admitted {
    outcome: AdmittedOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AdmittedOutcome {
    Allowed(Allowance),
    ShadowDenied(QuotaDenial),
}

impl Admitted {
    /// Constructs an admitted decision from a consumed allowance.
    pub const fn allowed(allowance: Allowance) -> Self {
        Self {
            outcome: AdmittedOutcome::Allowed(allowance),
        }
    }

    /// Constructs an admitted shadow quota denial.
    pub const fn shadow_denied(denial: QuotaDenial) -> Self {
        Self {
            outcome: AdmittedOutcome::ShadowDenied(denial),
        }
    }

    /// Returns a read-only view that discriminates every admitted outcome.
    pub const fn view(&self) -> AdmittedView {
        match self.outcome {
            AdmittedOutcome::Allowed(allowance) => AdmittedView::Allowed { allowance },
            AdmittedOutcome::ShadowDenied(denial) => AdmittedView::ShadowDenied { denial },
        }
    }
}

impl From<Allowance> for Admitted {
    fn from(allowance: Allowance) -> Self {
        Self::allowed(allowance)
    }
}

/// A read-only, discriminated view of a [`Decision`].
///
/// Each variant exposes exactly the metadata that is valid for that outcome.
/// Shadow denials contain only quota details because storage-capacity denials
/// are always enforced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionView {
    /// Quota was consumed and the request may proceed.
    Allowed {
        /// Validated allowance metadata.
        allowance: Allowance,
    },
    /// The application must enforce this denial.
    Denied {
        /// Backend-reported denial reason and details.
        denial: Denial,
    },
    /// Quota was exceeded in shadow mode, so the request may proceed.
    ShadowDenied {
        /// Validated quota-exhaustion details.
        denial: QuotaDenial,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Outcome {
    Admitted(Admitted),
    Denied(Denial),
}

/// The outcome of evaluating one check.
///
/// Allowed outcomes carry an [`Allowance`] describing the quota available
/// after the check and the backend-reported delay until full capacity is
/// replenished. Denied outcomes carry a [`Denial`]. Shadow-denied outcomes
/// carry a [`QuotaDenial`] and permit the request without consuming quota.
///
/// [`Decision::permits_request`] is the only boolean admission predicate.
/// [`Decision::admit`] splits a decision into the [`Admitted`] outcome the
/// application may proceed with or the [`Denial`] it must enforce, and
/// [`Decision::view`] names every outcome for telemetry.
///
/// With the `serde` feature, this is an object tagged by `outcome`. Invalid
/// allowed metadata, such as `available` exceeding `capacity`, is rejected,
/// and a `shadow_denied` object accepts only a `quota_exceeded` denial.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Decision {
    outcome: Outcome,
}

impl Decision {
    /// Constructs an allowed decision from a consumed allowance.
    pub const fn allowed(allowance: Allowance) -> Self {
        Self {
            outcome: Outcome::Admitted(Admitted::allowed(allowance)),
        }
    }

    /// Constructs an enforced denial.
    pub fn denied(denial: impl Into<Denial>) -> Self {
        Self {
            outcome: Outcome::Denied(denial.into()),
        }
    }

    /// Constructs a shadow quota denial.
    pub const fn shadow_denied(denial: QuotaDenial) -> Self {
        Self {
            outcome: Outcome::Admitted(Admitted::shadow_denied(denial)),
        }
    }

    /// Returns a read-only view that discriminates every valid outcome.
    pub const fn view(&self) -> DecisionView {
        match self.outcome {
            Outcome::Admitted(admitted) => match admitted.view() {
                AdmittedView::Allowed { allowance } => DecisionView::Allowed { allowance },
                AdmittedView::ShadowDenied { denial } => DecisionView::ShadowDenied { denial },
            },
            Outcome::Denied(denial) => DecisionView::Denied { denial },
        }
    }

    /// Returns whether the application may proceed.
    ///
    /// This is the only boolean admission predicate. It is true for consumed
    /// allowances and for quota denials from a shadow policy, and false for
    /// every enforced denial. Use [`Decision::admit`] to also obtain the
    /// admitted outcome or the denial to enforce, and [`Decision::view`] for
    /// telemetry that distinguishes shadow denials from allowances.
    pub const fn permits_request(&self) -> bool {
        matches!(self.outcome, Outcome::Admitted(_))
    }

    /// Splits the decision into the admitted outcome or the denial to enforce.
    ///
    /// The split is lossless: `Ok` holds the allowed or shadow-denied outcome
    /// the application may proceed with, and `Err` holds the enforced denial
    /// it must reject. `Ok` is returned exactly when
    /// [`Decision::permits_request`] is true.
    ///
    /// # Errors
    ///
    /// Returns the enforced [`Denial`] when the application must reject the
    /// request.
    pub const fn admit(self) -> Result<Admitted, Denial> {
        match self.outcome {
            Outcome::Admitted(admitted) => Ok(admitted),
            Outcome::Denied(denial) => Err(denial),
        }
    }
}

impl From<Allowance> for Decision {
    fn from(allowance: Allowance) -> Self {
        Self::allowed(allowance)
    }
}

impl From<Admitted> for Decision {
    fn from(admitted: Admitted) -> Self {
        Self {
            outcome: Outcome::Admitted(admitted),
        }
    }
}

/// The atomic outcome of evaluating checks in caller-supplied order.
///
/// An allowed batch contains one [`Allowance`] for each input check, in the
/// same order, and always at least one; a denied member is not representable.
/// A denied batch reports the original input index that failed together with
/// the size of the batch it was evaluated for; the index is validated below
/// that size when the batch is constructed, so a denied batch always describes
/// at least one check. Backends must not consume any check when returning an
/// enforced denial.
///
/// With the `serde` feature, this is an object tagged by `outcome`. Allowed
/// objects carry a nonempty `allowances` list. Denied objects carry `index`
/// and `batch_size`, and an index at or beyond the batch size is rejected. A
/// `shadow_denied` object accepts only a `quota_exceeded` denial.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchDecision {
    outcome: BatchOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BatchOutcome {
    Allowed(Vec<Allowance>),
    Denied {
        index: usize,
        batch_size: NonZeroUsize,
        denial: Denial,
    },
    ShadowDenied {
        index: usize,
        batch_size: NonZeroUsize,
        denial: QuotaDenial,
    },
}

/// A read-only, discriminated view of a [`BatchDecision`].
///
/// Allowances remain in caller order and are never empty. Denial indices
/// always refer to the original caller-supplied input order and are always
/// below the reported batch size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BatchDecisionView<'a> {
    /// Every check was allowed and consumed atomically.
    Allowed {
        /// One allowance per input check, in caller order.
        allowances: &'a [Allowance],
    },
    /// The application must enforce the named input's denial.
    Denied {
        /// Index of the denied input in caller order.
        index: usize,
        /// Number of checks in the evaluated batch; `index` is below it.
        batch_size: NonZeroUsize,
        /// Backend-reported denial reason and details.
        denial: Denial,
    },
    /// The named input exceeded quota in shadow mode.
    ShadowDenied {
        /// Index of the shadow-denied input in caller order.
        index: usize,
        /// Number of checks in the evaluated batch; `index` is below it.
        batch_size: NonZeroUsize,
        /// Validated quota-exhaustion details.
        denial: QuotaDenial,
    },
}

/// Validates a denied input index against its batch size.
const fn validate_batch_index(
    index: usize,
    batch_size: usize,
) -> Result<NonZeroUsize, DecisionError> {
    match NonZeroUsize::new(batch_size) {
        Some(batch_size) if index < batch_size.get() => Ok(batch_size),
        _ => Err(DecisionError::DeniedIndexOutOfRange { index, batch_size }),
    }
}

impl BatchDecision {
    /// Constructs an allowed batch from consumed allowances in caller order.
    ///
    /// # Errors
    ///
    /// Returns [`DecisionError::EmptyBatch`] when `allowances` is empty.
    pub fn allowed(allowances: Vec<Allowance>) -> Result<Self, DecisionError> {
        if allowances.is_empty() {
            return Err(DecisionError::EmptyBatch);
        }
        Ok(Self {
            outcome: BatchOutcome::Allowed(allowances),
        })
    }

    /// Constructs an enforced batch denial.
    ///
    /// # Errors
    ///
    /// Returns [`DecisionError::DeniedIndexOutOfRange`] when `index` is not
    /// below `batch_size`, which also rejects an empty batch.
    pub fn denied(
        index: usize,
        batch_size: usize,
        denial: impl Into<Denial>,
    ) -> Result<Self, DecisionError> {
        let batch_size = validate_batch_index(index, batch_size)?;
        Ok(Self {
            outcome: BatchOutcome::Denied {
                index,
                batch_size,
                denial: denial.into(),
            },
        })
    }

    /// Constructs a shadow batch denial.
    ///
    /// # Errors
    ///
    /// Returns [`DecisionError::DeniedIndexOutOfRange`] when `index` is not
    /// below `batch_size`, which also rejects an empty batch.
    pub const fn shadow_denied(
        index: usize,
        batch_size: usize,
        denial: QuotaDenial,
    ) -> Result<Self, DecisionError> {
        let batch_size = match validate_batch_index(index, batch_size) {
            Ok(batch_size) => batch_size,
            Err(error) => return Err(error),
        };
        Ok(Self {
            outcome: BatchOutcome::ShadowDenied {
                index,
                batch_size,
                denial,
            },
        })
    }

    /// Returns a read-only view that discriminates every valid outcome.
    pub fn view(&self) -> BatchDecisionView<'_> {
        match &self.outcome {
            BatchOutcome::Allowed(allowances) => BatchDecisionView::Allowed { allowances },
            BatchOutcome::Denied {
                index,
                batch_size,
                denial,
            } => BatchDecisionView::Denied {
                index: *index,
                batch_size: *batch_size,
                denial: *denial,
            },
            BatchOutcome::ShadowDenied {
                index,
                batch_size,
                denial,
            } => BatchDecisionView::ShadowDenied {
                index: *index,
                batch_size: *batch_size,
                denial: *denial,
            },
        }
    }

    /// Returns whether the application may proceed.
    ///
    /// This is the only boolean admission predicate. It is true for allowed
    /// and shadow-denied batches and false for every enforced denial. Match
    /// [`BatchDecision::view`] for anything else. There is no fallible
    /// conversion into the allowed allowances: its `is_ok()` would be an
    /// `is_allowed()` predicate that is false for a shadow denial.
    pub const fn permits_request(&self) -> bool {
        !matches!(self.outcome, BatchOutcome::Denied { .. })
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
enum DenialRef {
    QuotaExceeded {
        capacity: u64,
        retry_after: Duration,
    },
    StorageCapacity {
        retry_after: Option<Duration>,
    },
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case", deny_unknown_fields)]
enum DenialWire {
    QuotaExceeded {
        capacity: u64,
        retry_after: Duration,
    },
    StorageCapacity {
        retry_after: Option<Duration>,
    },
}

/// The only denial reason a shadow outcome accepts on the wire.
///
/// Deserializing a `storage_capacity` reason into a shadow outcome fails as an
/// unknown variant instead of being parsed and then rejected.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case", deny_unknown_fields)]
enum QuotaDenialWire {
    QuotaExceeded {
        capacity: u64,
        retry_after: Duration,
    },
}

#[cfg(feature = "serde")]
impl QuotaDenialWire {
    fn validate<E: serde::de::Error>(self) -> Result<QuotaDenial, E> {
        let Self::QuotaExceeded {
            capacity,
            retry_after,
        } = self;
        Capacity::new(capacity)
            .map(|capacity| QuotaDenial::new(capacity, retry_after))
            .map_err(E::custom)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Denial {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let wire = match *self {
            Self::QuotaExceeded(denial) => DenialRef::QuotaExceeded {
                capacity: denial.capacity().get(),
                retry_after: denial.retry_after().duration(),
            },
            Self::StorageCapacity { retry_after } => DenialRef::StorageCapacity {
                retry_after: retry_after.map(Delay::duration),
            },
        };
        serde::Serialize::serialize(&wire, serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Denial {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = <DenialWire as serde::Deserialize>::deserialize(deserializer)?;
        match wire {
            DenialWire::QuotaExceeded {
                capacity,
                retry_after,
            } => Capacity::new(capacity)
                .map(|capacity| Self::QuotaExceeded(QuotaDenial::new(capacity, retry_after)))
                .map_err(serde::de::Error::custom),
            DenialWire::StorageCapacity { retry_after } => Ok(Self::StorageCapacity {
                retry_after: retry_after.map(Delay::new),
            }),
        }
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct AllowanceWire {
    capacity: u64,
    available: u64,
    replenishes_after: Duration,
}

#[cfg(feature = "serde")]
impl AllowanceWire {
    fn validate<E: serde::de::Error>(self) -> Result<Allowance, E> {
        let capacity = Capacity::new(self.capacity).map_err(E::custom)?;
        Allowance::new(capacity, self.available, self.replenishes_after).map_err(E::custom)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Allowance {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let wire = AllowanceWire {
            capacity: self.capacity.get(),
            available: self.available,
            replenishes_after: self.replenishes_after.duration(),
        };
        serde::Serialize::serialize(&wire, serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Allowance {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        <AllowanceWire as serde::Deserialize>::deserialize(deserializer)?.validate()
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum DecisionRef<'a> {
    Allowed {
        capacity: u64,
        available: u64,
        replenishes_after: Duration,
    },
    Denied {
        denial: &'a Denial,
    },
    ShadowDenied {
        denial: Denial,
    },
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
enum DecisionWire {
    Allowed {
        capacity: u64,
        available: u64,
        replenishes_after: Duration,
    },
    Denied {
        denial: Denial,
    },
    ShadowDenied {
        denial: QuotaDenialWire,
    },
}

#[cfg(feature = "serde")]
impl serde::Serialize for Decision {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let wire = match &self.outcome {
            Outcome::Admitted(admitted) => match admitted.view() {
                AdmittedView::Allowed { allowance } => DecisionRef::Allowed {
                    capacity: allowance.capacity().get(),
                    available: allowance.available(),
                    replenishes_after: allowance.replenishes_after().duration(),
                },
                AdmittedView::ShadowDenied { denial } => DecisionRef::ShadowDenied {
                    denial: Denial::QuotaExceeded(denial),
                },
            },
            Outcome::Denied(denial) => DecisionRef::Denied { denial },
        };
        serde::Serialize::serialize(&wire, serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Decision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = <DecisionWire as serde::Deserialize>::deserialize(deserializer)?;
        match wire {
            DecisionWire::Allowed {
                capacity,
                available,
                replenishes_after,
            } => AllowanceWire {
                capacity,
                available,
                replenishes_after,
            }
            .validate()
            .map(Self::allowed),
            DecisionWire::Denied { denial } => Ok(Self::denied(denial)),
            DecisionWire::ShadowDenied { denial } => denial.validate().map(Self::shadow_denied),
        }
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum BatchDecisionRef<'a> {
    Allowed {
        allowances: &'a [Allowance],
    },
    Denied {
        index: usize,
        batch_size: NonZeroUsize,
        denial: &'a Denial,
    },
    ShadowDenied {
        index: usize,
        batch_size: NonZeroUsize,
        denial: Denial,
    },
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
enum BatchDecisionWire {
    Allowed {
        allowances: Vec<Allowance>,
    },
    Denied {
        index: usize,
        batch_size: usize,
        denial: Denial,
    },
    ShadowDenied {
        index: usize,
        batch_size: usize,
        denial: QuotaDenialWire,
    },
}

#[cfg(feature = "serde")]
impl serde::Serialize for BatchDecision {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let wire = match &self.outcome {
            BatchOutcome::Allowed(allowances) => BatchDecisionRef::Allowed { allowances },
            BatchOutcome::Denied {
                index,
                batch_size,
                denial,
            } => BatchDecisionRef::Denied {
                index: *index,
                batch_size: *batch_size,
                denial,
            },
            BatchOutcome::ShadowDenied {
                index,
                batch_size,
                denial,
            } => BatchDecisionRef::ShadowDenied {
                index: *index,
                batch_size: *batch_size,
                denial: Denial::QuotaExceeded(*denial),
            },
        };
        serde::Serialize::serialize(&wire, serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for BatchDecision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = <BatchDecisionWire as serde::Deserialize>::deserialize(deserializer)?;
        match wire {
            BatchDecisionWire::Allowed { allowances } => {
                Self::allowed(allowances).map_err(serde::de::Error::custom)
            }
            BatchDecisionWire::Denied {
                index,
                batch_size,
                denial,
            } => Self::denied(index, batch_size, denial).map_err(serde::de::Error::custom),
            BatchDecisionWire::ShadowDenied {
                index,
                batch_size,
                denial,
            } => Self::shadow_denied(index, batch_size, denial.validate()?)
                .map_err(serde::de::Error::custom),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, time::Duration};

    use super::{
        Admitted, AdmittedView, Allowance, BatchDecision, BatchDecisionView, Decision,
        DecisionError, DecisionView, Delay, Denial, QuotaDenial,
    };
    use crate::Capacity;

    fn capacity(value: u64) -> Capacity {
        Capacity::new(value).unwrap()
    }

    fn allowance(capacity_value: u64, available: u64, replenishes_after: Duration) -> Allowance {
        Allowance::new(capacity(capacity_value), available, replenishes_after).unwrap()
    }

    fn quota(capacity_value: u64, retry_after: Duration) -> QuotaDenial {
        QuotaDenial::new(capacity(capacity_value), retry_after)
    }

    #[test]
    fn allowed_decision_exposes_its_allowance() {
        let allowance = allowance(8, 7, Duration::from_millis(59_999));
        let decision = Decision::allowed(allowance);

        assert!(decision.permits_request());
        assert_eq!(decision, Decision::from(allowance));
        assert_eq!(decision.view(), DecisionView::Allowed { allowance });
        assert_eq!(allowance.capacity().get(), 8);
        assert_eq!(allowance.available(), 7);
        assert_eq!(
            allowance.replenishes_after().duration(),
            Duration::from_millis(59_999)
        );
        assert_eq!(allowance.replenishes_after().seconds(), 60);
    }

    #[test]
    fn quota_denial_exposes_exact_and_ceiling_retry_duration() {
        let quota = quota(8, Duration::from_millis(1_001));
        let decision = Decision::denied(quota);

        assert!(!decision.permits_request());
        assert_eq!(decision, Decision::denied(Denial::QuotaExceeded(quota)));
        assert_eq!(
            decision.view(),
            DecisionView::Denied {
                denial: Denial::QuotaExceeded(quota),
            }
        );
        assert_eq!(quota.capacity().get(), 8);
        assert_eq!(quota.retry_after().duration(), Duration::from_millis(1_001));
        assert_eq!(quota.retry_after().seconds(), 2);
    }

    #[test]
    fn delay_seconds_preserves_exact_seconds() {
        assert_eq!(Delay::new(Duration::ZERO).seconds(), 0);
        assert_eq!(Delay::new(Duration::from_secs(3)).seconds(), 3);
        assert_eq!(Delay::new(Duration::from_nanos(1)).seconds(), 1);
    }

    #[test]
    fn delay_seconds_saturates_without_losing_exact_duration() {
        let duration = Duration::new(u64::MAX, 1);
        let delay = Delay::from(duration);

        assert_eq!(delay.duration(), duration);
        assert_eq!(delay.seconds(), u64::MAX);
    }

    #[test]
    fn storage_capacity_retry_can_be_unknown() {
        let unknown = Denial::StorageCapacity { retry_after: None };
        let known = Denial::StorageCapacity {
            retry_after: Some(Delay::new(Duration::from_millis(1))),
        };

        assert_eq!(
            Decision::denied(unknown).view(),
            DecisionView::Denied { denial: unknown }
        );
        assert_eq!(
            Decision::denied(known).view(),
            DecisionView::Denied { denial: known }
        );
        assert_eq!(Decision::denied(known).admit(), Err(known));
    }

    #[test]
    fn admit_splits_every_outcome_consistently_with_the_predicate() {
        let allowance = allowance(8, 7, Duration::from_mins(1));
        let quota = quota(8, Duration::from_secs(30));
        let capacity = Denial::StorageCapacity { retry_after: None };

        for (decision, expected) in [
            (
                Decision::allowed(allowance),
                Ok(Admitted::allowed(allowance)),
            ),
            (
                Decision::shadow_denied(quota),
                Ok(Admitted::shadow_denied(quota)),
            ),
            (Decision::denied(quota), Err(Denial::QuotaExceeded(quota))),
            (Decision::denied(capacity), Err(capacity)),
        ] {
            assert_eq!(decision.admit(), expected);
            assert_eq!(decision.permits_request(), expected.is_ok());
            if let Ok(admitted) = expected {
                assert_eq!(Decision::from(admitted), decision);
            }
        }
    }

    #[test]
    fn admitted_views_discriminate_allowances_from_shadow_denials() {
        let allowance = allowance(8, 7, Duration::from_mins(1));
        let quota = quota(8, Duration::from_secs(30));

        assert_eq!(
            Admitted::allowed(allowance).view(),
            AdmittedView::Allowed { allowance }
        );
        assert_eq!(Admitted::from(allowance), Admitted::allowed(allowance));
        assert_eq!(
            Admitted::shadow_denied(quota).view(),
            AdmittedView::ShadowDenied { denial: quota }
        );
    }

    #[test]
    fn allowed_batches_are_never_empty() {
        assert_eq!(
            BatchDecision::allowed(Vec::new()),
            Err(DecisionError::EmptyBatch)
        );
    }

    #[test]
    fn batch_denial_index_must_be_below_the_batch_size() {
        let denial = quota(8, Duration::from_mins(1));

        assert_eq!(
            BatchDecision::denied(1, 1, denial),
            Err(DecisionError::DeniedIndexOutOfRange {
                index: 1,
                batch_size: 1,
            })
        );
        assert_eq!(
            BatchDecision::denied(0, 0, denial),
            Err(DecisionError::DeniedIndexOutOfRange {
                index: 0,
                batch_size: 0,
            })
        );
        assert_eq!(
            BatchDecision::shadow_denied(3, 2, denial),
            Err(DecisionError::DeniedIndexOutOfRange {
                index: 3,
                batch_size: 2,
            })
        );
        assert!(BatchDecision::denied(1, 2, denial).is_ok());
        assert!(BatchDecision::shadow_denied(1, 2, denial).is_ok());
    }

    #[test]
    fn shadow_denial_permits_the_request_without_claiming_consumption() {
        let denial = quota(8, Duration::from_millis(30_001));
        let decision = Decision::shadow_denied(denial);

        assert!(decision.permits_request());
        assert_eq!(decision.view(), DecisionView::ShadowDenied { denial });
        assert_eq!(denial.retry_after().seconds(), 31);
        assert!(
            BatchDecision::shadow_denied(0, 1, denial)
                .unwrap()
                .permits_request()
        );
        assert!(
            !BatchDecision::denied(0, 1, denial)
                .unwrap()
                .permits_request()
        );
    }

    #[test]
    fn decision_views_outlive_the_decision() {
        fn view_of_temporary() -> DecisionView {
            Decision::denied(Denial::StorageCapacity { retry_after: None }).view()
        }

        assert_eq!(
            view_of_temporary(),
            DecisionView::Denied {
                denial: Denial::StorageCapacity { retry_after: None },
            }
        );
    }

    #[test]
    fn batch_views_preserve_allowed_order_and_denial_indices() {
        let first = allowance(8, 7, Duration::from_mins(1));
        let second = allowance(4, 2, Duration::from_secs(30));
        let allowed_batch = BatchDecision::allowed(vec![first, second]).unwrap();
        assert_eq!(
            allowed_batch.view(),
            BatchDecisionView::Allowed {
                allowances: &[first, second],
            }
        );

        let quota = quota(8, Duration::from_secs(30));
        let denied = BatchDecision::denied(1, 2, quota).unwrap();
        assert_eq!(
            denied.view(),
            BatchDecisionView::Denied {
                index: 1,
                batch_size: NonZeroUsize::new(2).unwrap(),
                denial: Denial::QuotaExceeded(quota),
            }
        );

        let capacity =
            BatchDecision::denied(2, 3, Denial::StorageCapacity { retry_after: None }).unwrap();
        assert_eq!(
            capacity.view(),
            BatchDecisionView::Denied {
                index: 2,
                batch_size: NonZeroUsize::new(3).unwrap(),
                denial: Denial::StorageCapacity { retry_after: None },
            }
        );

        let shadow = BatchDecision::shadow_denied(2, 3, quota).unwrap();
        assert_eq!(
            shadow.view(),
            BatchDecisionView::ShadowDenied {
                index: 2,
                batch_size: NonZeroUsize::new(3).unwrap(),
                denial: quota,
            }
        );
    }

    #[test]
    fn invalid_metadata_is_rejected_at_construction() {
        assert_eq!(
            Allowance::new(capacity(8), 9, Duration::ZERO),
            Err(DecisionError::AvailableExceedsCapacity {
                capacity: capacity(8),
                available: 9,
            })
        );
        assert!(Allowance::new(capacity(8), 8, Duration::ZERO).is_ok());
    }
}
