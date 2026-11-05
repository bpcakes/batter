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
            assert!(
                scope
                    .work()
                    .classify()
                    .unwrap_err()
                    .destruction_panic()
                    .is_some()
            );
            assert!(matches!(
                scope.work().classify().unwrap_err().returned_result(),
                Err(CommandCause::Interrupted(Interruption::DeadlineExceeded))
            ));
            Ok::<_, Infallible>(())
        },
    )
    .unwrap()
    .start();
    let report = retained(running.wait().await);
    assert!(
        report
            .work()
            .classify()
            .unwrap_err()
            .destruction_panic()
            .is_some()
    );
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
    let success = observer
        .wait()
        .await
        .expect("owner loss cannot reclassify completed work");
    assert_eq!((*success.work(), *success.finalization()), (7, 9));
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
                scope.work().classify().unwrap_err().returned_result(),
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
    assert_eq!(
        report.work().classify().unwrap_err().returned_result().ok(),
        Some(&42)
    );
    assert_eq!(
        report
            .work()
            .classify()
            .unwrap_err()
            .interruption_after_return(),
        Some(Interruption::Cancelled)
    );
    assert!(report.work().classify().is_err());
    assert!(matches!(
        report
            .finalization()
            .classify()
            .unwrap_err()
            .returned_result(),
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
}

#[tokio::test(start_paused = true)]
async fn zero_reserve_preserves_original_deadline_and_skips_expired_finalization() {
    let parent = parent();
    let deadline = parent.context().deadline();
    let invoked = Arc::new(AtomicBool::new(false));
    let mark = invoked.clone();
    let phases = parent.context().split_finalization().unwrap();
    // Starting later must not reset the absolute native-derived deadline.
    tokio::time::advance(Duration::from_secs(1)).await;
    let running = phases
        .into_operation(
            async move |work| {
                assert_eq!(work.deadline(), deadline);
                pending::<Result<(), Infallible>>().await
            },
            async move |_scope| {
                mark.store(true, Ordering::SeqCst);
                Ok::<_, Infallible>(())
            },
        )
        .start();
    let report = retained(running.wait().await);
    for phase in [report.work(), report.finalization()] {
        assert!(matches!(
            phase.classify().unwrap_err().returned_result(),
            Err(CommandCause::Interrupted(Interruption::DeadlineExceeded))
        ));
    }
    assert!(!invoked.load(Ordering::SeqCst));
    assert_eq!(tokio::time::Instant::now(), deadline);
}

struct ReturnedThenPanics(u8);
impl std::future::Future for ReturnedThenPanics {
    type Output = Result<u8, Infallible>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::task::Poll::Ready(Ok(self.0))
    }
}
impl Drop for ReturnedThenPanics {
    fn drop(&mut self) {
        panic!("returned phase destruction secret");
    }
}

#[tokio::test]
async fn returned_values_with_destruction_panics_cannot_be_accepted_as_success() {
    let parent = parent();
    let running = PhasedOperation::new(
        parent.context(),
        reserve(),
        |_scope: &mut batter_core::operation::OperationContext| ReturnedThenPanics(7),
        |scope: &mut batter_core::operation::FinalizationScope<u8, Infallible>| {
            let failed = scope
                .work()
                .classify()
                .expect_err("destruction invalidated returned work");
            assert_eq!(failed.returned_result().ok(), Some(&7));
            assert!(failed.destruction_panic().is_some());
            ReturnedThenPanics(9)
        },
    )
    .unwrap()
    .start();
    let report = retained(running.wait().await);
    for (phase, expected) in [(report.work(), 7), (report.finalization(), 9)] {
        let failed = phase.classify().unwrap_err();
        assert_eq!(failed.returned_result().ok(), Some(&expected));
        assert!(failed.destruction_panic().is_some());
    }
    assert!(!format!("{report:?}").contains("secret"));
}
