//! Serde wire-contract and validation tests.

#![cfg(feature = "serde")]

use std::time::Duration;

use runlimit_core::{
    Allowance, BatchDecision, BatchDecisionView, Capacity, Decision, DecisionError, Delay, Denial,
    FixedWindowPolicy, GcraPolicy, MAX_WINDOW_MILLIS, PolicyId, QuotaDenial, QuotaMode, ScopeId,
};
use serde_json::json;

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
fn identifiers_use_validated_string_values() {
    let policy = PolicyId::new("auth/login:v2").unwrap();
    let scope = ScopeId::new("client-ip_64").unwrap();

    assert_eq!(
        serde_json::to_value(&policy).unwrap(),
        json!("auth/login:v2")
    );
    assert_eq!(
        serde_json::from_value::<PolicyId>(json!("auth/login:v2")).unwrap(),
        policy
    );
    assert_eq!(
        serde_json::from_value::<ScopeId>(json!("client-ip_64")).unwrap(),
        scope
    );
    assert!(serde_json::from_value::<PolicyId>(json!("auth login")).is_err());
    assert!(serde_json::from_value::<ScopeId>(json!("")).is_err());
}

#[test]
fn policy_wire_shape_recomputes_its_fingerprint() {
    let policy = FixedWindowPolicy::new(
        PolicyId::new("auth.login").unwrap(),
        ScopeId::new("identity").unwrap(),
        8,
        Duration::from_millis(60_001),
    )
    .unwrap();

    let value = serde_json::to_value(&policy).unwrap();
    assert_eq!(
        value,
        json!({
            "id": "auth.login",
            "scope": "identity",
            "limit": 8,
            "window_millis": 60_001,
            "quota_mode": "enforce"
        })
    );

    let decoded: FixedWindowPolicy = serde_json::from_value(value).unwrap();
    assert_eq!(decoded, policy);
    assert_eq!(decoded.fingerprint(), policy.fingerprint());
}

#[test]
fn policy_deserialization_cannot_supply_or_bypass_derived_state() {
    assert!(
        serde_json::from_value::<FixedWindowPolicy>(json!({
            "id": "auth.login",
            "scope": "identity",
            "limit": 8,
            "window_millis": 60_000,
            "fingerprint": "untrusted"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<FixedWindowPolicy>(json!({
            "id": "auth login",
            "scope": "identity",
            "limit": 8,
            "window_millis": 60_000
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<FixedWindowPolicy>(json!({
            "id": "auth.login",
            "scope": "identity",
            "limit": 0,
            "window_millis": 60_000
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<FixedWindowPolicy>(json!({
            "id": "auth.login",
            "scope": "identity",
            "limit": 8,
            "window_millis": MAX_WINDOW_MILLIS + 1
        }))
        .is_err()
    );
}

#[test]
fn policy_modes_default_to_enforcement_and_do_not_change_fingerprints() {
    let value = json!({
        "id": "auth.login",
        "scope": "identity",
        "limit": 8,
        "window_millis": 60_000
    });
    let enforced: FixedWindowPolicy = serde_json::from_value(value.clone()).unwrap();
    let shadow: FixedWindowPolicy = serde_json::from_value(json!({
        "id": "auth.login",
        "scope": "identity",
        "limit": 8,
        "window_millis": 60_000,
        "quota_mode": "shadow"
    }))
    .unwrap();

    assert_eq!(enforced.quota_mode(), QuotaMode::Enforce);
    assert_eq!(shadow.quota_mode(), QuotaMode::Shadow);
    assert_eq!(enforced.fingerprint(), shadow.fingerprint());
}

#[test]
fn gcra_policy_has_a_validated_wire_contract() {
    let policy = GcraPolicy::new(
        PolicyId::new("api.read").unwrap(),
        ScopeId::new("account").unwrap(),
        10,
        Duration::from_millis(1_500),
        20,
    )
    .unwrap()
    .with_quota_mode(QuotaMode::Shadow);
    let value = serde_json::to_value(&policy).unwrap();

    assert_eq!(
        value,
        json!({
            "id": "api.read",
            "scope": "account",
            "quota": 10,
            "period_millis": 1_500,
            "burst_capacity": 20,
            "quota_mode": "shadow"
        })
    );
    assert_eq!(serde_json::from_value::<GcraPolicy>(value).unwrap(), policy);
}

#[test]
fn decisions_and_denials_have_exact_tagged_wire_shapes() {
    let allowed = Decision::allowed(allowance(8, 7, Duration::new(1, 234_567)));
    let quota_denial = quota(8, Duration::new(2, 345_678));
    let denied = Decision::denied(quota_denial);
    let storage_denial = Denial::StorageCapacity { retry_after: None };

    assert_eq!(
        serde_json::to_value(allowed).unwrap(),
        json!({
            "outcome": "allowed",
            "capacity": 8,
            "available": 7,
            "replenishes_after": {"secs": 1, "nanos": 234_567}
        })
    );
    assert_eq!(
        serde_json::to_value(denied).unwrap(),
        json!({
            "outcome": "denied",
            "denial": {
                "reason": "quota_exceeded",
                "capacity": 8,
                "retry_after": {"secs": 2, "nanos": 345_678}
            }
        })
    );
    assert_eq!(
        serde_json::to_value(storage_denial).unwrap(),
        json!({
            "reason": "storage_capacity",
            "retry_after": null
        })
    );

    assert_eq!(
        serde_json::from_value::<Decision>(serde_json::to_value(allowed).unwrap()).unwrap(),
        allowed
    );
    assert_eq!(
        serde_json::from_value::<Decision>(serde_json::to_value(denied).unwrap()).unwrap(),
        denied
    );
    assert_eq!(
        serde_json::from_value::<Denial>(serde_json::to_value(storage_denial).unwrap()).unwrap(),
        storage_denial
    );
}

#[test]
fn decision_deserialization_rejects_impossible_metadata() {
    assert!(
        serde_json::from_value::<Decision>(json!({
            "outcome": "denied",
            "denial": {
                "reason": "quota_exceeded",
                "capacity": 0,
                "retry_after": {"secs": 1, "nanos": 0}
            }
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<Decision>(json!({
            "outcome": "allowed",
            "capacity": 8,
            "available": 9,
            "replenishes_after": {"secs": 1, "nanos": 0}
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<Decision>(json!({
            "outcome": "allowed",
            "capacity": 0,
            "available": 0,
            "replenishes_after": {"secs": 1, "nanos": 0}
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<Decision>(json!({
            "outcome": "allowed",
            "capacity": 8,
            "available": 7,
            "replenishes_after": {"secs": 1, "nanos": 0},
            "unexpected": true
        }))
        .is_err()
    );
}

#[test]
fn invalid_allowances_cannot_be_constructed() {
    assert_eq!(
        Allowance::new(capacity(8), 9, Duration::from_secs(1)),
        Err(DecisionError::AvailableExceedsCapacity {
            capacity: capacity(8),
            available: 9,
        })
    );
}

#[test]
fn zero_response_durations_round_trip_without_losing_backend_metadata() {
    let values = [
        Decision::allowed(allowance(8, 7, Duration::ZERO)),
        Decision::denied(quota(8, Duration::ZERO)),
        Decision::denied(Denial::StorageCapacity {
            retry_after: Some(Delay::new(Duration::ZERO)),
        }),
    ];

    for decision in values {
        let value = serde_json::to_value(decision).unwrap();
        assert_eq!(serde_json::from_value::<Decision>(value).unwrap(), decision);
    }
}

#[test]
fn shadow_decisions_round_trip_but_storage_capacity_cannot_be_shadowed() {
    let decision = Decision::shadow_denied(quota(8, Duration::from_secs(1)));
    let value = serde_json::to_value(decision).unwrap();

    assert_eq!(
        value,
        json!({
            "outcome": "shadow_denied",
            "denial": {
                "reason": "quota_exceeded",
                "capacity": 8,
                "retry_after": {"secs": 1, "nanos": 0}
            }
        })
    );
    assert_eq!(serde_json::from_value::<Decision>(value).unwrap(), decision);
    assert!(
        serde_json::from_value::<Decision>(json!({
            "outcome": "shadow_denied",
            "denial": {"reason": "storage_capacity", "retry_after": null}
        }))
        .is_err()
    );
}

#[test]
fn shadow_batches_round_trip_but_storage_capacity_cannot_be_shadowed() {
    let batch = BatchDecision::shadow_denied(2, 3, quota(8, Duration::from_secs(1))).unwrap();
    let value = serde_json::to_value(&batch).unwrap();
    assert_eq!(
        value,
        json!({
            "outcome": "shadow_denied",
            "index": 2,
            "batch_size": 3,
            "denial": {
                "reason": "quota_exceeded",
                "capacity": 8,
                "retry_after": {"secs": 1, "nanos": 0}
            }
        })
    );
    assert_eq!(
        serde_json::from_value::<BatchDecision>(value).unwrap(),
        batch
    );
    assert!(
        serde_json::from_value::<BatchDecision>(json!({
            "outcome": "shadow_denied",
            "index": 0,
            "batch_size": 1,
            "denial": {"reason": "storage_capacity", "retry_after": null}
        }))
        .is_err()
    );
}

#[test]
fn batch_decisions_round_trip_and_allowed_batches_carry_only_allowances() {
    let first = allowance(8, 7, Duration::from_mins(1));
    let second = allowance(3, 1, Duration::from_millis(750));
    let allowed = BatchDecision::allowed(vec![first, second]).unwrap();
    let denied = BatchDecision::denied(
        1,
        2,
        Denial::StorageCapacity {
            retry_after: Some(Delay::new(Duration::from_millis(5))),
        },
    )
    .unwrap();

    assert_eq!(
        serde_json::to_value(first).unwrap(),
        json!({
            "capacity": 8,
            "available": 7,
            "replenishes_after": {"secs": 60, "nanos": 0}
        })
    );
    assert_eq!(
        serde_json::from_value::<Allowance>(serde_json::to_value(first).unwrap()).unwrap(),
        first
    );
    assert_eq!(
        serde_json::to_value(&allowed).unwrap(),
        json!({
            "outcome": "allowed",
            "allowances": [
                {
                    "capacity": 8,
                    "available": 7,
                    "replenishes_after": {"secs": 60, "nanos": 0}
                },
                {
                    "capacity": 3,
                    "available": 1,
                    "replenishes_after": {"secs": 0, "nanos": 750_000_000}
                }
            ]
        })
    );
    assert_eq!(
        serde_json::from_value::<BatchDecision>(serde_json::to_value(&allowed).unwrap()).unwrap(),
        allowed
    );
    assert_eq!(
        serde_json::from_value::<BatchDecision>(serde_json::to_value(&denied).unwrap()).unwrap(),
        denied
    );
    assert_eq!(
        serde_json::to_value(&denied).unwrap(),
        json!({
            "outcome": "denied",
            "index": 1,
            "batch_size": 2,
            "denial": {
                "reason": "storage_capacity",
                "retry_after": {"secs": 0, "nanos": 5_000_000}
            }
        })
    );

    for outcome in ["allowed", "denied", "shadow_denied"] {
        assert!(
            serde_json::from_value::<BatchDecision>(json!({
                "outcome": "allowed",
                "allowances": [{
                    "outcome": outcome,
                    "capacity": 8,
                    "available": 7,
                    "replenishes_after": {"secs": 1, "nanos": 0}
                }]
            }))
            .is_err(),
            "an allowance is not a tagged decision object"
        );
    }
    assert!(
        serde_json::from_value::<BatchDecision>(json!({
            "outcome": "allowed",
            "allowances": [{
                "capacity": 8,
                "available": 9,
                "replenishes_after": {"secs": 1, "nanos": 0}
            }]
        }))
        .is_err(),
        "an allowed batch member must satisfy the allowance invariants"
    );
    assert!(
        serde_json::from_value::<BatchDecision>(json!({
            "outcome": "allowed",
            "decisions": []
        }))
        .is_err(),
        "an allowed batch must carry allowances"
    );
}

#[test]
fn allowed_batches_reject_an_empty_allowance_list() {
    assert!(
        serde_json::from_value::<BatchDecision>(json!({
            "outcome": "allowed",
            "allowances": []
        }))
        .is_err(),
        "an allowed batch must carry at least one allowance"
    );
}

#[test]
fn batch_denials_reject_indices_outside_the_batch_size() {
    let denial = json!({
        "reason": "quota_exceeded",
        "capacity": 8,
        "retry_after": {"secs": 1, "nanos": 0}
    });

    for outcome in ["denied", "shadow_denied"] {
        for (index, batch_size) in [(0, 0), (1, 1), (2, 1), (usize::MAX, 3)] {
            assert!(
                serde_json::from_value::<BatchDecision>(json!({
                    "outcome": outcome,
                    "index": index,
                    "batch_size": batch_size,
                    "denial": denial,
                }))
                .is_err(),
                "a {outcome} batch must reject index {index} of {batch_size}"
            );
        }
        assert!(
            serde_json::from_value::<BatchDecision>(json!({
                "outcome": outcome,
                "index": 0,
                "denial": denial,
            }))
            .is_err(),
            "a {outcome} batch must require its batch size"
        );
        let accepted = serde_json::from_value::<BatchDecision>(json!({
            "outcome": outcome,
            "index": 1,
            "batch_size": 2,
            "denial": denial,
        }))
        .unwrap();
        assert!(matches!(
            accepted.view(),
            BatchDecisionView::Denied { index: 1, .. }
                | BatchDecisionView::ShadowDenied { index: 1, .. }
        ));
    }
}
