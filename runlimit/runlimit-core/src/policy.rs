use std::{fmt, num::NonZeroU64, time::Duration};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{PolicyId, ScopeId};

// These domain strings and the byte layouts in `fingerprint` and
// `gcra_fingerprint` are persistent cross-replica protocols. Changing any byte
// splits storage keys during rolling deployments. A deliberate incompatible
// change therefore requires a new domain version and a semver-signaled storage
// migration, never an in-place rewrite of either v1 encoding.
const FIXED_WINDOW_FINGERPRINT_DOMAIN: &[u8] = b"runlimit/fixed-window-policy/v1\0";
const GCRA_FINGERPRINT_DOMAIN: &[u8] = b"runlimit/gcra-policy/v1\0";
const MAX_EXACT_DOUBLE_INTEGER: u64 = 1_u64 << f64::MANTISSA_DIGITS;

/// Largest quota or immediate capacity supported by built-in policies.
///
/// The portable ceiling is the largest positive value representable by the
/// signed 64-bit counters used by persistent backends.
pub const MAX_LIMIT: u64 = i64::MAX as u64;

/// Largest whole-millisecond policy duration supported by built-in policies.
///
/// This deliberately conservative ceiling keeps the equivalent microsecond
/// count in the consecutive-integer range of common backend time
/// representations while still allowing durations of roughly 285 years.
pub const MAX_WINDOW_MILLIS: u64 = MAX_EXACT_DOUBLE_INTEGER / 1_000;

/// Largest policy duration supported by built-in policies.
pub const MAX_WINDOW: Duration = Duration::from_millis(MAX_WINDOW_MILLIS);

/// A validated quota or immediate capacity in the portable policy range.
///
/// A capacity is never zero and never exceeds [`MAX_LIMIT`], so every value
/// fits the signed 64-bit counters used by persistent backends. Policies,
/// allowances, and quota denials all carry a `Capacity`, which is why none of
/// them re-validates the number at construction.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Capacity(NonZeroU64);

impl Capacity {
    /// The largest portable capacity, [`MAX_LIMIT`].
    pub const MAX: Self = Self(NonZeroU64::new(MAX_LIMIT).expect("MAX_LIMIT is nonzero"));

    /// Validates a capacity.
    ///
    /// # Errors
    ///
    /// Returns [`CapacityError::Zero`] for zero and [`CapacityError::TooLarge`]
    /// for a value above [`MAX_LIMIT`].
    pub const fn new(value: u64) -> Result<Self, CapacityError> {
        match NonZeroU64::new(value) {
            None => Err(CapacityError::Zero),
            Some(_) if value > MAX_LIMIT => Err(CapacityError::TooLarge {
                actual: value,
                maximum: MAX_LIMIT,
            }),
            Some(value) => Ok(Self(value)),
        }
    }

    /// Returns the capacity as a plain integer.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Display for Capacity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl From<Capacity> for u64 {
    fn from(capacity: Capacity) -> Self {
        capacity.get()
    }
}

/// An invalid quota or immediate capacity.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum CapacityError {
    /// The capacity was zero.
    #[error("capacity must be greater than zero")]
    Zero,
    /// The capacity exceeded the portable backend maximum.
    #[error("capacity {actual} exceeds portable maximum {maximum}")]
    TooLarge {
        /// Supplied capacity.
        actual: u64,
        /// Largest capacity supported by every backend.
        maximum: u64,
    },
}

/// A validated replenishment period.
///
/// A period is never zero, is an exact whole number of milliseconds, and never
/// exceeds [`MAX_WINDOW`], so every backend can store it exactly. A
/// fixed-window policy's window and a GCRA policy's period are both
/// `QuotaPeriod` values.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct QuotaPeriod(NonZeroU64);

impl QuotaPeriod {
    /// The longest portable period, [`MAX_WINDOW`].
    pub const MAX: Self = Self(NonZeroU64::new(MAX_WINDOW_MILLIS).expect("MAX_WINDOW is nonzero"));

    /// Validates a replenishment period.
    ///
    /// # Errors
    ///
    /// Returns an error when the period is zero, has finer precision than a
    /// whole millisecond, or exceeds [`MAX_WINDOW`].
    pub fn new(period: Duration) -> Result<Self, QuotaPeriodError> {
        if period.is_zero() {
            return Err(QuotaPeriodError::Zero);
        }
        if !period.subsec_nanos().is_multiple_of(1_000_000) {
            return Err(QuotaPeriodError::NotWholeMilliseconds);
        }
        if period > MAX_WINDOW {
            return Err(QuotaPeriodError::TooLarge {
                actual: period,
                maximum: MAX_WINDOW,
            });
        }

        // Both conversions are guaranteed by the checks above; they are kept as
        // errors rather than panics so this path never unwinds.
        let millis = u64::try_from(period.as_millis()).map_err(|_| QuotaPeriodError::TooLarge {
            actual: period,
            maximum: MAX_WINDOW,
        })?;
        NonZeroU64::new(millis)
            .map(Self)
            .ok_or(QuotaPeriodError::Zero)
    }

    /// Returns the period as a duration.
    pub const fn duration(self) -> Duration {
        Duration::from_millis(self.0.get())
    }

    /// Returns the period as an exact, nonzero millisecond count.
    pub const fn millis(self) -> u64 {
        self.0.get()
    }
}

impl From<QuotaPeriod> for Duration {
    fn from(period: QuotaPeriod) -> Self {
        period.duration()
    }
}

/// An invalid replenishment period.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum QuotaPeriodError {
    /// The period was zero.
    #[error("period must be greater than zero")]
    Zero,
    /// The period had finer precision than a whole millisecond.
    #[error("period must be an exact whole number of milliseconds")]
    NotWholeMilliseconds,
    /// The period exceeded the portable backend maximum.
    #[error("period {actual:?} exceeds portable maximum {maximum:?}")]
    TooLarge {
        /// Supplied period.
        actual: Duration,
        /// Largest period supported by every backend.
        maximum: Duration,
    },
}

/// Whether quota exhaustion is enforced or reported in shadow mode.
///
/// This deployment flag is deliberately not part of a policy fingerprint.
/// Switching a policy from [`QuotaMode::Shadow`] to [`QuotaMode::Enforce`]
/// therefore keeps the counter state warmed while it was shadowed.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(rename_all = "snake_case")
)]
pub enum QuotaMode {
    /// Quota exhaustion denies the operation.
    #[default]
    Enforce,
    /// Quota exhaustion is reported but permits the operation to proceed.
    Shadow,
}

/// Backend-independent policy metadata required to construct a check.
///
/// Storage backends remain free to support one specific policy algorithm by
/// choosing it as [`crate::Limiter::Policy`]. Application adapters can be
/// generic over this trait without assuming fixed-window behavior.
///
/// Every numeric value is returned as a validated type. A third-party policy
/// therefore cannot report a zero or oversized quota, capacity, or period, and
/// checks, decisions, and HTTP encoders built from it never re-validate them.
pub trait RateLimitPolicy: fmt::Debug + Send + Sync {
    /// Returns the application-defined policy identifier.
    fn id(&self) -> &PolicyId;

    /// Returns the application-defined policy scope.
    fn scope(&self) -> &ScopeId;

    /// Returns the quota replenished during [`Self::quota_period`].
    fn quota(&self) -> Capacity;

    /// Returns the period during which [`Self::quota`] is replenished.
    fn quota_period(&self) -> QuotaPeriod;

    /// Returns the largest single cost and maximum immediately available
    /// allowance supported by this policy.
    fn capacity(&self) -> Capacity;

    /// Returns the deterministic storage-key fingerprint.
    fn fingerprint(&self) -> PolicyFingerprint;

    /// Returns whether quota exhaustion is enforced or shadowed.
    fn quota_mode(&self) -> QuotaMode;
}

/// A deterministic digest of a policy's identity, scope, and configuration.
///
/// Storage backends include this value in counter keys. Consequently, changing
/// any storage-relevant policy configuration starts an independent counter
/// instead of reinterpreting existing state.
///
/// The built-in policies' exact derivations are persistent cross-replica
/// protocols pinned by golden-vector tests. Their domains, field order,
/// separators, and integer encodings must remain stable through rolling
/// deployments.
///
/// This storage-key component deliberately does not implement Serde traits.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PolicyFingerprint([u8; 32]);

impl PolicyFingerprint {
    /// Constructs a fingerprint from an already domain-separated digest.
    ///
    /// This is intended for third-party [`RateLimitPolicy`] implementations.
    /// The digest must cover the algorithm identity and every
    /// storage-relevant policy field. Deployment-only fields such as
    /// [`QuotaMode`] should be excluded so a mode change reuses warmed state.
    ///
    /// Built-in policies derive their fingerprints automatically.
    pub const fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    /// Returns the 32-byte SHA-256 fingerprint.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Consumes the value and returns the 32-byte SHA-256 fingerprint.
    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for PolicyFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PolicyFingerprint(")?;
        write_hex(formatter, &self.0)?;
        formatter.write_str(")")
    }
}

impl fmt::Display for PolicyFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(formatter, &self.0)
    }
}

fn write_hex(formatter: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for byte in bytes {
        write!(formatter, "{byte:02x}")?;
    }
    Ok(())
}

/// An anchored fixed-window rate-limit policy.
///
/// A backend starts a window on the first allowed check for a storage key.
/// Later allowed checks use that anchor until the full window has elapsed.
/// This differs from fixed wall-clock boundaries such as calendar minutes.
///
/// Windows have exact whole-millisecond precision. A policy owns its
/// application-defined identifier and scope so it can be reused by checks.
///
/// With the `serde` feature, the wire object contains `id`, `scope`, `limit`,
/// `window_millis`, and `quota_mode`. The derived fingerprint is deliberately
/// omitted and recomputed through [`FixedWindowPolicy::new`] when
/// deserializing. An omitted `quota_mode` defaults to enforcement.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FixedWindowPolicy {
    id: PolicyId,
    scope: ScopeId,
    limit: Capacity,
    window: QuotaPeriod,
    fingerprint: PolicyFingerprint,
    quota_mode: QuotaMode,
}

impl FixedWindowPolicy {
    /// Validates and constructs an anchored fixed-window policy.
    ///
    /// # Errors
    ///
    /// Returns an error if `limit` or `window` is zero, if `limit` exceeds
    /// [`MAX_LIMIT`], if the window is not an exact whole number of
    /// milliseconds, or if it exceeds [`MAX_WINDOW`].
    pub fn new(
        id: PolicyId,
        scope: ScopeId,
        limit: u64,
        window: Duration,
    ) -> Result<Self, PolicyError> {
        let limit = Capacity::new(limit).map_err(|error| match error {
            CapacityError::Zero => PolicyError::ZeroLimit,
            CapacityError::TooLarge { actual, maximum } => {
                PolicyError::LimitTooLarge { actual, maximum }
            }
        })?;
        let window = QuotaPeriod::new(window).map_err(|error| match error {
            QuotaPeriodError::Zero => PolicyError::ZeroWindow,
            QuotaPeriodError::NotWholeMilliseconds => PolicyError::WindowNotWholeMilliseconds,
            QuotaPeriodError::TooLarge { actual, maximum } => {
                PolicyError::WindowTooLarge { actual, maximum }
            }
        })?;
        let fingerprint = fingerprint(&id, &scope, limit, window);

        Ok(Self {
            id,
            scope,
            limit,
            window,
            fingerprint,
            quota_mode: QuotaMode::Enforce,
        })
    }

    /// Returns this policy with the requested quota deployment mode.
    ///
    /// The policy fingerprint is unchanged because the mode is not
    /// storage-relevant.
    #[must_use]
    pub const fn with_quota_mode(mut self, quota_mode: QuotaMode) -> Self {
        self.quota_mode = quota_mode;
        self
    }

    /// Returns the application-defined policy identifier.
    pub const fn id(&self) -> &PolicyId {
        &self.id
    }

    /// Returns the application-defined policy scope.
    pub const fn scope(&self) -> &ScopeId {
        &self.scope
    }

    /// Returns the maximum cost allowed during one window.
    pub const fn limit(&self) -> Capacity {
        self.limit
    }

    /// Returns the anchored window duration.
    pub const fn window(&self) -> QuotaPeriod {
        self.window
    }

    /// Returns the deterministic configuration fingerprint.
    pub const fn fingerprint(&self) -> PolicyFingerprint {
        self.fingerprint
    }

    /// Returns whether quota exhaustion is enforced or shadowed.
    pub const fn quota_mode(&self) -> QuotaMode {
        self.quota_mode
    }
}

impl RateLimitPolicy for FixedWindowPolicy {
    fn id(&self) -> &PolicyId {
        self.id()
    }

    fn scope(&self) -> &ScopeId {
        self.scope()
    }

    fn quota(&self) -> Capacity {
        self.limit()
    }

    fn quota_period(&self) -> QuotaPeriod {
        self.window()
    }

    fn capacity(&self) -> Capacity {
        self.limit()
    }

    fn fingerprint(&self) -> PolicyFingerprint {
        self.fingerprint()
    }

    fn quota_mode(&self) -> QuotaMode {
        self.quota_mode()
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Serialize)]
struct FixedWindowPolicyRef<'a> {
    id: &'a PolicyId,
    scope: &'a ScopeId,
    limit: u64,
    window_millis: u64,
    quota_mode: QuotaMode,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FixedWindowPolicyWire {
    id: PolicyId,
    scope: ScopeId,
    limit: u64,
    window_millis: u64,
    #[serde(default)]
    quota_mode: QuotaMode,
}

#[cfg(feature = "serde")]
impl serde::Serialize for FixedWindowPolicy {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serde::Serialize::serialize(
            &FixedWindowPolicyRef {
                id: self.id(),
                scope: self.scope(),
                limit: self.limit().get(),
                window_millis: self.window().millis(),
                quota_mode: self.quota_mode(),
            },
            serializer,
        )
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for FixedWindowPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = <FixedWindowPolicyWire as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(
            wire.id,
            wire.scope,
            wire.limit,
            Duration::from_millis(wire.window_millis),
        )
        .map(|policy| policy.with_quota_mode(wire.quota_mode))
        .map_err(serde::de::Error::custom)
    }
}

fn fingerprint(
    id: &PolicyId,
    scope: &ScopeId,
    limit: Capacity,
    window: QuotaPeriod,
) -> PolicyFingerprint {
    // v1 = domain || id || NUL || scope || NUL || limit_u64_be || window_ms_u64_be
    let mut digest = Sha256::new();
    digest.update(FIXED_WINDOW_FINGERPRINT_DOMAIN);
    digest.update(id.as_str().as_bytes());
    digest.update([0]);
    digest.update(scope.as_str().as_bytes());
    digest.update([0]);
    digest.update(limit.get().to_be_bytes());
    digest.update(window.millis().to_be_bytes());
    PolicyFingerprint(digest.finalize().into())
}

/// A generic-cell-rate-algorithm policy.
///
/// `quota` units are replenished uniformly during `period`, while
/// `burst_capacity` controls the maximum immediately available allowance. This
/// avoids fixed-window boundary bursts while retaining constant-size state per
/// storage key.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GcraPolicy {
    id: PolicyId,
    scope: ScopeId,
    quota: Capacity,
    period: QuotaPeriod,
    burst_capacity: Capacity,
    fingerprint: PolicyFingerprint,
    quota_mode: QuotaMode,
}

impl GcraPolicy {
    /// Validates and constructs a GCRA policy.
    ///
    /// # Errors
    ///
    /// Returns an error when the quota or burst capacity is zero or exceeds
    /// [`MAX_LIMIT`], or when the period is not a supported exact
    /// whole-millisecond duration.
    pub fn new(
        id: PolicyId,
        scope: ScopeId,
        quota: u64,
        period: Duration,
        burst_capacity: u64,
    ) -> Result<Self, GcraPolicyError> {
        let quota = Capacity::new(quota).map_err(|error| match error {
            CapacityError::Zero => GcraPolicyError::ZeroQuota,
            CapacityError::TooLarge { actual, maximum } => {
                GcraPolicyError::QuotaTooLarge { actual, maximum }
            }
        })?;
        let burst_capacity = Capacity::new(burst_capacity).map_err(|error| match error {
            CapacityError::Zero => GcraPolicyError::ZeroBurstCapacity,
            CapacityError::TooLarge { actual, maximum } => {
                GcraPolicyError::BurstCapacityTooLarge { actual, maximum }
            }
        })?;
        let period = QuotaPeriod::new(period).map_err(|error| match error {
            QuotaPeriodError::Zero => GcraPolicyError::ZeroPeriod,
            QuotaPeriodError::NotWholeMilliseconds => GcraPolicyError::PeriodNotWholeMilliseconds,
            QuotaPeriodError::TooLarge { actual, maximum } => {
                GcraPolicyError::PeriodTooLarge { actual, maximum }
            }
        })?;
        let full_refill_millis = div_ceil_u128(
            u128::from(burst_capacity.get()) * u128::from(period.millis()),
            u128::from(quota.get()),
        );
        if full_refill_millis > u128::from(MAX_WINDOW_MILLIS) {
            return Err(GcraPolicyError::RefillDurationTooLarge {
                actual_millis: full_refill_millis,
                maximum_millis: MAX_WINDOW_MILLIS,
            });
        }
        let fingerprint = gcra_fingerprint(&id, &scope, quota, period, burst_capacity);

        Ok(Self {
            id,
            scope,
            quota,
            period,
            burst_capacity,
            fingerprint,
            quota_mode: QuotaMode::Enforce,
        })
    }

    /// Returns this policy with the requested quota deployment mode.
    #[must_use]
    pub const fn with_quota_mode(mut self, quota_mode: QuotaMode) -> Self {
        self.quota_mode = quota_mode;
        self
    }

    /// Returns the application-defined policy identifier.
    pub const fn id(&self) -> &PolicyId {
        &self.id
    }

    /// Returns the application-defined policy scope.
    pub const fn scope(&self) -> &ScopeId {
        &self.scope
    }

    /// Returns the number of units replenished during one period.
    pub const fn quota(&self) -> Capacity {
        self.quota
    }

    /// Returns the replenishment period.
    pub const fn period(&self) -> QuotaPeriod {
        self.period
    }

    /// Returns the maximum immediately available allowance.
    pub const fn burst_capacity(&self) -> Capacity {
        self.burst_capacity
    }

    /// Returns the deterministic configuration fingerprint.
    pub const fn fingerprint(&self) -> PolicyFingerprint {
        self.fingerprint
    }

    /// Returns whether quota exhaustion is enforced or shadowed.
    pub const fn quota_mode(&self) -> QuotaMode {
        self.quota_mode
    }
}

impl RateLimitPolicy for GcraPolicy {
    fn id(&self) -> &PolicyId {
        self.id()
    }

    fn scope(&self) -> &ScopeId {
        self.scope()
    }

    fn quota(&self) -> Capacity {
        self.quota()
    }

    fn quota_period(&self) -> QuotaPeriod {
        self.period()
    }

    fn capacity(&self) -> Capacity {
        self.burst_capacity()
    }

    fn fingerprint(&self) -> PolicyFingerprint {
        self.fingerprint()
    }

    fn quota_mode(&self) -> QuotaMode {
        self.quota_mode()
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Serialize)]
struct GcraPolicyRef<'a> {
    id: &'a PolicyId,
    scope: &'a ScopeId,
    quota: u64,
    period_millis: u64,
    burst_capacity: u64,
    quota_mode: QuotaMode,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GcraPolicyWire {
    id: PolicyId,
    scope: ScopeId,
    quota: u64,
    period_millis: u64,
    burst_capacity: u64,
    #[serde(default)]
    quota_mode: QuotaMode,
}

#[cfg(feature = "serde")]
impl serde::Serialize for GcraPolicy {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serde::Serialize::serialize(
            &GcraPolicyRef {
                id: self.id(),
                scope: self.scope(),
                quota: self.quota().get(),
                period_millis: self.period().millis(),
                burst_capacity: self.burst_capacity().get(),
                quota_mode: self.quota_mode(),
            },
            serializer,
        )
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for GcraPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = <GcraPolicyWire as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(
            wire.id,
            wire.scope,
            wire.quota,
            Duration::from_millis(wire.period_millis),
            wire.burst_capacity,
        )
        .map(|policy| policy.with_quota_mode(wire.quota_mode))
        .map_err(serde::de::Error::custom)
    }
}

fn gcra_fingerprint(
    id: &PolicyId,
    scope: &ScopeId,
    quota: Capacity,
    period: QuotaPeriod,
    burst_capacity: Capacity,
) -> PolicyFingerprint {
    // v1 = domain || id || NUL || scope || NUL || quota_u64_be
    //      || period_ms_u64_be || burst_capacity_u64_be
    let mut digest = Sha256::new();
    digest.update(GCRA_FINGERPRINT_DOMAIN);
    digest.update(id.as_str().as_bytes());
    digest.update([0]);
    digest.update(scope.as_str().as_bytes());
    digest.update([0]);
    digest.update(quota.get().to_be_bytes());
    digest.update(period.millis().to_be_bytes());
    digest.update(burst_capacity.get().to_be_bytes());
    PolicyFingerprint(digest.finalize().into())
}

const fn div_ceil_u128(numerator: u128, denominator: u128) -> u128 {
    numerator / denominator
        + if numerator.is_multiple_of(denominator) {
            0
        } else {
            1
        }
}

/// An invalid fixed-window policy configuration.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum PolicyError {
    /// The configured limit was zero.
    #[error("fixed-window limit must be greater than zero")]
    ZeroLimit,
    /// The configured limit exceeded the portable backend maximum.
    #[error("fixed-window limit {actual} exceeds portable maximum {maximum}")]
    LimitTooLarge {
        /// Supplied limit.
        actual: u64,
        /// Largest limit supported by every backend.
        maximum: u64,
    },
    /// The configured window was zero.
    #[error("fixed-window duration must be greater than zero")]
    ZeroWindow,
    /// The configured window had finer precision than a whole millisecond.
    #[error("fixed-window duration must be an exact whole number of milliseconds")]
    WindowNotWholeMilliseconds,
    /// The configured window exceeded the portable backend maximum.
    #[error("fixed-window duration {actual:?} exceeds portable maximum {maximum:?}")]
    WindowTooLarge {
        /// Supplied window.
        actual: Duration,
        /// Largest window supported by every backend.
        maximum: Duration,
    },
}

/// An invalid GCRA policy configuration.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum GcraPolicyError {
    /// The replenishment quota was zero.
    #[error("GCRA quota must be greater than zero")]
    ZeroQuota,
    /// The replenishment quota exceeded the portable backend maximum.
    #[error("GCRA quota {actual} exceeds portable maximum {maximum}")]
    QuotaTooLarge {
        /// Supplied quota.
        actual: u64,
        /// Largest quota supported by every backend.
        maximum: u64,
    },
    /// The burst capacity was zero.
    #[error("GCRA burst capacity must be greater than zero")]
    ZeroBurstCapacity,
    /// The burst capacity exceeded the portable backend maximum.
    #[error("GCRA burst capacity {actual} exceeds portable maximum {maximum}")]
    BurstCapacityTooLarge {
        /// Supplied burst capacity.
        actual: u64,
        /// Largest burst capacity supported by every backend.
        maximum: u64,
    },
    /// The replenishment period was zero.
    #[error("GCRA period must be greater than zero")]
    ZeroPeriod,
    /// The replenishment period had finer precision than a millisecond.
    #[error("GCRA period must be an exact whole number of milliseconds")]
    PeriodNotWholeMilliseconds,
    /// The replenishment period exceeded the portable backend maximum.
    #[error("GCRA period {actual:?} exceeds portable maximum {maximum:?}")]
    PeriodTooLarge {
        /// Supplied period.
        actual: Duration,
        /// Largest supported period.
        maximum: Duration,
    },
    /// Filling the complete burst would take longer than the portable maximum.
    #[error(
        "GCRA full-refill duration {actual_millis}ms exceeds portable maximum {maximum_millis}ms"
    )]
    RefillDurationTooLarge {
        /// Computed full-refill duration in milliseconds.
        actual_millis: u128,
        /// Largest supported full-refill duration in milliseconds.
        maximum_millis: u64,
    },
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        Capacity, CapacityError, FixedWindowPolicy, GcraPolicy, GcraPolicyError, MAX_LIMIT,
        MAX_WINDOW, MAX_WINDOW_MILLIS, PolicyError, QuotaMode, QuotaPeriod, QuotaPeriodError,
        RateLimitPolicy,
    };
    use crate::{PolicyId, ScopeId};

    fn policy(limit: u64, window: Duration) -> Result<FixedWindowPolicy, PolicyError> {
        FixedWindowPolicy::new(
            PolicyId::new("auth.login").unwrap(),
            ScopeId::new("client").unwrap(),
            limit,
            window,
        )
    }

    #[test]
    fn capacity_is_validated_once() {
        assert_eq!(Capacity::new(0), Err(CapacityError::Zero));
        assert_eq!(
            Capacity::new(MAX_LIMIT + 1),
            Err(CapacityError::TooLarge {
                actual: MAX_LIMIT + 1,
                maximum: MAX_LIMIT,
            })
        );
        assert_eq!(Capacity::new(MAX_LIMIT), Ok(Capacity::MAX));
        assert_eq!(Capacity::new(8).unwrap().get(), 8);
        assert_eq!(u64::from(Capacity::new(8).unwrap()), 8);
        assert_eq!(Capacity::new(8).unwrap().to_string(), "8");
    }

    #[test]
    fn quota_period_is_validated_once() {
        assert_eq!(
            QuotaPeriod::new(Duration::ZERO),
            Err(QuotaPeriodError::Zero)
        );
        assert_eq!(
            QuotaPeriod::new(Duration::from_micros(1_500)),
            Err(QuotaPeriodError::NotWholeMilliseconds)
        );
        assert_eq!(
            QuotaPeriod::new(MAX_WINDOW + Duration::from_millis(1)),
            Err(QuotaPeriodError::TooLarge {
                actual: MAX_WINDOW + Duration::from_millis(1),
                maximum: MAX_WINDOW,
            })
        );
        assert_eq!(QuotaPeriod::new(MAX_WINDOW), Ok(QuotaPeriod::MAX));
        let period = QuotaPeriod::new(Duration::from_millis(1_500)).unwrap();
        assert_eq!(period.millis(), 1_500);
        assert_eq!(period.duration(), Duration::from_millis(1_500));
        assert_eq!(Duration::from(period), Duration::from_millis(1_500));
    }

    #[test]
    fn accepts_nonzero_whole_millisecond_windows() {
        let policy = policy(8, Duration::from_millis(60_001)).unwrap();

        assert_eq!(policy.limit().get(), 8);
        assert_eq!(policy.window().duration(), Duration::from_millis(60_001));
        assert_eq!(policy.window().millis(), 60_001);
        assert_eq!(policy.id().as_str(), "auth.login");
        assert_eq!(policy.scope().as_str(), "client");
    }

    #[test]
    fn rejects_zero_limit_and_window() {
        assert_eq!(
            policy(0, Duration::from_secs(1)),
            Err(PolicyError::ZeroLimit)
        );
        assert_eq!(policy(1, Duration::ZERO), Err(PolicyError::ZeroWindow));
    }

    #[test]
    fn rejects_sub_millisecond_and_fractional_millisecond_windows() {
        assert_eq!(
            policy(1, Duration::from_nanos(1)),
            Err(PolicyError::WindowNotWholeMilliseconds)
        );
        assert_eq!(
            policy(1, Duration::from_micros(1_500)),
            Err(PolicyError::WindowNotWholeMilliseconds)
        );
    }

    #[test]
    fn accepts_portable_upper_bounds() {
        let policy = policy(MAX_LIMIT, MAX_WINDOW).unwrap();

        assert_eq!(policy.limit(), Capacity::MAX);
        assert_eq!(policy.window(), QuotaPeriod::MAX);
        assert_eq!(policy.window().millis(), MAX_WINDOW_MILLIS);
    }

    #[test]
    fn rejects_limit_above_portable_maximum() {
        assert_eq!(
            policy(MAX_LIMIT + 1, Duration::from_secs(1)),
            Err(PolicyError::LimitTooLarge {
                actual: MAX_LIMIT + 1,
                maximum: MAX_LIMIT,
            })
        );
    }

    #[test]
    fn rejects_window_above_portable_maximum() {
        let actual = MAX_WINDOW + Duration::from_millis(1);

        assert_eq!(
            policy(1, actual),
            Err(PolicyError::WindowTooLarge {
                actual,
                maximum: MAX_WINDOW,
            })
        );
    }

    #[test]
    fn rejects_windows_far_beyond_portable_maximum() {
        let actual = Duration::from_secs(u64::MAX);

        assert_eq!(
            policy(1, actual),
            Err(PolicyError::WindowTooLarge {
                actual,
                maximum: MAX_WINDOW,
            })
        );
    }

    #[test]
    fn fingerprint_is_deterministic() {
        let first = policy(8, Duration::from_mins(1)).unwrap();
        let second = policy(8, Duration::from_mins(1)).unwrap();

        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_eq!(first.fingerprint().as_bytes().len(), 32);
        assert_eq!(first.fingerprint().to_string().len(), 64);
    }

    #[test]
    fn fixed_window_fingerprint_matches_stable_protocol_vector() {
        let policy = policy(8, Duration::from_mins(1)).unwrap();

        assert_eq!(
            policy.fingerprint().to_string(),
            "b4c06fc2a76c7f9c49dabaf929ced6ed17e7d739a558174ca41c42cea25751d9"
        );
    }

    #[test]
    fn fingerprint_changes_with_every_storage_relevant_field() {
        let baseline = policy(8, Duration::from_mins(1)).unwrap();
        let different_limit = policy(9, Duration::from_mins(1)).unwrap();
        let different_window = policy(8, Duration::from_secs(61)).unwrap();
        let different_id = FixedWindowPolicy::new(
            PolicyId::new("auth.signup").unwrap(),
            ScopeId::new("client").unwrap(),
            8,
            Duration::from_mins(1),
        )
        .unwrap();
        let different_scope = FixedWindowPolicy::new(
            PolicyId::new("auth.login").unwrap(),
            ScopeId::new("identity").unwrap(),
            8,
            Duration::from_mins(1),
        )
        .unwrap();

        assert_ne!(baseline.fingerprint(), different_limit.fingerprint());
        assert_ne!(baseline.fingerprint(), different_window.fingerprint());
        assert_ne!(baseline.fingerprint(), different_id.fingerprint());
        assert_ne!(baseline.fingerprint(), different_scope.fingerprint());
    }

    #[test]
    fn quota_mode_does_not_change_fixed_window_storage_identity() {
        let enforced = policy(8, Duration::from_mins(1)).unwrap();
        let shadowed = enforced.clone().with_quota_mode(QuotaMode::Shadow);

        assert_eq!(enforced.fingerprint(), shadowed.fingerprint());
        assert_eq!(enforced.quota_mode(), QuotaMode::Enforce);
        assert_eq!(shadowed.quota_mode(), QuotaMode::Shadow);
        assert_eq!(RateLimitPolicy::capacity(&shadowed).get(), 8);
    }

    #[test]
    fn gcra_policy_exposes_uniform_replenishment_and_distinct_fingerprint() {
        let id = PolicyId::new("api.read").unwrap();
        let scope = ScopeId::new("account").unwrap();
        let gcra =
            GcraPolicy::new(id.clone(), scope.clone(), 10, Duration::from_secs(1), 20).unwrap();
        let fixed = FixedWindowPolicy::new(id, scope, 10, Duration::from_secs(1)).unwrap();

        assert_eq!(gcra.quota().get(), 10);
        assert_eq!(gcra.period().duration(), Duration::from_secs(1));
        assert_eq!(gcra.burst_capacity().get(), 20);
        assert_eq!(RateLimitPolicy::capacity(&gcra).get(), 20);
        assert_eq!(RateLimitPolicy::quota_period(&gcra), gcra.period());
        assert_ne!(gcra.fingerprint(), fixed.fingerprint());
        assert_eq!(
            gcra.fingerprint(),
            gcra.clone()
                .with_quota_mode(QuotaMode::Shadow)
                .fingerprint()
        );
    }

    #[test]
    fn gcra_fingerprint_tracks_every_storage_field_but_not_quota_mode() {
        let make = |id: &str, scope: &str, quota, period_millis, burst_capacity| {
            GcraPolicy::new(
                PolicyId::new(id).unwrap(),
                ScopeId::new(scope).unwrap(),
                quota,
                Duration::from_millis(period_millis),
                burst_capacity,
            )
            .unwrap()
        };
        let baseline = make("api.read", "account", 10, 1_000, 20);

        assert_eq!(
            baseline.fingerprint(),
            make("api.read", "account", 10, 1_000, 20).fingerprint()
        );
        assert_ne!(
            baseline.fingerprint(),
            make("api.write", "account", 10, 1_000, 20).fingerprint()
        );
        assert_ne!(
            baseline.fingerprint(),
            make("api.read", "client", 10, 1_000, 20).fingerprint()
        );
        assert_ne!(
            baseline.fingerprint(),
            make("api.read", "account", 11, 1_000, 20).fingerprint()
        );
        assert_ne!(
            baseline.fingerprint(),
            make("api.read", "account", 10, 1_001, 20).fingerprint()
        );
        assert_ne!(
            baseline.fingerprint(),
            make("api.read", "account", 10, 1_000, 21).fingerprint()
        );
        assert_eq!(
            baseline.fingerprint(),
            baseline
                .clone()
                .with_quota_mode(QuotaMode::Shadow)
                .fingerprint()
        );
    }

    #[test]
    fn gcra_fingerprint_matches_stable_protocol_vector() {
        let policy = GcraPolicy::new(
            PolicyId::new("api.read").unwrap(),
            ScopeId::new("account").unwrap(),
            10,
            Duration::from_secs(1),
            20,
        )
        .unwrap();

        assert_eq!(
            policy.fingerprint().to_string(),
            "c0aac2c2bbad1b6e7a626da8f103429e6863736dfe23bdd4b6c5853b80260d54"
        );
    }

    #[test]
    fn gcra_policy_rejects_invalid_portable_values() {
        let make = |quota, period, burst| {
            GcraPolicy::new(
                PolicyId::new("api.read").unwrap(),
                ScopeId::new("account").unwrap(),
                quota,
                period,
                burst,
            )
        };

        assert_eq!(
            make(0, Duration::from_secs(1), 1),
            Err(GcraPolicyError::ZeroQuota)
        );
        assert_eq!(
            make(1, Duration::from_secs(1), 0),
            Err(GcraPolicyError::ZeroBurstCapacity)
        );
        assert_eq!(
            make(1, Duration::from_nanos(1), 1),
            Err(GcraPolicyError::PeriodNotWholeMilliseconds)
        );
        assert_eq!(make(1, Duration::ZERO, 1), Err(GcraPolicyError::ZeroPeriod));
        let period_too_large = MAX_WINDOW + Duration::from_millis(1);
        assert_eq!(
            make(1, period_too_large, 1),
            Err(GcraPolicyError::PeriodTooLarge {
                actual: period_too_large,
                maximum: MAX_WINDOW,
            })
        );
        assert!(matches!(
            make(1, MAX_WINDOW, 2),
            Err(GcraPolicyError::RefillDurationTooLarge { .. })
        ));
    }

    #[test]
    fn policy_validation_preserves_precedence_for_multiple_invalid_fields() {
        assert_eq!(policy(0, Duration::ZERO), Err(PolicyError::ZeroLimit));

        let gcra = GcraPolicy::new(
            PolicyId::new("api.read").unwrap(),
            ScopeId::new("account").unwrap(),
            0,
            Duration::ZERO,
            0,
        );
        assert_eq!(gcra, Err(GcraPolicyError::ZeroQuota));
    }
}
