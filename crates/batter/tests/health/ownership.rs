use super::*;

#[tokio::test]
async fn construction_and_unpolled_run_are_inert_and_writer_loss_is_immediate() {
    let calls = Arc::new(AtomicUsize::new(0));
    for start in [false, true] {
        let called = calls.clone();
        let monitor = HealthMonitor::new(policy(), move || {
            called.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, Infallible>(()) }
        });
        let reader = monitor.reader();
        assert_eq!(reader.snapshot().status(), HealthStatus::Unknown);
        assert!(!reader.is_healthy());
        if start {
            drop(monitor.run(ShutdownHandle::new().signal()));
        } else {
            drop(monitor);
        }
        assert_eq!(reader.snapshot().status(), HealthStatus::Stopped);
        assert!(reader.snapshot().last_probe().is_none());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

struct EventOnDrop(Arc<Mutex<Vec<&'static str>>>, &'static str);
impl Drop for EventOnDrop {
    fn drop(&mut self) {
        self.0.lock().unwrap().push(self.1);
    }
}

#[tokio::test]
async fn drain_destroys_active_probe_before_monitor_return_and_dependency_cleanup() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = oneshot::channel();
    let mut entered = Some(entered_tx);
    let probe_events = events.clone();
    let monitor = HealthMonitor::new(policy(), move || {
        let entered = entered.take().expect("only one admitted probe");
        let guard = EventOnDrop(probe_events.clone(), "probe-dropped");
        async move {
            let _guard = guard;
            entered.send(()).unwrap();
            pending::<Result<(), Infallible>>().await
        }
    });
    let reader = monitor.reader();
    let mut base = supervisor();
    let completed = events.clone();
    base.register("health", move |shutdown| async move {
        monitor.run(shutdown).await;
        completed.lock().unwrap().push("monitor-returned");
        Ok(())
    })
    .unwrap();
    let cleanup = events.clone();
    base.on_cleanup("dependency", move || async move {
        cleanup.lock().unwrap().push("dependency-cleanup");
        Ok(())
    })
    .unwrap();
    let running = base.start();
    timeout(Duration::from_secs(2), entered_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(running.shutdown().await.unwrap().is_success());
    assert_eq!(
        *events.lock().unwrap(),
        ["probe-dropped", "monitor-returned", "dependency-cleanup"]
    );
    assert_eq!(reader.snapshot().status(), HealthStatus::Stopped);
}

#[tokio::test(start_paused = true)]
async fn drain_during_delay_stops_before_another_probe_and_overrides_cached_success() {
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    let monitor = HealthMonitor::new(policy(), move || {
        called.fetch_add(1, Ordering::SeqCst);
        async { Ok::<_, Infallible>(()) }
    });
    let reader = monitor.reader();
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    let mut run = Box::pin(monitor.run(handle.signal()));
    assert!(poll_once(run.as_mut()).await.is_pending());
    assert!(reader.is_healthy());
    handle.request();
    // Even before the monitor is rescheduled, lifecycle state forbids readiness.
    assert_ne!(handle.readiness(), Readiness::Ready);
    let saved = reader.snapshot();
    run.await;
    assert!(saved.is_healthy()); // explicitly historical, never self-updating
    assert_eq!(reader.snapshot().status(), HealthStatus::Stopped);
    assert!(matches!(
        reader.snapshot().last_probe().unwrap().outcome(),
        ProbeOutcome::Succeeded
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

struct DrainOnDrop(ShutdownHandle);
impl Future for DrainOnDrop {
    type Output = Result<(), Infallible>;
    fn poll(self: Pin<&mut Self>, _: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(()))
    }
}
impl Drop for DrainOnDrop {
    fn drop(&mut self) {
        self.0.request();
    }
}

#[tokio::test]
async fn drain_immediately_before_publication_cannot_publish_success() {
    let handle = ShutdownHandle::new();
    let request = handle.clone();
    let monitor = HealthMonitor::new(policy(), move || DrainOnDrop(request.clone()));
    let reader = monitor.reader();
    monitor.run(handle.signal()).await;
    let snapshot = reader.snapshot();
    assert_eq!(snapshot.status(), HealthStatus::Stopped);
    assert!(
        snapshot.last_probe().is_none(),
        "late completion must not publish"
    );
}

#[tokio::test]
async fn forced_cancellation_destroys_the_direct_probe_before_completion() {
    let base = supervisor();
    let signal = base.handle().signal();
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let monitor = HealthMonitor::new(policy(), move || {
        let guard = EventOnDrop(captured.clone(), "probe-dropped");
        async move {
            let _guard = guard;
            pending::<Result<(), Infallible>>().await
        }
    });
    let reader = monitor.reader();
    let mut run = Box::pin(monitor.run(signal.clone()));
    assert!(poll_once(run.as_mut()).await.is_pending());
    // Abandoning the unstarted supervisor synchronously signals forced
    // cancellation as well as drain, before the parked monitor is polled again.
    drop(base);
    assert!(signal.is_cancelled());
    run.await;
    assert_eq!(*events.lock().unwrap(), ["probe-dropped"]);
    assert_eq!(reader.snapshot().status(), HealthStatus::Stopped);
}

#[tokio::test]
async fn outer_task_abort_drops_active_probe_and_invalidates_readers() {
    let (entered_tx, entered_rx) = oneshot::channel();
    let mut entered = Some(entered_tx);
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let monitor = HealthMonitor::new(policy(), move || {
        let entered = entered.take().unwrap();
        let guard = EventOnDrop(captured.clone(), "probe-dropped");
        async move {
            let _guard = guard;
            entered.send(()).unwrap();
            pending::<Result<(), Infallible>>().await
        }
    });
    let reader = monitor.reader();
    let task = tokio::spawn(monitor.run(ShutdownHandle::new().signal()));
    entered_rx.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(*events.lock().unwrap(), ["probe-dropped"]);
    assert_eq!(reader.snapshot().status(), HealthStatus::Stopped);
}

#[tokio::test]
async fn probe_panic_remains_a_critical_failure_and_stops_the_writer() {
    let monitor = HealthMonitor::new(policy(), || async {
        panic!("deliberate probe panic");
        #[allow(unreachable_code)]
        Ok::<_, Infallible>(())
    });
    let reader = monitor.reader();
    let mut base = supervisor();
    base.register("health", move |shutdown| async move {
        monitor.run(shutdown).await;
        Ok(())
    })
    .unwrap();
    let report = base.start().wait().await.unwrap();
    assert!(!report.is_success());
    assert_eq!(
        report.tasks[0].outcome,
        batter::lifecycle::TaskOutcome::Panicked
    );
    assert_eq!(reader.snapshot().status(), HealthStatus::Stopped);
}

#[tokio::test]
async fn already_draining_run_never_invokes_its_factory() {
    let handle = ShutdownHandle::new();
    handle.request();
    let monitor = HealthMonitor::new(policy(), || {
        panic!("a drained monitor must not construct a probe");
        #[allow(unreachable_code)]
        async {
            Ok::<_, Infallible>(())
        }
    });
    let reader = monitor.reader();
    monitor.run(handle.signal()).await;
    assert_eq!(reader.snapshot().status(), HealthStatus::Stopped);
    assert!(reader.snapshot().last_probe().is_none());
}
