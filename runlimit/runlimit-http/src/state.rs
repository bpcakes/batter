//! The version-neutral input of every `RateLimit` encoder.

use runlimit_core::{Admitted, AdmittedView, Allowance, QuotaDenial};

/// The quota state a `RateLimit` field can describe.
///
/// Only an allowance or a quota denial has service-limit metadata. A
/// storage-capacity denial is not representable here, so it cannot reach
/// [`draft_12::service_limit`](crate::draft_12::service_limit) or
/// [`draft_11::service_limit`](crate::draft_11::service_limit) and become an
/// encoding error in the response path. Callers holding a
/// [`runlimit_core::Denial`] match it and pass the [`QuotaDenial`] from its
/// `QuotaExceeded` arm. An [`Admitted`] outcome converts directly: an
/// allowance becomes [`QuotaState::Available`] and a shadow denial becomes
/// [`QuotaState::Exhausted`], so a shadow denial exposes the service value
/// that would have applied if the policy were enforced.
///
/// Every draft module reads the same two values from a state: the available
/// quota, which is zero when exhausted, and the effective window, which is
/// the allowance's replenishment delay or the denial's retry delay rounded up
/// to whole seconds by [`runlimit_core::Delay::seconds`].
///
/// Migration: this replaces `runlimit_http::draft_11::QuotaState`; its
/// variants and conversions are unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaState {
    /// Quota was consumed and the allowance remains available.
    Available(Allowance),
    /// The requested cost exceeded the quota, enforced or shadowed.
    Exhausted(QuotaDenial),
}

impl From<Allowance> for QuotaState {
    fn from(allowance: Allowance) -> Self {
        Self::Available(allowance)
    }
}

impl From<QuotaDenial> for QuotaState {
    fn from(denial: QuotaDenial) -> Self {
        Self::Exhausted(denial)
    }
}

impl From<Admitted> for QuotaState {
    fn from(admitted: Admitted) -> Self {
        match admitted.view() {
            AdmittedView::Allowed { allowance } => Self::Available(allowance),
            AdmittedView::ShadowDenied { denial } => Self::Exhausted(denial),
        }
    }
}

/// The two service-limit values every draft encodes, before range checking.
pub(crate) struct ServiceLimit {
    /// Quota units still available; zero for an exhausted quota.
    pub(crate) available: u64,
    /// Whole seconds within which no more than `available` can be used,
    /// rounded up.
    pub(crate) effective_window: u64,
}

impl QuotaState {
    /// Derives the service-limit values. This is the one mapping from a quota
    /// state to `RateLimit` values; draft modules differ only in the keys they
    /// spell them with.
    pub(crate) const fn service_limit(self) -> ServiceLimit {
        match self {
            Self::Available(allowance) => ServiceLimit {
                available: allowance.available(),
                effective_window: allowance.replenishes_after().seconds(),
            },
            Self::Exhausted(denial) => ServiceLimit {
                available: 0,
                effective_window: denial.retry_after().seconds(),
            },
        }
    }
}
