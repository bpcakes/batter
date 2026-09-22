//! Compile and behavior check for the README decision-model example.

use std::time::Duration;

use runlimit_core::{Allowance, Capacity, Decision, DecisionView, Delay, Denial, QuotaDenial};

fn describe(decision: &Decision) -> String {
    match decision.view() {
        DecisionView::Allowed { allowance } => format!(
            "admitted; {} of {} left",
            allowance.available(),
            allowance.capacity(),
        ),
        DecisionView::ShadowDenied { denial } => format!(
            "admitted; quota of {} would have denied for {}s",
            denial.capacity(),
            denial.retry_after().seconds(),
        ),
        DecisionView::Denied {
            denial: Denial::QuotaExceeded(quota),
        } => format!("rejected; retry after {}s", quota.retry_after().seconds()),
        DecisionView::Denied {
            denial: Denial::StorageCapacity { retry_after },
        } => match retry_after {
            Some(retry_after) => format!("rejected; backend full for {}s", retry_after.seconds()),
            None => "rejected; backend full".to_owned(),
        },
    }
}

#[test]
fn every_outcome_is_described_without_a_catch_all() {
    let capacity = Capacity::new(8).unwrap();
    let quota = QuotaDenial::new(capacity, Duration::from_millis(30_001));
    assert_eq!(
        describe(&Decision::allowed(
            Allowance::new(capacity, 7, Duration::from_secs(60)).unwrap()
        )),
        "admitted; 7 of 8 left"
    );
    assert_eq!(
        describe(&Decision::shadow_denied(quota)),
        "admitted; quota of 8 would have denied for 31s"
    );
    assert_eq!(
        describe(&Decision::denied(quota)),
        "rejected; retry after 31s"
    );
    assert_eq!(
        describe(&Decision::denied(Denial::StorageCapacity {
            retry_after: Some(Delay::new(Duration::from_millis(1))),
        })),
        "rejected; backend full for 1s"
    );
    assert_eq!(
        describe(&Decision::denied(Denial::StorageCapacity {
            retry_after: None
        })),
        "rejected; backend full"
    );
}
