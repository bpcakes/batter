use super::support::{Case, budget, yields};
use batter::lifecycle::{Readiness, Supervisor, TaskOutcome};
use std::sync::Arc;
use tokio::sync::{Barrier, oneshot};

pub async fn approvals(case: Case, order: usize, delay: u64) {
    let mut supervisor = Supervisor::with_process_capacity(budget(), 1).unwrap();
    let handle = supervisor.handle();
    let (first, first_rx) = oneshot::channel();
    let (first_ack, first_acked) = oneshot::channel();
    let (second, second_rx) = oneshot::channel();
    let (second_ack, second_acked) = oneshot::channel();
    let (release, released) = oneshot::channel();
    supervisor
        .register("first", move |signal| async move {
            first_rx.await.unwrap();
            yields(delay).await;
            assert!(signal.mark_started());
            assert!(!signal.mark_started());
            first_ack.send(()).unwrap();
            signal.draining().await;
            Ok(())
        })
        .unwrap();
    supervisor
        .register("second", move |signal| async move {
            second_rx.await.unwrap();
            yields(delay.rotate_left(7)).await;
            signal.mark_started();
            second_ack.send(()).unwrap();
            released.await.unwrap();
            signal.draining().await;
            Ok(())
        })
        .unwrap();
    if order == 0 {
        handle.mark_ready();
    }
    assert_eq!(handle.readiness(), Readiness::Starting);
    let running = supervisor.start();
    first.send(()).unwrap();
    first_acked.await.unwrap();
    assert_eq!(handle.readiness(), Readiness::Starting);
    if order == 2 {
        handle.request();
    }
    if order != 0 {
        assert_eq!(handle.mark_ready(), order != 2);
    }
    second.send(()).unwrap();
    second_acked.await.unwrap();
    if order != 2 {
        handle.wait_ready().await.unwrap();
        assert_eq!(handle.readiness(), Readiness::Ready);
        handle.request();
    }
    assert!(!handle.mark_ready());
    // Keep the acknowledged component alive: Stopped cannot hide an invalid
    // Ready publication after drain while the coordinator finishes its report.
    assert_eq!(handle.readiness(), Readiness::Draining);
    assert!(handle.wait_ready().await.is_err());
    release.send(()).unwrap();
    let report = running.wait().await.unwrap();
    assert!(report.is_success());
    assert_eq!(handle.readiness(), Readiness::Stopped);
    case.event("ordered-readiness-checked");
}

pub async fn acknowledgement_race(case: Case, delay: u64) {
    let mut supervisor = Supervisor::with_process_capacity(budget(), 1).unwrap();
    let handle = supervisor.handle();
    let barrier = Arc::new(Barrier::new(4));
    let component_barrier = barrier.clone();
    let (acked, ack) = oneshot::channel();
    let (release, released) = oneshot::channel();
    supervisor
        .register("racing-startup", move |signal| async move {
            component_barrier.wait().await;
            yields(delay).await;
            signal.mark_started();
            acked.send(()).unwrap();
            released.await.unwrap();
            signal.draining().await;
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    assert_eq!(handle.readiness(), Readiness::Starting);
    let request_barrier = barrier.clone();
    let requesting = handle.clone();
    let drain = tokio::spawn(async move {
        request_barrier.wait().await;
        yields(delay.rotate_left(9)).await;
        requesting.request();
    });
    let approval_barrier = barrier.clone();
    let approving = handle.clone();
    let approval = tokio::spawn(async move {
        approval_barrier.wait().await;
        yields(delay.rotate_left(21)).await;
        approving.mark_ready();
    });
    barrier.wait().await;
    drain.await.unwrap();
    ack.await.unwrap();
    approval.await.unwrap();
    case.event("drain-and-ack-returned");
    assert!(!handle.mark_ready());
    assert_eq!(handle.readiness(), Readiness::Draining);
    release.send(()).unwrap();
    assert!(running.wait().await.unwrap().is_success());
    assert_eq!(handle.readiness(), Readiness::Stopped);
}

pub async fn critical_exit(case: Case) {
    let mut supervisor = Supervisor::with_process_capacity(budget(), 1).unwrap();
    supervisor
        .register("uninitialized-exit", |_| async { Ok(()) })
        .unwrap();
    let handle = supervisor.handle();
    handle.mark_ready();
    let running = supervisor.start();
    assert!(handle.wait_ready().await.is_err());
    let report = running.wait().await.unwrap();
    assert!(!report.is_success());
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].outcome, TaskOutcome::UnexpectedExit);
    assert!(!handle.mark_ready());
    case.event("critical-exit-without-ready");
}
