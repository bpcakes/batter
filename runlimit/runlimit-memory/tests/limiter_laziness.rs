//! The async trait must defer consumption until its future is polled.

use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
    time::Duration,
};

use runlimit_core::{
    BatchDecisionView, Check, DecisionView, FixedWindowPolicy, GcraPolicy, Limiter, PolicyId,
    ScopeId, SubjectKey,
};
use runlimit_memory::{Clock, GcraStore, MemoryStore, MemoryStoreConfig};

struct FrozenClock;

impl Clock for FrozenClock {
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}

fn poll_ready<F: Future>(future: F) -> F::Output {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("memory checks complete on their first poll"),
    }
}

fn assert_lazy<L: Limiter>(limiter: &L, check: &Check<'_, L::Policy>, batch: bool) {
    let checks = std::slice::from_ref(check);
    if batch {
        drop(limiter.check_all(checks));
    } else {
        drop(limiter.check(check));
    }

    let first_allowance = if batch {
        match poll_ready(limiter.check_all(checks)).unwrap().view() {
            BatchDecisionView::Allowed {
                allowances: [allowance],
            } => *allowance,
            other => panic!("the first one-check batch is allowed: {other:?}"),
        }
    } else {
        match poll_ready(limiter.check(check)).unwrap().view() {
            DecisionView::Allowed { allowance } => allowance,
            other => panic!("the first check is allowed: {other:?}"),
        }
    };
    assert_eq!(first_allowance.available(), 0);
    assert!(!poll_ready(limiter.check(check)).unwrap().permits_request());
}

fn fixed_window(batch: bool) {
    let store = MemoryStore::builder(MemoryStoreConfig::new(1).unwrap())
        .with_clock(FrozenClock)
        .build();
    let policy = FixedWindowPolicy::new(
        PolicyId::new("lazy").unwrap(),
        ScopeId::new("client").unwrap(),
        1,
        Duration::from_secs(60),
    )
    .unwrap();
    assert_lazy(
        &store,
        &Check::new(SubjectKey::from_digest([1; 32]).bind(&policy)),
        batch,
    );
}

fn gcra(batch: bool) {
    let store = GcraStore::builder(MemoryStoreConfig::new(1).unwrap())
        .with_clock(FrozenClock)
        .build();
    let policy = GcraPolicy::new(
        PolicyId::new("lazy").unwrap(),
        ScopeId::new("client").unwrap(),
        1,
        Duration::from_secs(60),
        1,
    )
    .unwrap();
    assert_lazy(
        &store,
        &Check::new(SubjectKey::from_digest([1; 32]).bind(&policy)),
        batch,
    );
}

#[test]
fn fixed_window_single_consumes_only_when_polled() {
    fixed_window(false);
}

#[test]
fn fixed_window_batch_consumes_only_when_polled() {
    fixed_window(true);
}

#[test]
fn gcra_single_consumes_only_when_polled() {
    gcra(false);
}

#[test]
fn gcra_batch_consumes_only_when_polled() {
    gcra(true);
}
