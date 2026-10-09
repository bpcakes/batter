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

struct ReentrantWaker {
    invocation: JobInvocation,
    drops: Arc<AtomicUsize>,
    wakes: Arc<AtomicUsize>,
}

impl Wake for ReentrantWaker {
    fn wake(self: Arc<Self>) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
    }
}

impl Drop for ReentrantWaker {
    fn drop(&mut self) {
        assert!(!self.invocation.has_ended());
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn assert_reentrant_waker_drop_completes(replace: bool) {
    let (finished, completion) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let owner = JobInvocationOwner::new();
        let invocation = owner.invocation();
        let drops = Arc::new(AtomicUsize::new(0));
        let wakes = Arc::new(AtomicUsize::new(0));
        let waker = Waker::from(Arc::new(ReentrantWaker {
            invocation: invocation.clone(),
            drops: Arc::clone(&drops),
            wakes: Arc::clone(&wakes),
        }));
        let mut waiting = invocation.ended();
        assert_eq!(poll(&mut waiting, &waker), Poll::Pending);
        drop(waker); // Only the registered waiter now retains this waker.
        let (next_count, next) = counting_waker();
        if replace {
            assert_eq!(poll(&mut waiting, &next), Poll::Pending);
        } else {
            drop(waiting);
        }
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        let hooks = Arc::new(AtomicUsize::new(0));
        let ran = Arc::clone(&hooks);
        invocation.on_end(move || {
            ran.fetch_add(1, Ordering::SeqCst);
        });
        assert!(owner.end().is_empty());
        assert!(invocation.has_ended());
        assert_eq!(hooks.load(Ordering::SeqCst), 1);
        assert_eq!(wakes.load(Ordering::SeqCst), 0);
        assert_eq!(next_count.0.load(Ordering::SeqCst), usize::from(replace));
        finished.send(()).expect("completion receiver");
    });
    completion
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("waker destruction must not deadlock invocation state");
    worker.join().expect("waiter thread");
}

#[test]
fn removing_a_waiter_destroys_its_waker_outside_the_invocation_lock() {
    assert_reentrant_waker_drop_completes(false);
}

#[test]
fn replacing_a_waiter_destroys_its_waker_outside_the_invocation_lock() {
    assert_reentrant_waker_drop_completes(true);
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

struct PanickingWaker;

impl Wake for PanickingWaker {
    fn wake(self: Arc<Self>) {
        panic!("waiter wake failure");
    }
}

#[test]
fn panicking_waiters_do_not_withhold_notifications_on_end_or_drop() {
    for explicit_end in [true, false] {
        let owner = JobInvocationOwner::new();
        let invocation = owner.invocation();
        let (count, counting) = counting_waker();
        let panicking = Waker::from(Arc::new(PanickingWaker));
        let mut waiters = Vec::new();
        for waker in [&panicking, &counting, &panicking, &counting] {
            let mut waiter = invocation.ended();
            assert_eq!(poll(&mut waiter, waker), Poll::Pending);
            waiters.push(waiter);
        }
        let ran = Arc::new(AtomicUsize::new(0));
        let before = Arc::clone(&ran);
        let woken = Arc::clone(&count);
        invocation.on_end(move || {
            assert_eq!(woken.0.load(Ordering::SeqCst), 2, "waiters precede hooks");
            before.fetch_add(1, Ordering::SeqCst);
        });
        invocation.on_end(|| panic!("exit hook failure"));
        let after = Arc::clone(&ran);
        invocation.on_end(move || {
            after.fetch_add(1, Ordering::SeqCst);
        });
        if explicit_end {
            let contained = owner.end();
            assert_eq!(contained.len(), 3);
            assert_eq!(format!("{contained:?}"), "JobInvocationHookPanics(3)");
            let payloads = contained.into_payloads();
            let messages: Vec<_> = payloads
                .iter()
                .map(|payload| *payload.downcast_ref::<&str>().expect("panic message"))
                .collect();
            assert_eq!(
                messages,
                [
                    "waiter wake failure",
                    "waiter wake failure",
                    "exit hook failure"
                ]
            );
        } else {
            drop(owner);
        }
        assert!(invocation.has_ended());
        assert_eq!(count.0.load(Ordering::SeqCst), 2);
        assert_eq!(ran.load(Ordering::SeqCst), 2);
        for mut waiter in waiters {
            assert_eq!(poll(&mut waiter, &counting), Poll::Ready(()));
        }
        assert_eq!(
            count.0.load(Ordering::SeqCst),
            2,
            "each waiter is woken once"
        );
    }
}

struct PanicOnDrop(Arc<AtomicUsize>);

impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("opaque payload destruction");
    }
}

struct PayloadWaker(Arc<AtomicUsize>);

impl Wake for PayloadWaker {
    fn wake(self: Arc<Self>) {
        std::panic::panic_any(PanicOnDrop(Arc::clone(&self.0)));
    }
}

#[test]
fn owner_and_report_drop_never_destroy_opaque_panic_payloads() {
    for explicit_end in [true, false] {
        let owner = JobInvocationOwner::new();
        let invocation = owner.invocation();
        let destroyed = Arc::new(AtomicUsize::new(0));
        let waker = Waker::from(Arc::new(PayloadWaker(Arc::clone(&destroyed))));
        let mut waiter = invocation.ended();
        assert_eq!(poll(&mut waiter, &waker), Poll::Pending);
        let hook_payload = Arc::clone(&destroyed);
        invocation.on_end(move || std::panic::panic_any(PanicOnDrop(hook_payload)));
        invocation.on_end(|| panic!("later hook failure"));
        let ran = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&ran);
        invocation.on_end(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        });

        if explicit_end {
            let report = owner.end();
            let mut payloads = report.payloads();
            assert_eq!(payloads.len(), 3);
            assert!(payloads.next().expect("wake panic").is::<PanicOnDrop>());
            assert!(payloads.next().expect("hook panic").is::<PanicOnDrop>());
            assert_eq!(
                payloads.next().expect("later panic").downcast_ref::<&str>(),
                Some(&"later hook failure")
            );
        } else {
            drop(owner);
        }
        assert!(invocation.has_ended());
        assert_eq!(ran.load(Ordering::SeqCst), 1);
        assert_eq!(
            destroyed.load(Ordering::SeqCst),
            0,
            "opaque allocations retained"
        );
    }
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
