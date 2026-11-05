use super::*;

struct DestructionPanic;
impl Drop for DestructionPanic {
    fn drop(&mut self) {
        panic!("destruction secret");
    }
}

#[tokio::test(start_paused = true)]
async fn work_destruction_panic_is_visible_to_finalization_without_losing_result() {
    let parent = parent();
    let running = PhasedOperation::new(
        parent.context(),
        reserve(),
        async |_work| {
            let _guard = DestructionPanic;
            pending::<Result<(), Infallible>>().await
        },
        async |scope| {
            assert!(scope.work().destruction_panic().is_some());
            assert!(matches!(
                scope.work().result(),
                Err(CommandCause::Interrupted(Interruption::DeadlineExceeded))
            ));
            Ok::<_, Infallible>(())
        },
    )
    .unwrap()
    .start();
    let report = retained(running.wait().await);
    assert!(report.work().destruction_panic().is_some());
    assert!(report.finalization().is_success());
}

#[tokio::test(start_paused = true)]
async fn dropping_owner_during_finalization_does_not_cancel_it() {
    let parent = parent();
    let (started, ready) = tokio::sync::oneshot::channel();
    let release = Arc::new(Notify::new());
    let wait = release.clone();
    let running = PhasedOperation::new(
        parent.context(),
        reserve(),
        async |_work| Ok::<_, Infallible>(7),
        async move |scope| {
            started.send(()).unwrap();
            wait.notified().await;
            scope.context().check().unwrap();
            Ok::<_, Infallible>(9)
        },
    )
    .unwrap()
    .start();
    let observer = running.observer();
    ready.await.unwrap();
    drop(running);
    release.notify_one();
    let report = retained(observer.wait().await);
    assert!(
        report.is_success(),
        "work cancellation after completion cannot reclassify it"
    );
}

#[tokio::test(start_paused = true)]
async fn dropping_owner_during_work_waits_for_destruction_before_finalization() {
    let parent = parent();
    let (started, ready) = tokio::sync::oneshot::channel();
    let destroyed = Arc::new(AtomicBool::new(false));
    let mark = destroyed.clone();
    let running = PhasedOperation::new(
        parent.context(),
        reserve(),
        async move |_work| {
            let _guard = Destroyed(mark);
            started.send(()).unwrap();
            pending::<Result<(), Infallible>>().await
        },
        async move |scope| {
            assert!(destroyed.load(Ordering::SeqCst));
            assert!(matches!(
                scope.work().result(),
                Err(CommandCause::Interrupted(Interruption::Cancelled))
            ));
            Ok::<_, Infallible>(())
        },
    )
    .unwrap()
    .start();
    let observer = running.observer();
    ready.await.unwrap();
    drop(running);
    assert!(retained(observer.wait().await).finalization().is_success());
}

#[tokio::test(start_paused = true)]
async fn ready_return_after_parent_cancel_is_retained_alongside_interruption() {
    let parent = parent();
    let context = parent.context().clone();
    let running = PhasedOperation::new(
        &context,
        reserve(),
        async move |_work| {
            parent.cancel();
            Ok::<_, Infallible>(42)
        },
        async |_scope| Ok::<_, Infallible>(()),
    )
    .unwrap()
    .start();
    let report = retained(running.wait().await);
    assert_eq!(report.work().result().ok(), Some(&42));
    assert_eq!(
        report.work().interruption_after_return(),
        Some(Interruption::Cancelled)
    );
    assert!(!report.is_success());
    assert!(matches!(
        report.finalization().result(),
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
}
