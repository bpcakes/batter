//! Observer containment includes disposal of the caught payload.

use std::{
    panic::panic_any,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use runlimit_core::{
    CleanupObservation, CleanupOutcome, Observation, Observer,
    attempts::{AttemptObservation, AttemptObserver, observe_attempt_safely},
    observe_safely,
};

enum Failure {
    BorrowedString,
    OwnedString,
    Opaque(Arc<AtomicUsize>),
}

struct Payload(Arc<AtomicUsize>);

impl Drop for Payload {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("opaque payload destructor ran");
    }
}

impl Failure {
    fn fail(&self) {
        match self {
            Self::BorrowedString => panic_any("observer failure"),
            Self::OwnedString => panic_any(String::from("observer failure")),
            Self::Opaque(drops) => panic_any(Payload(drops.clone())),
        }
    }
}

impl Observer for Failure {
    fn observe(&self, _: &Observation<'_>) {
        self.fail();
    }
}

impl AttemptObserver for Failure {
    fn observe(&self, _: AttemptObservation) {
        self.fail();
    }
}

#[test]
fn quota_observations_return_after_string_and_opaque_payload_panics() {
    let drops = Arc::new(AtomicUsize::new(0));
    let event = Observation::Cleanup(CleanupObservation::new(
        1,
        CleanupOutcome::NoEffect,
        Duration::ZERO,
    ));
    for failure in [
        Failure::BorrowedString,
        Failure::OwnedString,
        Failure::Opaque(drops.clone()),
    ] {
        observe_safely(&failure, &event);
        observe_safely(&failure, &event);
    }
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(Arc::strong_count(&drops), 3, "two opaque payloads retained");
}

#[test]
fn attempt_observations_return_after_string_and_opaque_payload_panics() {
    let drops = Arc::new(AtomicUsize::new(0));
    for failure in [
        Failure::BorrowedString,
        Failure::OwnedString,
        Failure::Opaque(drops.clone()),
    ] {
        observe_attempt_safely(&failure, AttemptObservation::Admitted);
        observe_attempt_safely(&failure, AttemptObservation::Stale);
    }
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(Arc::strong_count(&drops), 3, "two opaque payloads retained");
}
