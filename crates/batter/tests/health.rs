use batter::{
    ConfigurationError, RegistrationError,
    cleanup::CleanupBudget,
    health::{HealthMonitor, HealthPolicy, HealthReader, HealthStatus, ProbeOutcome},
    lifecycle::{Readiness, ShutdownBudget, ShutdownHandle, Supervisor},
    operation::OperationContext,
    startup::Startup,
};
use std::{
    convert::Infallible,
    future::{Future, pending, poll_fn},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
    time::Duration,
};
use tokio::{
    sync::{Barrier, oneshot},
    time::{advance, timeout},
};

#[path = "health/ownership.rs"]
mod ownership;
#[path = "health/publication.rs"]
mod publication;

fn policy() -> HealthPolicy {
    HealthPolicy::new(
        Duration::from_secs(2),
        Duration::from_secs(3),
        Duration::from_secs(6),
        Duration::from_secs(1),
    )
    .unwrap()
}
fn supervisor() -> Supervisor {
    let second = Duration::from_secs(1);
    Supervisor::new(
        ShutdownBudget::new(
            second,
            second,
            second,
            CleanupBudget::new(second, second, second).unwrap(),
        )
        .unwrap(),
    )
}
async fn poll_once<F: Future>(mut future: Pin<&mut F>) -> Poll<F::Output> {
    poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await
}
async fn wait_status<E>(reader: &HealthReader<E>, status: HealthStatus) {
    timeout(Duration::from_secs(2), async {
        while reader.snapshot().status() != status {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("health state reached within the watchdog");
}

struct Dropped(Arc<AtomicUsize>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn rejected_registration_drops_the_inert_writer_without_invoking_its_probe() {
    let calls = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let mut supervisor = supervisor();
    supervisor
        .register("dependency.health", |_| async { Ok(()) })
        .unwrap();
    let called = calls.clone();
    let captured = Dropped(dropped.clone());
    let monitor = HealthMonitor::new(policy(), move || {
        let _ = &captured;
        called.fetch_add(1, Ordering::SeqCst);
        async { Ok::<_, Infallible>(()) }
    });

    let error = match monitor.register_in(&mut supervisor, "dependency.health") {
        Ok(_) => panic!("duplicate health registration unexpectedly succeeded"),
        Err(error) => error,
    };

    assert_eq!(error, RegistrationError::Duplicate("dependency.health"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn protected_registration_owns_readiness_sampling_and_reader_shutdown() {
    let (reader_tx, reader_rx) = oneshot::channel();
    let mut starting = Startup::scoped(
        supervisor(),
        OperationContext::new(Duration::from_secs(2)).unwrap(),
        CleanupBudget::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap(),
        move |scope| {
            Box::pin(async move {
                let reader = HealthMonitor::new(policy(), || async { Ok::<_, Infallible>(()) })
                    .register_in(scope, "dependency.health")?;
                assert!(reader_tx.send(reader).is_ok());
                Ok::<_, RegistrationError>(())
            })
        },
    )
    .start();

    let running = starting.wait().await.unwrap();
    let reader = reader_rx.await.unwrap();
    running.handle().wait_ready().await.unwrap();
    wait_status(&reader, HealthStatus::Healthy).await;
    assert_eq!(running.handle().readiness(), Readiness::Ready);

    let report = running.shutdown().await.unwrap();
    assert!(report.is_success(), "{report}");
    assert_eq!(reader.snapshot().status(), HealthStatus::Stopped);
}

#[test]
fn timing_policy_rejects_zero_overflow_and_insufficient_freshness() {
    let second = Duration::from_secs(1);
    for index in 0..4 {
        let mut values = [second, second, Duration::from_secs(3), second];
        values[index] = Duration::ZERO;
        assert!(matches!(
            HealthPolicy::new(values[0], values[1], values[2], values[3]),
            Err(ConfigurationError::Zero(_))
        ));
        values[index] = Duration::MAX;
        assert!(matches!(
            HealthPolicy::new(values[0], values[1], values[2], values[3]),
            Err(ConfigurationError::TooLarge(_))
        ));
    }
    assert_eq!(
        HealthPolicy::new(second, second, second, second).unwrap_err(),
        ConfigurationError::InvalidHealthTiming
    );
    let year = Duration::from_secs(365 * 24 * 60 * 60);
    assert!(matches!(
        HealthPolicy::new(year, second, year, second),
        Err(ConfigurationError::TooLarge(_))
    ));
    let valid = HealthPolicy::new(second, second, Duration::from_secs(3), second).unwrap();
    assert_eq!(valid.probe_budget(), second);
    assert_eq!(valid.probe_delay(), second);
    assert_eq!(valid.maximum_age(), Duration::from_secs(3));
    assert_eq!(valid.scheduling_margin(), second);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_thousand_concurrent_reads_start_no_additional_probes() {
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    let policy = HealthPolicy::new(
        Duration::from_secs(1),
        Duration::from_secs(3600),
        Duration::from_secs(3602),
        Duration::from_secs(1),
    )
    .unwrap();
    let monitor = HealthMonitor::new(policy, move || {
        called.fetch_add(1, Ordering::SeqCst);
        async { Ok::<_, Infallible>(()) }
    });
    let reader = monitor.reader();
    let handle = ShutdownHandle::new();
    let running = tokio::spawn(monitor.run(handle.signal()));
    wait_status(&reader, HealthStatus::Healthy).await;
    let completed_at = reader.snapshot().last_probe().unwrap().completed_at();
    let barrier = Arc::new(Barrier::new(2001));
    let mut readers = tokio::task::JoinSet::new();
    for _ in 0..2000 {
        let reader = reader.clone();
        let barrier = barrier.clone();
        readers.spawn(async move {
            barrier.wait().await;
            let snapshot = reader.snapshot();
            assert!(snapshot.is_healthy());
            assert_eq!(snapshot.last_probe().unwrap().completed_at(), completed_at);
        });
    }
    timeout(Duration::from_secs(5), async {
        barrier.wait().await;
        while let Some(result) = readers.join_next().await {
            result.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    handle.request();
    timeout(Duration::from_secs(2), running)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reader.snapshot().status(), HealthStatus::Stopped);
}

#[tokio::test(start_paused = true)]
async fn a_stalled_owner_expires_at_the_boundary_and_never_catches_up() {
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    let monitor = HealthMonitor::new(policy(), move || {
        called.fetch_add(1, Ordering::SeqCst);
        async { Ok::<_, Infallible>(()) }
    });
    let reader = monitor.reader();
    let handle = ShutdownHandle::new();
    let mut run = Box::pin(monitor.run(handle.signal()));
    assert!(poll_once(run.as_mut()).await.is_pending());
    let first = reader.snapshot();
    assert!(first.is_healthy());
    advance(Duration::from_secs(6) - Duration::from_nanos(1)).await;
    assert!(reader.is_healthy());
    advance(Duration::from_nanos(1)).await;
    assert_eq!(reader.snapshot().status(), HealthStatus::Stale);
    assert!(!reader.is_healthy());
    advance(Duration::from_secs(100)).await;
    assert_eq!(
        reader.snapshot().last_probe().unwrap().completed_at(),
        first.last_probe().unwrap().completed_at()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // One late observation of the elapsed delay permits exactly one new probe.
    assert!(poll_once(run.as_mut()).await.is_pending());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(reader.is_healthy());
    assert!(poll_once(run.as_mut()).await.is_pending());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    handle.request();
    run.await;
}

struct Active {
    active: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}
impl Drop for Active {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn timeout_covers_acquisition_plus_query_and_probes_never_overlap() {
    let calls = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let queries = Arc::new(AtomicUsize::new(0));
    let monitor = HealthMonitor::new(policy(), {
        let (calls, active, dropped, queries) = (
            calls.clone(),
            active.clone(),
            dropped.clone(),
            queries.clone(),
        );
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                active.fetch_add(1, Ordering::SeqCst),
                0,
                "overlapping probes"
            );
            let guard = Active {
                active: active.clone(),
                dropped: dropped.clone(),
            };
            let queries = queries.clone();
            async move {
                let _guard = guard;
                tokio::time::sleep(Duration::from_millis(1500)).await; // acquisition
                queries.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(1500)).await; // query
                Ok::<_, Infallible>(())
            }
        }
    });
    let reader = monitor.reader();
    let handle = ShutdownHandle::new();
    let mut run = Box::pin(monitor.run(handle.signal()));
    assert!(poll_once(run.as_mut()).await.is_pending());
    assert_eq!(reader.snapshot().status(), HealthStatus::Unknown);
    advance(Duration::from_millis(1500)).await;
    assert!(poll_once(run.as_mut()).await.is_pending());
    assert_eq!(queries.load(Ordering::SeqCst), 1);
    advance(Duration::from_millis(500)).await;
    assert!(poll_once(run.as_mut()).await.is_pending());
    assert_eq!(reader.snapshot().status(), HealthStatus::TimedOut);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    advance(Duration::from_millis(2999)).await;
    assert!(poll_once(run.as_mut()).await.is_pending());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    advance(Duration::from_millis(1)).await;
    assert!(poll_once(run.as_mut()).await.is_pending());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(active.load(Ordering::SeqCst), 1);
    handle.request();
    run.await;
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(dropped.load(Ordering::SeqCst), 2);
}

struct PrivateFailure(u32);
impl std::fmt::Debug for PrivateFailure {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        panic!("must not format cause")
    }
}

#[tokio::test(start_paused = true)]
async fn concrete_failures_and_timeouts_recover_without_draining_the_supervisor() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let called = attempts.clone();
    let monitor = HealthMonitor::new(policy(), move || {
        let attempt = called.fetch_add(1, Ordering::SeqCst) + 1;
        async move {
            match attempt {
                1 => Err(PrivateFailure(42)),
                2 => pending().await,
                _ => Ok(()),
            }
        }
    });
    let reader = monitor.reader();
    let mut base = supervisor();
    let handle = base.handle();
    base.register("health", move |shutdown| async move {
        monitor.run(shutdown).await;
        Ok(())
    })
    .unwrap();
    handle.mark_ready();
    let running = base.start();
    handle.wait_ready().await.unwrap();
    wait_status(&reader, HealthStatus::Failed).await;
    let failed = reader.snapshot();
    let ProbeOutcome::Failed(cause) = failed.last_probe().unwrap().outcome() else {
        panic!("expected concrete failure")
    };
    assert_eq!(cause.0, 42);
    let again = reader.snapshot();
    let ProbeOutcome::Failed(same) = again.last_probe().unwrap().outcome() else {
        panic!("expected retained failure")
    };
    assert!(Arc::ptr_eq(cause, same));
    assert!(format!("{failed:?}").contains("cause retained"));
    assert_eq!(handle.readiness(), Readiness::Ready);
    advance(Duration::from_secs(3)).await;
    timeout(Duration::from_secs(2), async {
        while attempts.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    advance(Duration::from_secs(2)).await;
    wait_status(&reader, HealthStatus::TimedOut).await;
    assert_eq!(handle.readiness(), Readiness::Ready);
    advance(Duration::from_secs(3)).await;
    wait_status(&reader, HealthStatus::Healthy).await;
    assert!(running.shutdown().await.unwrap().is_success());
    assert_eq!(reader.snapshot().status(), HealthStatus::Stopped);
}

struct SlowPoll(bool);
impl Future for SlowPoll {
    type Output = Result<(), Infallible>;
    fn poll(self: Pin<&mut Self>, _: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        if self.0 {
            std::thread::sleep(Duration::from_millis(20));
        }
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn construction_and_polling_overruns_cannot_publish_healthy_results() {
    for slow_factory in [false, true] {
        let policy = HealthPolicy::new(
            Duration::from_millis(1),
            Duration::from_secs(60),
            Duration::from_secs(62),
            Duration::from_secs(1),
        )
        .unwrap();
        let monitor = HealthMonitor::new(policy, move || {
            if slow_factory {
                std::thread::sleep(Duration::from_millis(20));
            }
            SlowPoll(!slow_factory)
        });
        let reader = monitor.reader();
        let handle = ShutdownHandle::new();
        let task = tokio::spawn(monitor.run(handle.signal()));
        wait_status(&reader, HealthStatus::TimedOut).await;
        assert!(!reader.is_healthy());
        handle.request();
        task.await.unwrap();
    }
}
