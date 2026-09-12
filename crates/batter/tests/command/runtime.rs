use super::*;

#[test]
fn published_report_survives_its_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let observer = runtime.block_on(async {
        let command = Command::new(context(), budget(), |_| {
            Box::pin(async { Ok::<_, Infallible>(42) })
        })
        .start();
        assert!(command.wait().await.unwrap().is_success());
        command.observer()
    });
    drop(runtime);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let report = runtime.block_on(observer.wait()).unwrap();
    assert_eq!(*report.work.as_ref().unwrap(), 42);
}

#[test]
fn runtime_death_before_publication_cannot_report_completed_cleanup() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (command, observer) = runtime.block_on(async {
        let (started, starting) = oneshot::channel();
        let command = Command::new(context(), budget(), |_| {
            Box::pin(async move {
                started.send(()).unwrap();
                pending::<()>().await;
                Ok::<_, Infallible>(())
            })
        })
        .start();
        starting.await.unwrap();
        let observer = command.observer();
        (command, observer)
    });
    drop(runtime);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let observed =
        runtime.block_on(async move { tokio::spawn(async move { observer.wait().await }).await });
    assert!(observed.unwrap_err().is_panic());
    drop(command);
}

#[tokio::test(start_paused = true)]
async fn final_poll_cancellation_keeps_returned_value_without_false_success() {
    let command = Command::new(context(), budget(), |scope| {
        Box::pin(async move {
            scope.context().cancel();
            Ok::<_, Infallible>(42)
        })
    })
    .start();
    let report = command.wait().await.unwrap();
    assert_eq!(*report.work.as_ref().unwrap(), 42);
    assert_eq!(
        report.interruption_after_work,
        Some(Interruption::Cancelled)
    );
    assert!(!report.is_success());
}
