use super::support::{Case, supervisor};
use batter::lifecycle::ProcessAdmissionError;
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::oneshot;

pub async fn capacity_one(case: Case, drain: bool) {
    let supervisor = supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let (scope_tx, scope_rx) = oneshot::channel();
    let (finish_tx, finish_rx) = oneshot::channel();
    let parent = process
        .try_spawn("ancestor", |scope| async move {
            scope_tx.send(scope).ok().unwrap();
            finish_rx.await.unwrap();
            Ok::<_, Infallible>(())
        })
        .unwrap();
    let scope = scope_rx.await.unwrap();
    case.event("ancestor-holds-only-permit");
    if drain {
        running.handle().request();
        case.event("drain-returned");
    }
    let invoked = Arc::new(AtomicBool::new(false));
    let in_factory = invoked.clone();
    case.event("descendant-submit");
    let rejected = scope.try_spawn("descendant", move |_| {
        in_factory.store(true, Ordering::SeqCst);
        async { Ok::<_, Infallible>(()) }
    });
    assert!(
        matches!(rejected, Err(ProcessAdmissionError::Full)),
        "shared capacity violated: descendant must be Full while ancestor owns the only permit"
    );
    assert!(!invoked.load(Ordering::SeqCst), "rejected factory invoked");
    case.event("descendant-full");
    finish_tx.send(()).unwrap();
    parent.wait().await.unwrap();
    assert!(matches!(
        scope.try_spawn("expired", |_| async { Ok::<_, Infallible>(()) }),
        Err(ProcessAdmissionError::Closed)
    ));
    let report = running.shutdown().await.unwrap();
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, 1);
}
