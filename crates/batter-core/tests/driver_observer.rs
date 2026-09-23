use batter_core::lifecycle::ComponentExit;
use batter_core::{
    BoxError,
    cleanup::CleanupBudget,
    lifecycle::{ProcessCapacity, Readiness, ShutdownBudget, ShutdownFailure, Supervisor},
};
use std::{
    future::{Future, pending, poll_fn},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
    time::Duration,
};

fn budget() -> ShutdownBudget {
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second, second, second).unwrap();
    ShutdownBudget::new(second, second, second, cleanup).unwrap()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn observer_survives_last_owner_drop_before_coordinator_first_poll() {
    let mut supervisor = Supervisor::new(budget());
    let started = Arc::new(AtomicUsize::new(0));
    let in_component = started.clone();
    supervisor
        .register("worker", move |startup| async move {
            in_component.fetch_add(1, Ordering::SeqCst);
            startup.shutdown().draining().await;
            Ok(startup.abandon())
        })
        .unwrap();
    let finalized = Arc::new(AtomicUsize::new(0));
    let in_cleanup = finalized.clone();
    supervisor
        .on_cleanup("dependency", move || async move {
            in_cleanup.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();

    // No await on this current-thread runtime: the coordinator has not polled.
    let pending_owner = supervisor.start();
    assert_eq!(pending_owner.status().readiness(), Readiness::Starting);
    let observer = pending_owner.observer();
    let another_observer = observer.clone();
    drop(pending_owner);
    assert_eq!(started.load(Ordering::SeqCst), 0);
    assert_eq!(finalized.load(Ordering::SeqCst), 0);

    let report = tokio::time::timeout(Duration::from_secs(5), observer.wait())
        .await
        .expect("owned driver must publish after last-owner drop")
        .unwrap();
    assert!(report.is_success());
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(finalized.load(Ordering::SeqCst), 1);
    drop(observer);
    let retained = another_observer.wait().await.unwrap();
    assert!(std::ptr::eq(&*report, &*retained));
    let checked = another_observer.wait_checked().await.unwrap();
    assert!(std::ptr::eq(&*report, &**checked.report()));
    let extracted = checked.into_report();
    assert!(std::ptr::eq(&*report, &*extracted));
}

#[tokio::test(start_paused = true)]
async fn coordinator_panic_is_retained_after_last_owner_drop_before_first_poll() {
    struct PanickingDrop(Arc<AtomicUsize>);
    impl Drop for PanickingDrop {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("coordinator-owned capture panic");
        }
    }

    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("worker", |_| pending::<Result<ComponentExit, BoxError>>())
        .unwrap();
    let captures_dropped = Arc::new(AtomicUsize::new(0));
    let capture = PanickingDrop(captures_dropped.clone());
    let factory_calls = Arc::new(AtomicUsize::new(0));
    let in_factory = factory_calls.clone();
    supervisor
        .on_cleanup("dependency", move || {
            in_factory.fetch_add(1, Ordering::SeqCst);
            async move {
                drop(capture);
                Ok(())
            }
        })
        .unwrap();

    // The coordinator cannot poll until this current-thread test yields.
    let pending_owner = supervisor.start();
    assert_eq!(pending_owner.status().readiness(), Readiness::Starting);
    let observer = pending_owner.observer();
    let another_observer = observer.clone();
    drop(pending_owner);
    assert_eq!(captures_dropped.load(Ordering::SeqCst), 0);

    let error = tokio::time::timeout(Duration::from_secs(5), observer.wait())
        .await
        .expect("owner loss must not prevent coordinator failure publication")
        .unwrap_err();
    assert!(error.is_panic());
    assert_eq!(captures_dropped.load(Ordering::SeqCst), 1);
    // The capture panicked in the coordinator while skipping dependent cleanup,
    // rather than in a finalizer task whose failure would produce a report.
    assert_eq!(factory_calls.load(Ordering::SeqCst), 0);
    drop(observer);
    let retained = tokio::time::timeout(Duration::from_secs(5), another_observer.wait())
        .await
        .expect("the cloned observer must retain the coordinator failure")
        .unwrap_err();
    assert!(Arc::ptr_eq(&error, &retained));
    let ShutdownFailure::Coordinator(checked) = another_observer.wait_checked().await.unwrap_err()
    else {
        panic!("a coordinator failure cannot become successful evidence")
    };
    assert!(Arc::ptr_eq(&error, &checked));
}

#[test]
fn observer_created_after_completion_retains_report_after_owners_and_runtime_drop() {
    let owner_runtime = runtime();
    let (observer, report) = owner_runtime.block_on(async {
        let supervisor =
            Supervisor::with_process_capacity(budget(), ProcessCapacity::new(1).unwrap());
        let running = supervisor.start();
        let report = tokio::time::timeout(Duration::from_secs(5), running.shutdown())
            .await
            .expect("the driver must publish its report")
            .unwrap();
        assert!(report.is_success());
        assert_eq!(running.status().readiness(), Readiness::Stopped);

        // Construct a new observer from an owner clone after publication.
        let another_owner = running.clone();
        let observer = another_owner.observer();
        drop(another_owner);
        drop(running);
        (observer, report)
    });
    drop(owner_runtime);

    let retained = runtime().block_on(async {
        tokio::time::timeout(Duration::from_secs(5), observer.wait())
            .await
            .expect("the late observer must retain the published report")
            .unwrap()
    });
    assert!(std::ptr::eq(&*report, &*retained));
    let checked = runtime().block_on(observer.wait_checked()).unwrap();
    assert!(std::ptr::eq(&*report, &**checked.report()));
}

#[tokio::test(start_paused = true)]
async fn checked_waits_leave_a_running_process_active() {
    let mut supervisor = Supervisor::new(budget());
    let drained = Arc::new(AtomicUsize::new(0));
    let in_component = drained.clone();
    supervisor
        .register("worker", move |startup| async move {
            let shutdown = startup.acknowledge_started();
            shutdown.draining().await;
            in_component.fetch_add(1, Ordering::SeqCst);
            Ok(shutdown.stopped())
        })
        .unwrap();

    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    assert_eq!(running.status().readiness(), Readiness::Ready);
    let observer = running.observer();
    let mut owner_wait = Box::pin(running.wait_checked());
    let mut observer_wait = Box::pin(observer.wait_checked());
    poll_fn(|cx| {
        assert!(owner_wait.as_mut().poll(cx).is_pending());
        assert!(observer_wait.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(running.status().readiness(), Readiness::Ready);
    tokio::task::yield_now().await;
    assert_eq!(drained.load(Ordering::SeqCst), 0);

    running.handle().request();
    let success = owner_wait.await.unwrap();
    let observed = observer_wait.await.unwrap();
    assert!(success.report().is_success());
    assert!(std::ptr::eq(&**success.report(), &**observed.report()));
    assert_eq!(drained.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn unapproved_checked_completion_never_approves_readiness() {
    let mut supervisor = Supervisor::new(budget());
    let drained = Arc::new(AtomicUsize::new(0));
    let in_component = drained.clone();
    supervisor
        .register("worker", move |startup| async move {
            startup.shutdown().draining().await;
            in_component.fetch_add(1, Ordering::SeqCst);
            Ok(startup.abandon())
        })
        .unwrap();
    let pending = supervisor.start_unapproved();
    assert_eq!(pending.status().readiness(), Readiness::Starting);
    let observer = pending.observer();
    let mut owner_wait = Box::pin(pending.wait_checked());
    let mut observer_wait = Box::pin(observer.wait_checked());
    poll_fn(|cx| {
        assert!(owner_wait.as_mut().poll(cx).is_pending());
        assert!(observer_wait.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(pending.status().readiness(), Readiness::Starting);
    tokio::task::yield_now().await;
    assert_eq!(drained.load(Ordering::SeqCst), 0);

    let success = pending.shutdown_checked().await.unwrap();
    assert!(success.report().is_success());
    assert_eq!(pending.status().readiness(), Readiness::Stopped);
    let later = owner_wait.await.unwrap();
    let observed = observer_wait.await.unwrap();
    assert!(std::ptr::eq(&**success.report(), &**later.report()));
    assert!(std::ptr::eq(&**success.report(), &**observed.report()));
    assert_eq!(drained.load(Ordering::SeqCst), 1);
}

#[test]
#[should_panic(expected = "owned driver retains completion sender")]
fn observer_panics_when_owning_runtime_drops_before_publication() {
    let owner_runtime = runtime();
    let running = {
        let _entered = owner_runtime.enter();
        Supervisor::with_process_capacity(budget(), ProcessCapacity::new(1).unwrap()).start()
    };
    let observer = running.observer();
    // Entering a current-thread runtime does not drive its spawned futures.
    // No outcome can have been published before the runtime destroys its tasks.
    drop(owner_runtime);

    runtime().block_on(async {
        let _outcome = tokio::time::timeout(Duration::from_secs(5), observer.wait())
            .await
            .expect("a lost sender must resolve instead of hanging");
    });
    drop(running);
}
