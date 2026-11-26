//! Response metadata measured from the evaluation sample, and the one place
//! both quota limiters re-measure it from their path's last database response.

use std::time::Duration;

use runlimit_core::{Allowance, Capacity, Denial, QuotaDenial, QuotaMode, gcra};
use sqlx::types::chrono::{DateTime, Utc};

use crate::{CheckError, admission::Admission};

pub(crate) const STORED_USAGE_EXCEEDS_POLICY_LIMIT: &str = "stored usage exceeds its policy limit";

/// Database-clock time between the evaluation sample and a later response,
/// saturating at zero when the clock stepped backwards.
pub(crate) fn authoritative_elapsed(start: DateTime<Utc>, end: DateTime<Utc>) -> Duration {
    end.signed_duration_since(start)
        .to_std()
        .unwrap_or(Duration::ZERO)
}

/// Allowance metadata as evaluated at the database sample.
pub(crate) trait SampledAllowance {
    fn sampled(&self) -> Allowance;
}

/// A fixed-window allowance whose stored usage was validated against its
/// policy limit.
#[derive(Debug)]
pub(crate) struct PendingAllowance {
    allowance: Allowance,
}

impl PendingAllowance {
    pub(crate) fn new(
        limit: Capacity,
        used: u64,
        reset_from_sample: Duration,
    ) -> Result<Self, CheckError> {
        let remaining = limit
            .get()
            .checked_sub(used)
            .ok_or(CheckError::storage_invariant(
                STORED_USAGE_EXCEEDS_POLICY_LIMIT,
            ))?;
        let allowance = Allowance::new(limit, remaining, reset_from_sample)
            .map_err(|_| CheckError::storage_invariant(STORED_USAGE_EXCEEDS_POLICY_LIMIT))?;
        Ok(Self { allowance })
    }
}

impl SampledAllowance for PendingAllowance {
    fn sampled(&self) -> Allowance {
        self.allowance
    }
}

impl SampledAllowance for gcra::PendingAllowance {
    fn sampled(&self) -> Allowance {
        self.allowance
    }
}

#[derive(Debug)]
pub(crate) struct PendingQuotaDenial {
    pub(crate) limit: Capacity,
    pub(crate) retry_from_sample: Duration,
}

impl PendingQuotaDenial {
    fn finish(self, authoritative_elapsed: Duration) -> QuotaDenial {
        QuotaDenial::new(
            self.limit,
            self.retry_from_sample.saturating_sub(authoritative_elapsed),
        )
    }
}

#[derive(Debug)]
pub(crate) enum PendingDenial {
    Quota(PendingQuotaDenial),
    StorageCapacity,
}

/// An evaluated transaction outcome whose delays still count from the
/// evaluation sample.
///
/// `A` is the backend's evaluated allowance, kept whole because a backend may
/// still have to persist it. A denial names the first denied check in caller
/// order and the quota mode of that check's policy.
#[derive(Debug)]
pub(crate) enum Pending<A> {
    Allowed(Vec<A>),
    Denied {
        index: usize,
        denial: PendingDenial,
        mode: QuotaMode,
    },
}

impl<A: SampledAllowance> Pending<A> {
    /// Finalizes response metadata without changing the evaluated transition:
    /// every delay loses the authoritative time that passed since the sample,
    /// saturating at zero.
    ///
    /// Only a quota denial can be shadowed. A storage-capacity denial is
    /// always enforced and carries no quota delay.
    pub(crate) fn finish(self, authoritative_elapsed: Duration) -> Admission {
        match self {
            Self::Allowed(allowances) => Admission::Allowed(
                // Consuming the evaluated allowances lets a fixed-window
                // response reuse their buffer instead of allocating another.
                allowances
                    .into_iter()
                    .map(|allowance| {
                        let sampled = allowance.sampled();
                        Allowance::new(
                            sampled.capacity(),
                            sampled.available(),
                            sampled
                                .replenishes_after()
                                .duration()
                                .saturating_sub(authoritative_elapsed),
                        )
                        .expect("a sampled allowance already validated its availability")
                    })
                    .collect(),
            ),
            Self::Denied {
                index,
                denial,
                mode,
            } => match (denial, mode) {
                (PendingDenial::Quota(quota), QuotaMode::Shadow) => Admission::ShadowDenied {
                    index,
                    denial: quota.finish(authoritative_elapsed),
                },
                (PendingDenial::Quota(quota), QuotaMode::Enforce) => Admission::Denied {
                    index,
                    denial: Denial::QuotaExceeded(quota.finish(authoritative_elapsed)),
                },
                (PendingDenial::StorageCapacity, QuotaMode::Enforce | QuotaMode::Shadow) => {
                    Admission::Denied {
                        index,
                        denial: Denial::StorageCapacity { retry_after: None },
                    }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capacity() -> Capacity {
        Capacity::new(10).unwrap()
    }

    fn quota() -> PendingDenial {
        PendingDenial::Quota(PendingQuotaDenial {
            limit: capacity(),
            retry_from_sample: Duration::from_millis(150),
        })
    }

    #[test]
    fn stable_storage_invariant_classification_is_pinned_without_a_database() {
        assert_eq!(
            STORED_USAGE_EXCEEDS_POLICY_LIMIT,
            "stored usage exceeds its policy limit"
        );
    }

    #[test]
    fn response_metadata_subtracts_only_authoritative_elapsed_time() {
        let sample = DateTime::from_timestamp_millis(1_000).unwrap();
        for (response_delta, elapsed) in [(80, 80), (300, 300), (-80, 0)] {
            let elapsed = {
                let actual = authoritative_elapsed(
                    sample,
                    DateTime::from_timestamp_millis(1_000 + response_delta).unwrap(),
                );
                assert_eq!(actual, Duration::from_millis(elapsed));
                actual
            };
            let expected_allowance = Allowance::new(
                capacity(),
                4,
                Duration::from_millis(250).saturating_sub(elapsed),
            )
            .unwrap();
            // Persisted logical time is deliberately ahead of the database sample.
            let gcra = gcra::PendingAllowance {
                tat_scaled: 36_002_500,
                expires_at_millis: 3_600_250,
                allowance: Allowance::new(capacity(), 4, Duration::from_millis(250)).unwrap(),
            };
            let Admission::Allowed(allowances) = Pending::Allowed(vec![gcra]).finish(elapsed)
            else {
                panic!("expected allowance");
            };
            assert_eq!(allowances, [expected_allowance]);
            let fixed_window =
                PendingAllowance::new(capacity(), 6, Duration::from_millis(250)).unwrap();
            let Admission::Allowed(allowances) =
                Pending::Allowed(vec![fixed_window]).finish(elapsed)
            else {
                panic!("expected allowance");
            };
            assert_eq!(allowances, [expected_allowance]);

            let expected = QuotaDenial::new(
                capacity(),
                Duration::from_millis(150).saturating_sub(elapsed),
            );
            let Admission::Denied { index, denial } = Pending::<PendingAllowance>::Denied {
                index: 2,
                denial: quota(),
                mode: QuotaMode::Enforce,
            }
            .finish(elapsed) else {
                panic!("expected denial")
            };
            assert_eq!(index, 2);
            assert_eq!(denial, Denial::QuotaExceeded(expected));
            let Admission::ShadowDenied { index, denial } = Pending::<PendingAllowance>::Denied {
                index: 1,
                denial: quota(),
                mode: QuotaMode::Shadow,
            }
            .finish(elapsed) else {
                panic!("expected shadow denial")
            };
            assert_eq!(index, 1);
            assert_eq!(denial, expected);
        }
    }

    #[test]
    fn storage_capacity_denials_are_enforced_in_every_mode_without_a_quota_delay() {
        for mode in [QuotaMode::Enforce, QuotaMode::Shadow] {
            let Admission::Denied { index, denial } = Pending::<PendingAllowance>::Denied {
                index: 3,
                denial: PendingDenial::StorageCapacity,
                mode,
            }
            .finish(Duration::from_secs(1)) else {
                panic!("expected storage denial")
            };
            assert_eq!(index, 3);
            assert_eq!(denial, Denial::StorageCapacity { retry_after: None });
        }
    }
}
