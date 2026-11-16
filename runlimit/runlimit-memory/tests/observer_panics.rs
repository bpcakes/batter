//! A failed observation must not hide an already-applied state transition.

use std::{
    num::NonZeroUsize,
    panic::panic_any,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use runlimit_core::{
    Check, FixedWindowPolicy, GcraPolicy, KeyHasher, Observation, Observer, PolicyId, QuotaPeriod,
    ScopeId,
    attempts::{
        AttemptAdmission, AttemptCompletionResult, AttemptDenial, AttemptObservation,
        AttemptObserver, AttemptOutcome, AttemptPolicy,
    },
};
use runlimit_memory::{
    Clock, GcraStore, MemoryStore, MemoryStoreConfig, attempts::MemoryAttemptLimiter,
};

struct FrozenClock;
impl Clock for FrozenClock {
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}

struct Payload;
impl Drop for Payload {
    fn drop(&mut self) {
        panic!("opaque payload destructor ran");
    }
}

#[derive(Default)]
struct PanickingObserver(AtomicUsize);
impl Observer for PanickingObserver {
    fn observe(&self, observation: &Observation<'_>) {
        if matches!(observation, Observation::Admission(_)) {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic_any(Payload);
        }
    }
}
impl AttemptObserver for PanickingObserver {
    fn observe(&self, _: AttemptObservation) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic_any(Payload);
    }
}

#[test]
fn fixed_window_returns_consumption_and_later_denial_despite_observer_payload() {
    for batch in [false, true] {
        let observer = Arc::new(PanickingObserver::default());
        let store = MemoryStore::builder(MemoryStoreConfig::new(1).unwrap())
            .with_clock(FrozenClock)
            .build()
            .with_observer(observer.clone());
        let policy = FixedWindowPolicy::new(
            PolicyId::new("quota").unwrap(),
            ScopeId::new("subject").unwrap(),
            1,
            Duration::from_secs(60),
        )
        .unwrap();
        let hasher = KeyHasher::new([7; 32]).unwrap();
        let check = Check::new(hasher.hash_for(&policy, b"subject"));
        if batch {
            assert!(store.check_all(&[check]).unwrap().permits_request());
        } else {
            assert!(store.check(&check).unwrap().permits_request());
        }
        assert!(!store.check(&check).unwrap().permits_request());
        assert_eq!(observer.0.load(Ordering::SeqCst), 2);
    }
}

#[test]
fn gcra_returns_consumption_and_later_denial_despite_observer_payload() {
    for batch in [false, true] {
        let observer = Arc::new(PanickingObserver::default());
        let store = GcraStore::builder(MemoryStoreConfig::new(1).unwrap())
            .with_clock(FrozenClock)
            .build()
            .with_observer(observer.clone());
        let policy = GcraPolicy::new(
            PolicyId::new("quota").unwrap(),
            ScopeId::new("subject").unwrap(),
            1,
            Duration::from_secs(60),
            1,
        )
        .unwrap();
        let hasher = KeyHasher::new([7; 32]).unwrap();
        let check = Check::new(hasher.hash_for(&policy, b"subject"));
        if batch {
            assert!(store.check_all(&[check]).unwrap().permits_request());
        } else {
            assert!(store.check(&check).unwrap().permits_request());
        }
        assert!(!store.check(&check).unwrap().permits_request());
        assert_eq!(observer.0.load(Ordering::SeqCst), 2);
    }
}

#[test]
fn attempt_returns_receipt_and_completion_despite_observer_payload() {
    for outcome in [
        AttemptOutcome::Success,
        AttemptOutcome::Failure,
        AttemptOutcome::Abandoned,
    ] {
        let observer = Arc::new(PanickingObserver::default());
        let store = MemoryAttemptLimiter::with_clock(NonZeroUsize::new(1).unwrap(), FrozenClock)
            .with_observer(observer.clone());
        let period = |ms| QuotaPeriod::new(Duration::from_millis(ms)).unwrap();
        let policy = AttemptPolicy::new(
            PolicyId::new("attempt").unwrap(),
            ScopeId::new("subject").unwrap(),
            period(10),
            period(40),
            period(100),
            period(20),
        )
        .unwrap();
        let hasher = KeyHasher::new([7; 32]).unwrap();
        let subject = hasher.hash_attempt_for(&policy, b"subject");
        let AttemptAdmission::Admitted(receipt) = store.admit(subject).unwrap() else {
            panic!("first attempt admitted");
        };
        assert!(matches!(
            store.admit(subject).unwrap(),
            AttemptAdmission::Denied(AttemptDenial::Busy { .. })
        ));
        let AttemptCompletionResult::Applied(completed) = store.complete(receipt, outcome).unwrap()
        else {
            panic!("current receipt applies");
        };
        assert_eq!(completed.outcome(), outcome);
        let next = store.admit(subject).unwrap();
        match outcome {
            AttemptOutcome::Success => assert!(matches!(next, AttemptAdmission::Admitted(_))),
            AttemptOutcome::Failure | AttemptOutcome::Abandoned => assert!(matches!(
                next,
                AttemptAdmission::Denied(AttemptDenial::Backoff { .. })
            )),
        }
        assert_eq!(observer.0.load(Ordering::SeqCst), 4);
    }
}
