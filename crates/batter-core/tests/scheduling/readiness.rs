use super::support::{Case, budget, yields};
use batter_core::lifecycle::{ProcessCapacity, Readiness, Supervisor, TaskOutcome};
use std::sync::Arc;
use tokio::sync::{Barrier, oneshot};

pub async fn approvals(case: Case, order: usize, delay: u64) {
    let mut supervisor =
        Supervisor::with_process_capacity(budget(), ProcessCapacity::new(1).unwrap());
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
            let signal = signal.acknowledge_started();
            first_ack.send(()).unwrap();
            signal.draining().await;
            Ok(signal.stopped())
        })
        .unwrap();
    supervisor
        .register("second", move |signal| async move {
            second_rx.await.unwrap();
            yields(delay.rotate_left(7)).await;
            let signal = signal.acknowledge_started();
            second_ack.send(()).unwrap();
            released.await.unwrap();
            signal.draining().await;
            Ok(signal.stopped())
        })
        .unwrap();
    assert_eq!(handle.status().readiness(), Readiness::Starting);
    let mut pending = Some(supervisor.start_unapproved());
    let mut running = if order == 0 {
        Some(pending.take().unwrap().approve_readiness())
    } else {
        None
    };
    first.send(()).unwrap();
    first_acked.await.unwrap();
    assert_eq!(handle.status().readiness(), Readiness::Starting);
    if order == 2 {
        handle.request();
    }
    if order != 0 {
        running = Some(pending.take().unwrap().approve_readiness());
    }
    second.send(()).unwrap();
    second_acked.await.unwrap();
    if order != 2 {
        handle.status().wait_ready().await.unwrap();
        assert_eq!(handle.status().readiness(), Readiness::Ready);
        handle.request();
    }
    // Keep the acknowledged component alive: Stopped cannot hide an invalid
    // Ready publication after drain while the coordinator finishes its report.
    assert_eq!(handle.status().readiness(), Readiness::Draining);
    assert!(handle.status().wait_ready().await.is_err());
    release.send(()).unwrap();
    let report = running.unwrap().wait().await.unwrap();
    assert!(report.is_success());
    assert_eq!(handle.status().readiness(), Readiness::Stopped);
    case.event("ordered-readiness-checked");
}

pub async fn acknowledgement_race(case: Case, delay: u64) {
    let mut supervisor =
        Supervisor::with_process_capacity(budget(), ProcessCapacity::new(1).unwrap());
    let handle = supervisor.handle();
    let barrier = Arc::new(Barrier::new(4));
    let component_barrier = barrier.clone();
    let (acked, ack) = oneshot::channel();
    let (release, released) = oneshot::channel();
    supervisor
        .register("racing-startup", move |signal| async move {
            component_barrier.wait().await;
            yields(delay).await;
            let signal = signal.acknowledge_started();
            acked.send(()).unwrap();
            released.await.unwrap();
            signal.draining().await;
            Ok(signal.stopped())
        })
        .unwrap();
    let pending = supervisor.start_unapproved();
    assert_eq!(handle.status().readiness(), Readiness::Starting);
    let request_barrier = barrier.clone();
    let requesting = handle.clone();
    let drain = tokio::spawn(async move {
        request_barrier.wait().await;
        yields(delay.rotate_left(9)).await;
        requesting.request();
    });
    let approval_barrier = barrier.clone();
    let approval = tokio::spawn(async move {
        approval_barrier.wait().await;
        yields(delay.rotate_left(21)).await;
        pending.approve_readiness()
    });
    barrier.wait().await;
    drain.await.unwrap();
    ack.await.unwrap();
    let running = approval.await.unwrap();
    case.event("drain-and-ack-returned");
    assert_eq!(handle.status().readiness(), Readiness::Draining);
    release.send(()).unwrap();
    assert!(running.wait().await.unwrap().is_success());
    assert_eq!(handle.status().readiness(), Readiness::Stopped);
}

pub async fn critical_exit(case: Case) {
    let mut supervisor =
        Supervisor::with_process_capacity(budget(), ProcessCapacity::new(1).unwrap());
    supervisor
        .register("uninitialized-exit", |startup| async {
            Ok(startup.abandon())
        })
        .unwrap();
    let handle = supervisor.handle();
    let running = supervisor.start();
    assert!(handle.status().wait_ready().await.is_err());
    let report = running.wait().await.unwrap();
    assert!(!report.is_success());
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].outcome, TaskOutcome::UnexpectedExit);
    case.event("critical-exit-without-ready");
}
