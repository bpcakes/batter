use super::support::{Case, budget, supervisor, yields};
use batter::{
    BoxError,
    lifecycle::{ProcessAdmissionError, Readiness, Supervisor},
};
use std::{
    convert::Infallible,
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::oneshot;

pub async fn receipt_waiter(case: Case, delay: u64) {
    let supervisor = supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let (started, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let receipt = process
        .try_spawn("receipt-independent", |_| async move {
            started.send(()).unwrap();
            released.await.unwrap();
            Ok::<_, Infallible>(29)
        })
        .unwrap();
    let waiter = tokio::spawn(receipt.wait());
    ready.await.unwrap();
    yields(delay).await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert!(matches!(
        process.try_spawn("still-full", |_| async { Ok::<_, Infallible>(()) }),
        Err(ProcessAdmissionError::Full)
    ));
    release.send(()).unwrap();
    let report = running.shutdown().await.unwrap();
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, 1);
    case.event("cancelled-receipt-retained-work-and-capacity");
}

pub async fn cleanup_waiter(case: Case, delay: u64) {
    let mut supervisor = supervisor(1);
    let (started, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let finished = Arc::new(AtomicBool::new(false));
    let flag = finished.clone();
    supervisor
        .on_cleanup("owned-finalizer", move || async move {
            started.send(()).unwrap();
            released.await.unwrap();
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let handle = running.handle();
    let observer = running.observer();
    let last_owner = running.clone();
    drop(running);
    assert!(!handle.is_draining());
    let waiter = tokio::spawn(async move { last_owner.shutdown().await });
    ready.await.unwrap();
    yields(delay).await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert!(!finished.load(Ordering::SeqCst));
    release.send(()).unwrap();
    let first = observer.wait().await.unwrap();
    let second = observer.clone().wait().await.unwrap();
    assert!(std::ptr::eq(&*first, &*second));
    assert!(first.is_success());
    assert_eq!(first.cleanup.records.len(), 1);
    assert!(finished.load(Ordering::SeqCst));
    assert_eq!(handle.readiness(), Readiness::Stopped);
    case.event("cancelled-shutdown-waiter-retained-cleanup-and-report");
}

pub async fn last_owner(case: Case, delay: u64) {
    let supervisor = supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let (started, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let receipt = process
        .try_spawn("last-owner-work", |scope| async move {
            started.send(()).unwrap();
            scope.signal().draining().await;
            assert!(!scope.signal().is_cancelled());
            released.await.unwrap();
            Ok::<_, Infallible>(31)
        })
        .unwrap();
    ready.await.unwrap();
    let observer = running.observer();
    let handle = running.handle();
    yields(delay).await;
    drop(running);
    assert!(handle.is_draining());
    release.send(()).unwrap();
    assert_eq!(receipt.wait().await.unwrap(), 31);
    let report = observer.wait().await.unwrap();
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, 1);
    case.event("last-owner-drop-drained-owned-work");
}

struct DropSignal(Option<oneshot::Sender<()>>);
impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

pub async fn caller_owned(case: Case, polled: bool) {
    let mut supervisor = Supervisor::new(budget());
    let handle = supervisor.handle();
    let token = handle.operation_token();
    let (started, ready) = oneshot::channel();
    let (dropped, gone) = oneshot::channel();
    let invoked = Arc::new(AtomicBool::new(false));
    let flag = invoked.clone();
    supervisor
        .register("caller-owned-task", move |_| {
            flag.store(true, Ordering::SeqCst);
            async move {
                let _guard = DropSignal(Some(dropped));
                started.send(()).unwrap();
                pending::<Result<(), BoxError>>().await
            }
        })
        .unwrap();
    let cleanup = Arc::new(AtomicBool::new(false));
    let flag = cleanup.clone();
    supervisor
        .on_cleanup("unawaited-finalizer", move || async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let driver = supervisor.run_until(pending());
    if polled {
        let driver = tokio::spawn(driver);
        ready.await.unwrap();
        driver.abort();
        assert!(driver.await.unwrap_err().is_cancelled());
        gone.await.unwrap();
    } else {
        drop(driver);
        assert!(!invoked.load(Ordering::SeqCst));
    }
    assert!(token.is_cancelled());
    assert!(handle.is_draining());
    assert!(!cleanup.load(Ordering::SeqCst));
    case.event("caller-owned-drop-not-clean-shutdown");
}
