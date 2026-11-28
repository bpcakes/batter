use std::time::Duration;

use super::{
    Capacity, CapacityError, FixedWindowPolicy, GcraPolicy, GcraPolicyError, MAX_LIMIT, MAX_WINDOW,
    MAX_WINDOW_MILLIS, PolicyError, QuotaMode, QuotaPeriod, QuotaPeriodError, RateLimitPolicy,
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
    let gcra = GcraPolicy::new(id.clone(), scope.clone(), 10, Duration::from_secs(1), 20).unwrap();
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
