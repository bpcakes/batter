//! Tests that read the store's retained state directly.
use super::{AttemptState, Clock, MemoryAttemptError, MemoryAttemptLimiter};
use runlimit_core::{
    KeyHasher, PolicyId, QuotaPeriod, ScopeId,
    attempts::{
        AttemptAdmission, AttemptCompletionResult, AttemptDenial, AttemptObservation,
        AttemptObserver, AttemptOutcome, AttemptPolicy, FailedAttempts,
    },
};
use std::{
    num::{NonZeroU32, NonZeroUsize},
    sync::{
        Arc, Mutex,
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
#[derive(Default)]
struct Recorder(Mutex<Vec<AttemptObservation>>);
impl AttemptObserver for Recorder {
    fn observe(&self, observation: AttemptObservation) {
        self.0.lock().unwrap().push(observation);
    }
}
fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

/// Initial delay 10 ms, cap 40 ms, quiet period 100 ms, lease 20 ms.
fn policy() -> AttemptPolicy {
    let period = |value| QuotaPeriod::new(ms(value)).unwrap();
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

#[test]
fn a_refused_lease_deadline_still_stores_the_settled_subject() {
    #[derive(Clone)]
    struct Late(Arc<Mutex<Duration>>);
    impl Clock for Late {
        fn now(&self) -> Duration {
            *self.0.lock().unwrap()
        }
    }
    let policy = policy();
    let end = Duration::MAX;
    let clock = Late(Arc::new(Mutex::new(end.saturating_sub(ms(150)))));
    let limiter = MemoryAttemptLimiter::with_clock(NonZeroUsize::MIN, clock.clone());
    let subject = KeyHasher::new([7; 32])
        .unwrap()
        .hash_attempt_for(&policy, b"user");
    let AttemptAdmission::Admitted(_abandoned) = limiter.admit(subject).unwrap() else {
        panic!("first admission was denied");
    };
    // This scan rests the cursor on the only entry, so the next admission's
    // scan examines nothing and the evaluation alone settles the subject.
    assert!(matches!(
        limiter.admit(subject).unwrap(),
        AttemptAdmission::Denied(AttemptDenial::Busy { .. })
    ));
    // 140 ms later the lease has expired and its quiet period has passed,
    // and no 20 ms lease fits in the 10 ms left.
    *clock.0.lock().unwrap() = end.saturating_sub(ms(10));
    assert!(matches!(
        limiter.admit(subject),
        Err(MemoryAttemptError::ClockExhausted)
    ));
    assert!(limiter.state.lock().unwrap().entries.is_empty());
}

#[test]
fn cleanup_stores_the_failure_of_an_expired_lease_it_retains() {
    let policy = policy();
    let clock = TestClock::default();
    let limiter = MemoryAttemptLimiter::with_clock(NonZeroUsize::new(2).unwrap(), clock.clone());
    let hasher = KeyHasher::new([7; 32]).unwrap();
    let AttemptAdmission::Admitted(_abandoned) = limiter
        .admit(hasher.hash_attempt_for(&policy, b"first"))
        .unwrap()
    else {
        panic!("first admission was denied");
    };
    // Another subject's admission scans the first entry after its 20 ms
    // deadline and inside its quiet period, so it is retained as a failure.
    clock.0.store(25, Ordering::SeqCst);
    let AttemptAdmission::Admitted(_held) = limiter
        .admit(hasher.hash_attempt_for(&policy, b"second"))
        .unwrap()
    else {
        panic!("second admission was denied");
    };
    let state = limiter.state.lock().unwrap();
    let expired = AttemptState::Failed(FailedAttempts::new(NonZeroU32::MIN, ms(20), ms(30)));
    let mut states: Vec<_> = state.entries.values().map(|entry| entry.state).collect();
    states.retain(|state| !matches!(state, AttemptState::Leased(_)));
    assert_eq!(state.entries.len(), 2);
    assert_eq!(states, [expired]);
}

#[test]
fn completing_an_expired_receipt_counts_the_expiry_without_waiting_for_an_admission() {
    let policy = policy();
    let clock = TestClock::default();
    let recorder = Arc::new(Recorder::default());
    let limiter = MemoryAttemptLimiter::with_clock(NonZeroUsize::MIN, clock.clone())
        .with_observer(recorder.clone());
    let subject = KeyHasher::new([7; 32])
        .unwrap()
        .hash_attempt_for(&policy, b"user");
    let stored = |limiter: &MemoryAttemptLimiter<TestClock>| {
        let state = limiter.state.lock().unwrap();
        assert!(state.entries.len() <= 1);
        state.entries.values().next().map(|entry| entry.state)
    };

    let AttemptAdmission::Admitted(expired) = limiter.admit(subject).unwrap() else {
        panic!("first admission was denied");
    };
    clock.0.store(25, Ordering::SeqCst);
    assert_eq!(
        limiter.complete(expired, AttemptOutcome::Success).unwrap(),
        AttemptCompletionResult::Stale
    );
    // The stale completion did not apply its success, but the entry no
    // longer claims a lease: the expiry is the first failure, dated at the
    // 20 ms deadline rather than at the 25 ms it was noticed.
    assert_eq!(
        stored(&limiter),
        Some(AttemptState::Failed(FailedAttempts::new(
            NonZeroU32::MIN,
            ms(20),
            ms(30)
        )))
    );
    let backoff = AttemptDenial::Backoff {
        retry_after: runlimit_core::Delay::new(ms(5)),
    };
    assert!(matches!(
        limiter.admit(subject).unwrap(),
        AttemptAdmission::Denied(denial) if denial == backoff
    ));

    // A stale completion that finds the quiet period over releases the
    // capacity slot at once instead of leaving it to a later cleanup scan.
    clock.0.store(30, Ordering::SeqCst);
    let AttemptAdmission::Admitted(quiet) = limiter.admit(subject).unwrap() else {
        panic!("admission after the delay was denied");
    };
    clock.0.store(150, Ordering::SeqCst);
    assert_eq!(
        limiter.complete(quiet, AttemptOutcome::Failure).unwrap(),
        AttemptCompletionResult::Stale
    );
    assert_eq!(stored(&limiter), None);
    assert_eq!(
        *recorder.0.lock().unwrap(),
        [
            AttemptObservation::Admitted,
            AttemptObservation::Stale,
            AttemptObservation::Denied(backoff),
            AttemptObservation::Admitted,
            AttemptObservation::Stale,
        ]
    );
}
