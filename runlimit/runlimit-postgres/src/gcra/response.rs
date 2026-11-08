use std::time::Duration;

use runlimit_core::{Allowance, Denial, QuotaMode};

use super::{Admission, Pending};

impl Pending {
    /// Finalizes response metadata without changing the evaluated transition.
    pub(super) fn finish(self, elapsed: Duration) -> Admission {
        match self {
            Self::Allowed(allowances) => Admission::Allowed(
                allowances
                    .into_iter()
                    .map(|item| {
                        let allowance = item.allowance;
                        Allowance::new(
                            allowance.capacity(),
                            allowance.available(),
                            allowance
                                .replenishes_after()
                                .duration()
                                .saturating_sub(elapsed),
                        )
                        .expect("GCRA evaluation validated capacity and availability")
                    })
                    .collect(),
            ),
            Self::Denied {
                index,
                denial,
                mode,
            } => match denial.finish(elapsed) {
                Denial::QuotaExceeded(quota) if mode == QuotaMode::Shadow => {
                    Admission::ShadowDenied {
                        index,
                        denial: quota,
                    }
                }
                denial => Admission::Denied { index, denial },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use runlimit_core::{Capacity, QuotaDenial, gcra};
    use sqlx::types::chrono::DateTime;

    use super::*;
    use crate::admission::{PendingDenial, PendingQuotaDenial, authoritative_elapsed};

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
            let capacity = Capacity::new(10).unwrap();
            // Persisted logical time is deliberately ahead of the database sample.
            let pending = gcra::PendingAllowance {
                tat_scaled: 36_002_500,
                expires_at_millis: 3_600_250,
                allowance: Allowance::new(capacity, 4, Duration::from_millis(250)).unwrap(),
            };
            let Admission::Allowed(allowances) = Pending::Allowed(vec![pending]).finish(elapsed)
            else {
                panic!("expected allowance");
            };
            assert_eq!(
                allowances,
                [Allowance::new(
                    capacity,
                    4,
                    Duration::from_millis(250).saturating_sub(elapsed)
                )
                .unwrap()]
            );
            let quota = || {
                PendingDenial::Quota(PendingQuotaDenial {
                    limit: capacity,
                    retry_from_sample: Duration::from_millis(150),
                })
            };
            let expected =
                QuotaDenial::new(capacity, Duration::from_millis(150).saturating_sub(elapsed));
            let Admission::Denied { index, denial } = Pending::Denied {
                index: 2,
                denial: quota(),
                mode: QuotaMode::Enforce,
            }
            .finish(elapsed) else {
                panic!("expected denial")
            };
            assert_eq!(index, 2);
            assert_eq!(denial, Denial::QuotaExceeded(expected));
            let Admission::ShadowDenied { index, denial } = Pending::Denied {
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
    fn storage_capacity_denials_have_no_quota_delay() {
        let Admission::Denied { index, denial } = Pending::Denied {
            index: 3,
            denial: PendingDenial::StorageCapacity,
            mode: QuotaMode::Shadow,
        }
        .finish(Duration::from_secs(1)) else {
            panic!("expected storage denial")
        };
        assert_eq!(index, 3);
        assert_eq!(denial, Denial::StorageCapacity { retry_after: None });
    }
}
