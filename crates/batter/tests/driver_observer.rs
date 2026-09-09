use batter::{
    BoxError,
    cleanup::CleanupBudget,
    lifecycle::{Readiness, ShutdownBudget, Supervisor},
};
use std::{
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
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
        .register("worker", move |signal| async move {
            in_component.fetch_add(1, Ordering::SeqCst);
            signal.draining().await;
            Ok(())
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
    let running = supervisor.start();
    assert_eq!(running.handle().readiness(), Readiness::Starting);
    let observer = running.observer();
    let another_observer = observer.clone();
    drop(running);
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
    assert!(Arc::ptr_eq(&report, &retained));
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
        .register("worker", |_| pending::<Result<(), BoxError>>())
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
    let running = supervisor.start();
    assert_eq!(running.handle().readiness(), Readiness::Starting);
    let observer = running.observer();
    let another_observer = observer.clone();
    drop(running);
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
}

#[test]
fn observer_created_after_completion_retains_report_after_owners_and_runtime_drop() {
    let owner_runtime = runtime();
    let (observer, report) = owner_runtime.block_on(async {
        let supervisor = Supervisor::with_process_capacity(budget(), 1).unwrap();
        let running = supervisor.start();
        let report = tokio::time::timeout(Duration::from_secs(5), running.shutdown())
            .await
            .expect("the driver must publish its report")
            .unwrap();
        assert!(report.is_success());
        assert_eq!(running.handle().readiness(), Readiness::Stopped);

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
    assert!(Arc::ptr_eq(&report, &retained));
}

#[test]
#[should_panic(expected = "owned driver retains completion sender")]
fn observer_panics_when_owning_runtime_drops_before_publication() {
    let owner_runtime = runtime();
    let running = {
        let _entered = owner_runtime.enter();
        Supervisor::with_process_capacity(budget(), 1)
            .unwrap()
            .start()
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
