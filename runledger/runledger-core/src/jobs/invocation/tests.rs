use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Wake;

struct CountingWaker(AtomicUsize);

impl Wake for CountingWaker {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn counting_waker() -> (Arc<CountingWaker>, Waker) {
    let counter = Arc::new(CountingWaker(AtomicUsize::new(0)));
    (Arc::clone(&counter), Waker::from(counter))
}

fn poll(future: &mut JobInvocationEnded, waker: &Waker) -> Poll<()> {
    Pin::new(future).poll(&mut Context::from_waker(waker))
}

fn waiter_count(invocation: &JobInvocation) -> usize {
    invocation.shared.lock().waiters.len()
}

#[test]
fn observation_starts_active_and_every_clone_sees_one_end() {
    let owner = JobInvocationOwner::new();
    let invocation = owner.invocation();
    let clone = invocation.clone();
    drop(owner.invocation());
    assert!(!invocation.has_ended() && !clone.has_ended());
    assert!(owner.end().is_empty());
    assert!(invocation.has_ended() && clone.has_ended());
    assert_eq!(format!("{clone:?}"), "JobInvocation { ended: true }");
}

#[test]
fn dropping_the_owner_ends_the_invocation_and_runs_hooks_in_order() {
    let owner = JobInvocationOwner::default();
    let invocation = owner.invocation();
    let order = Arc::new(Mutex::new(Vec::new()));
    for index in 0..3 {
        let order = Arc::clone(&order);
        invocation.on_end(move || order.lock().expect("order").push(index));
    }
    assert!(order.lock().expect("order").is_empty());
    drop(owner);
    assert!(invocation.has_ended());
    assert_eq!(*order.lock().expect("order"), [0, 1, 2]);
}

#[test]
fn waiters_are_woken_once_and_replaced_or_removed_without_duplicates() {
    let owner = JobInvocationOwner::new();
    let invocation = owner.invocation();
    let (first_count, first) = counting_waker();
    let (second_count, second) = counting_waker();
    let mut waiting = invocation.ended();
    assert_eq!(poll(&mut waiting, &first), Poll::Pending);
    assert_eq!(poll(&mut waiting, &second), Poll::Pending);
    assert_eq!(waiter_count(&invocation), 1, "re-polling updates in place");
    let mut abandoned = invocation.ended();
    assert_eq!(poll(&mut abandoned, &first), Poll::Pending);
    assert_eq!(waiter_count(&invocation), 2);
    drop(abandoned);
    assert_eq!(waiter_count(&invocation), 1, "a dropped waiter is removed");

    assert!(owner.end().is_empty());
    assert_eq!(first_count.0.load(Ordering::SeqCst), 0, "replaced waker");
    assert_eq!(second_count.0.load(Ordering::SeqCst), 1);
    assert_eq!(poll(&mut waiting, &second), Poll::Ready(()));
    assert_eq!(waiter_count(&invocation), 0);
}

#[test]
fn late_observers_complete_immediately_and_late_hooks_run_on_the_caller() {
    let owner = JobInvocationOwner::new();
    let invocation = owner.invocation();
    drop(owner);
    let (count, waker) = counting_waker();
    let mut late = invocation.clone().ended();
    assert_eq!(poll(&mut late, &waker), Poll::Ready(()));
    assert_eq!(count.0.load(Ordering::SeqCst), 0);
    let ran = Arc::new(AtomicUsize::new(0));
    let hook = Arc::clone(&ran);
    invocation.on_end(move || {
        hook.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(ran.load(Ordering::SeqCst), 1);
}

#[test]
fn a_panicking_hook_is_contained_reported_and_does_not_withhold_later_hooks() {
    let owner = JobInvocationOwner::new();
    let invocation = owner.invocation();
    let ran = Arc::new(AtomicUsize::new(0));
    let before = Arc::clone(&ran);
    invocation.on_end(move || {
        before.fetch_add(1, Ordering::SeqCst);
    });
    invocation.on_end(|| panic!("exit hook failure"));
    let after = Arc::clone(&ran);
    invocation.on_end(move || {
        after.fetch_add(1, Ordering::SeqCst);
    });
    let contained = owner.end();
    assert_eq!(ran.load(Ordering::SeqCst), 2);
    assert_eq!(contained.len(), 1);
    assert_eq!(
        format!("{contained:?}"),
        "JobInvocationHookPanics(1)",
        "formatting reports only the count"
    );
    let payloads = contained.into_payloads();
    assert_eq!(
        payloads[0].downcast_ref::<&str>(),
        Some(&"exit hook failure")
    );
}

#[test]
fn hooks_observe_the_end_and_may_register_more_hooks_without_deadlock() {
    let owner = JobInvocationOwner::new();
    let invocation = owner.invocation();
    let nested = Arc::new(AtomicUsize::new(0));
    let observed = invocation.clone();
    let counter = Arc::clone(&nested);
    invocation.on_end(move || {
        assert!(observed.has_ended());
        observed.on_end(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
    });
    assert!(owner.end().is_empty());
    assert_eq!(nested.load(Ordering::SeqCst), 1);
}

#[test]
fn observation_can_cross_threads_and_outlive_its_owner() {
    fn assert_owned<T: Send + Sync + 'static>() {}
    assert_owned::<JobInvocation>();
    assert_owned::<JobInvocationOwner>();
    fn assert_send<T: Send + 'static>() {}
    assert_send::<JobInvocationEnded>();

    let owner = JobInvocationOwner::new();
    let invocation = owner.invocation();
    let observer = std::thread::spawn(move || {
        let (count, waker) = counting_waker();
        let mut waiting = invocation.ended();
        while poll(&mut waiting, &waker).is_pending() {
            while count.0.load(Ordering::SeqCst) == 0 {
                std::thread::yield_now();
            }
        }
        invocation.has_ended()
    });
    drop(owner);
    assert!(observer.join().expect("observer thread"));
}
