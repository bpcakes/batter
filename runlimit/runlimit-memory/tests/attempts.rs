//! Deterministic lifecycle and bounded-storage tests for outcome-aware attempts.
use runlimit_core::{
    KeyHasher, PolicyId, QuotaPeriod, ScopeId,
    attempts::{
        AttemptAdmission, AttemptCompletionResult, AttemptDenial, AttemptOutcome, AttemptPolicy,
        AttemptPolicyError,
    },
};
use runlimit_memory::{
    Clock,
    attempts::{MemoryAttemptLimiter, MemoryAttemptReceipt},
};
use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Default)]
struct TestClock(Arc<AtomicU64>);
impl Clock for TestClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}
impl TestClock {
    fn set(&self, millis: u64) {
        self.0.store(millis, Ordering::SeqCst);
    }
}
fn period(ms: u64) -> QuotaPeriod {
    QuotaPeriod::new(Duration::from_millis(ms)).unwrap()
}
fn policy() -> AttemptPolicy {
    AttemptPolicy::new(
        PolicyId::new("auth").unwrap(),
        ScopeId::new("identifier").unwrap(),
        period(10),
        period(40),
        period(100),
        period(20),
    )
    .unwrap()
}
fn receipt(value: AttemptAdmission<MemoryAttemptReceipt>) -> MemoryAttemptReceipt {
    match value {
        AttemptAdmission::Admitted(receipt) => receipt,
        AttemptAdmission::Denied(denial) => panic!("unexpected denial {denial:?}"),
    }
}

#[test]
fn failure_delays_escalate_cap_and_success_resets() {
    let clock = TestClock::default();
    let store = MemoryAttemptLimiter::with_clock(NonZeroUsize::new(2).unwrap(), clock.clone());
    let policy = policy();
    let hasher = KeyHasher::new([7; 32]).unwrap();
    let subject = hasher.hash_attempt_for(&policy, b"user");
    let mut now = 0;
    for (expected, delay) in [(1, 10), (2, 20), (3, 40), (4, 40)] {
        let admission = receipt(store.admit(subject).unwrap());
        assert!(matches!(
            store.admit(subject).unwrap(),
            AttemptAdmission::Denied(AttemptDenial::Busy { .. })
        ));
        let AttemptCompletionResult::Applied(done) =
            store.complete(admission, AttemptOutcome::Failure).unwrap()
        else {
            panic!()
        };
        assert_eq!(done.consecutive_failures(), expected);
        assert_eq!(done.retry_after().duration(), Duration::from_millis(delay));
        assert!(matches!(
            store.admit(subject).unwrap(),
            AttemptAdmission::Denied(AttemptDenial::Backoff { .. })
        ));
        now += delay;
        clock.set(now);
    }
    let success = receipt(store.admit(subject).unwrap());
    assert!(
        matches!(store.complete(success,AttemptOutcome::Success).unwrap(),AttemptCompletionResult::Applied(done) if done.consecutive_failures()==0)
    );
    let next = receipt(store.admit(subject).unwrap());
    assert!(
        matches!(store.complete(next,AttemptOutcome::Failure).unwrap(),AttemptCompletionResult::Applied(done) if done.consecutive_failures()==1)
    );
}

#[test]
fn stale_receipts_cannot_reset_new_failure_state_or_other_stores() {
    let clock = TestClock::default();
    let store = MemoryAttemptLimiter::with_clock(NonZeroUsize::new(1).unwrap(), clock.clone());
    let other = MemoryAttemptLimiter::with_clock(NonZeroUsize::new(1).unwrap(), clock.clone());
    let policy = policy();
    let hasher = KeyHasher::new([7; 32]).unwrap();
    let subject = hasher.hash_attempt_for(&policy, b"user");
    let old = receipt(store.admit(subject).unwrap());
    clock.set(20);
    assert!(matches!(
        store.admit(subject).unwrap(),
        AttemptAdmission::Denied(AttemptDenial::Backoff { .. })
    ));
    clock.set(30);
    let current = receipt(store.admit(subject).unwrap());
    assert_eq!(
        store.complete(old, AttemptOutcome::Success).unwrap(),
        AttemptCompletionResult::Stale
    );
    assert_eq!(
        other.complete(current, AttemptOutcome::Success).unwrap(),
        AttemptCompletionResult::Stale
    );
    assert!(matches!(
        store.admit(subject).unwrap(),
        AttemptAdmission::Denied(AttemptDenial::Busy { .. })
    ));
}

#[test]
fn bounded_storage_reclaims_quiet_subjects_and_never_evicts_live_lease() {
    let clock = TestClock::default();
    let store = MemoryAttemptLimiter::with_clock(NonZeroUsize::new(1).unwrap(), clock.clone());
    let policy = policy();
    let hasher = KeyHasher::new([7; 32]).unwrap();
    let first = hasher.hash_attempt_for(&policy, b"first");
    let second = hasher.hash_attempt_for(&policy, b"second");
    drop(receipt(store.admit(first).unwrap()));
    clock.set(19);
    assert!(matches!(
        store.admit(second).unwrap(),
        AttemptAdmission::Denied(AttemptDenial::StorageCapacity)
    ));
    clock.set(119);
    assert!(matches!(
        store.admit(second).unwrap(),
        AttemptAdmission::Denied(AttemptDenial::StorageCapacity)
    ));
    clock.set(120); // bounded cursor may wrap on a subsequent operation
    for _ in 0..2 {
        if matches!(store.admit(second).unwrap(), AttemptAdmission::Admitted(_)) {
            return;
        }
    }
    panic!("expired subject not reclaimed");
}

#[test]
fn quiet_period_resets_failures_and_abandonment_counts() {
    let clock = TestClock::default();
    let store = MemoryAttemptLimiter::with_clock(NonZeroUsize::new(1).unwrap(), clock.clone());
    let policy = policy();
    let hasher = KeyHasher::new([7; 32]).unwrap();
    let subject = hasher.hash_attempt_for(&policy, b"user");
    let first = receipt(store.admit(subject).unwrap());
    assert!(
        matches!(store.complete(first,AttemptOutcome::Abandoned).unwrap(),AttemptCompletionResult::Applied(done) if done.consecutive_failures()==1)
    );
    clock.set(100);
    let next = receipt(store.admit(subject).unwrap());
    assert!(
        matches!(store.complete(next,AttemptOutcome::Failure).unwrap(),AttemptCompletionResult::Applied(done) if done.consecutive_failures()==1)
    );
}

#[test]
fn policy_validates_relations_fingerprints_all_parameters_and_bounds_overflow() {
    let original = policy();
    assert_eq!(original.delay(u32::MAX), Duration::from_millis(40));
    let build = |values: [u64; 4]| {
        AttemptPolicy::new(
            original.id().clone(),
            original.scope().clone(),
            period(values[0]),
            period(values[1]),
            period(values[2]),
            period(values[3]),
        )
    };
    assert_eq!(
        build([40, 10, 100, 20]),
        Err(AttemptPolicyError::MaximumBelowInitial)
    );
    assert_eq!(
        build([10, 40, 20, 20]),
        Err(AttemptPolicyError::QuietBelowMaximum)
    );
    for values in [
        [11, 40, 100, 20],
        [10, 41, 100, 20],
        [10, 40, 101, 20],
        [10, 40, 100, 21],
    ] {
        assert_ne!(original.fingerprint(), build(values).unwrap().fingerprint());
    }
}

#[test]
fn backwards_clock_does_not_shorten_delay_or_clear_active_lease() {
    let clock = TestClock::default();
    clock.set(100);
    let store = MemoryAttemptLimiter::with_clock(NonZeroUsize::new(1).unwrap(), clock.clone());
    let policy = policy();
    let hasher = KeyHasher::new([7; 32]).unwrap();
    let subject = hasher.hash_attempt_for(&policy, b"user");
    let r = receipt(store.admit(subject).unwrap());
    store.complete(r, AttemptOutcome::Failure).unwrap();
    clock.set(1);
    assert!(
        matches!(store.admit(subject).unwrap(), AttemptAdmission::Denied(AttemptDenial::Backoff {retry_after}) if retry_after.duration()==Duration::from_millis(10))
    );
    clock.set(110);
    let current = receipt(store.admit(subject).unwrap());
    clock.set(0);
    assert!(
        matches!(store.admit(subject).unwrap(), AttemptAdmission::Denied(AttemptDenial::Busy{retry_after}) if retry_after.duration()==Duration::from_millis(20))
    );
    assert!(matches!(
        store.complete(current, AttemptOutcome::Success).unwrap(),
        AttemptCompletionResult::Applied(_)
    ));
}

#[test]
fn quiet_expiry_never_evicts_a_longer_active_lease() {
    let clock = TestClock::default();
    let store = MemoryAttemptLimiter::with_clock(NonZeroUsize::new(1).unwrap(), clock.clone());
    let policy = AttemptPolicy::new(
        PolicyId::new("auth").unwrap(),
        ScopeId::new("identifier").unwrap(),
        period(10),
        period(40),
        period(100),
        period(1000),
    )
    .unwrap();
    let hasher = KeyHasher::new([7; 32]).unwrap();
    let first = hasher.hash_attempt_for(&policy, b"first");
    let r = receipt(store.admit(first).unwrap());
    clock.set(500);
    assert!(matches!(
        store.admit(first).unwrap(),
        AttemptAdmission::Denied(AttemptDenial::Busy { .. })
    ));
    assert!(matches!(
        store
            .admit(hasher.hash_attempt_for(&policy, b"other"))
            .unwrap(),
        AttemptAdmission::Denied(AttemptDenial::StorageCapacity)
    ));
    assert!(matches!(
        store.complete(r, AttemptOutcome::Success).unwrap(),
        AttemptCompletionResult::Applied(_)
    ));
}

#[test]
fn completion_error_can_only_report_poisoned_state() {
    use runlimit_memory::attempts::MemoryAttemptCompletionError;
    use std::sync::atomic::AtomicBool;
    #[derive(Clone)]
    struct PanickingClock(Arc<AtomicBool>);
    impl Clock for PanickingClock {
        fn now(&self) -> Duration {
            assert!(!self.0.load(Ordering::SeqCst), "injected clock panic");
            Duration::ZERO
        }
    }
    let panic_clock = Arc::new(AtomicBool::new(false));
    let store = MemoryAttemptLimiter::with_clock(
        NonZeroUsize::new(1).unwrap(),
        PanickingClock(panic_clock.clone()),
    );
    let policy = policy();
    let h = KeyHasher::new([7; 32]).unwrap();
    let subject = h.hash_attempt_for(&policy, b"user");
    let r = receipt(store.admit(subject).unwrap());
    panic_clock.store(true, Ordering::SeqCst);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.admit(subject))).is_err()
    );
    let MemoryAttemptCompletionError = store.complete(r, AttemptOutcome::Success).unwrap_err();
}
