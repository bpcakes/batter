use std::{num::NonZeroUsize, time::Duration};

use super::{
    Admitted, AdmittedView, Allowance, AvailableExceedsCapacity, BatchDecision, BatchDecisionView,
    Decision, DecisionView, Delay, Denial, DeniedIndexOutOfRange, EmptyAllowedBatch, QuotaDenial,
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
    assert_eq!(BatchDecision::allowed(Vec::new()), Err(EmptyAllowedBatch));
}

#[test]
fn batch_denial_index_must_be_below_the_batch_size() {
    let denial = quota(8, Duration::from_mins(1));

    assert_eq!(
        BatchDecision::denied(1, 1, denial),
        Err(DeniedIndexOutOfRange {
            index: 1,
            batch_size: 1,
        })
    );
    assert_eq!(
        BatchDecision::denied(0, 0, denial),
        Err(DeniedIndexOutOfRange {
            index: 0,
            batch_size: 0,
        })
    );
    assert_eq!(
        BatchDecision::shadow_denied(3, 2, denial),
        Err(DeniedIndexOutOfRange {
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
        Err(AvailableExceedsCapacity {
            capacity: capacity(8),
            available: 9,
        })
    );
    assert!(Allowance::new(capacity(8), 8, Duration::ZERO).is_ok());
}
