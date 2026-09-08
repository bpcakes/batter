use super::support::{Case, poll_pending, supervisor, yields};
use batter::{
    cleanup::CleanupBudget,
    lifecycle::{ProcessAdmissionError, ShutdownBudget, Supervisor},
};
use std::{
    convert::Infallible,
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Barrier, oneshot};

pub async fn queued_capacity(case: Case) {
    let supervisor = supervisor(2);
    let handle = supervisor.handle();
    let process = supervisor.process_handle().unwrap();
    let invoked = Arc::new(AtomicUsize::new(0));
    let driver = supervisor.run_until(pending());
    tokio::pin!(driver);
    poll_pending(driver.as_mut()).await;
    handle.wait_ready().await.unwrap();
    for _ in 0..2 {
        let invoked = invoked.clone();
        drop(
            process
                .try_spawn("queued", move |_| {
                    invoked.fetch_add(1, Ordering::SeqCst);
                    async { Ok::<_, Infallible>(()) }
                })
                .unwrap(),
        );
    }
    case.event("two-queued-driver-held");
    assert_eq!(invoked.load(Ordering::SeqCst), 0);
    assert!(matches!(
        process.try_spawn("queue-full", |_| async { Ok::<_, Infallible>(()) }),
        Err(ProcessAdmissionError::Full)
    ));
    handle.request();
    let report = driver.await;
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, 2);
    assert_eq!(invoked.load(Ordering::SeqCst), 2);
}

pub async fn root_drain(case: Case, delay: u64) {
    let supervisor = supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let after = process.clone();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let submit_barrier = barrier.clone();
    let submitted = tokio::spawn(async move {
        submit_barrier.wait().await;
        yields(delay).await;
        case.event("root-submit-call");
        let result = process.try_spawn("racing-root", |_| async { Ok::<_, Infallible>(41) });
        case.event(if result.is_ok() {
            "root-accepted"
        } else {
            "root-rejected"
        });
        result
    });
    let handle = running.handle();
    let drain_barrier = barrier.clone();
    let drain = tokio::spawn(async move {
        drain_barrier.wait().await;
        yields(delay.rotate_left(13)).await;
        handle.request();
        case.event("drain-returned");
    });
    barrier.wait().await;
    drain.await.unwrap();
    assert!(matches!(
        after.try_spawn("after-drain", |_| async { Ok::<_, Infallible>(()) }),
        Err(ProcessAdmissionError::Closed)
    ));
    let accepted = match submitted.await.unwrap() {
        Ok(receipt) => {
            assert_eq!(receipt.wait().await.unwrap(), 41);
            1
        }
        Err(ProcessAdmissionError::Closed) => 0,
        Err(_) => panic!("unexpected root/drain outcome"),
    };
    let report = running.wait().await.unwrap();
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, accepted);
}

pub async fn descendant_outlives_parent(case: Case) {
    let supervisor = supervisor(2);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let (scope_tx, scope_rx) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let parent = process
        .try_spawn("ancestor", |scope| async move {
            let child = scope
                .try_spawn("child", |child_scope| async move {
                    scope_tx.send(child_scope).ok().unwrap();
                    released.await.unwrap();
                    Ok::<_, Infallible>(19)
                })
                .unwrap();
            Ok::<_, Infallible>((scope, child))
        })
        .unwrap();
    let child_scope = scope_rx.await.unwrap();
    let (expired_scope, child) = parent.wait().await.unwrap();
    assert!(matches!(
        expired_scope.try_spawn("expired", |_| async { Ok::<_, Infallible>(()) }),
        Err(ProcessAdmissionError::Closed)
    ));
    running.handle().request();
    assert!(!child_scope.signal().is_cancelled());
    case.event("parent-finished-child-active-during-drain");
    let grandchild = child_scope
        .try_spawn("grandchild", |_| async { Ok::<_, Infallible>(23) })
        .unwrap();
    assert_eq!(grandchild.wait().await.unwrap(), 23);
    release.send(()).unwrap();
    assert_eq!(child.wait().await.unwrap(), 19);
    let report = running.wait().await.unwrap();
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, 3);
}

pub async fn scope_expiry(case: Case, delay: u64) {
    let supervisor = supervisor(2);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let parent_barrier = barrier.clone();
    let (scope_tx, scope_rx) = oneshot::channel();
    let parent = process
        .try_spawn("finishing-ancestor", move |scope| async move {
            scope_tx.send(scope).ok().unwrap();
            parent_barrier.wait().await;
            yields(delay).await;
            case.event("ancestor-returning");
            Ok::<_, Infallible>(())
        })
        .unwrap();
    let scope = scope_rx.await.unwrap();
    let escaped = scope.clone();
    let child_barrier = barrier.clone();
    let submit = tokio::spawn(async move {
        child_barrier.wait().await;
        yields(delay.rotate_left(11)).await;
        case.event("escaped-scope-submit");
        scope.try_spawn("racing-child", |_| async { Ok::<_, Infallible>(()) })
    });
    barrier.wait().await;
    parent.wait().await.unwrap();
    assert!(matches!(
        escaped.try_spawn("after-finish", |_| async { Ok::<_, Infallible>(()) }),
        Err(ProcessAdmissionError::Closed)
    ));
    let children = match submit.await.unwrap() {
        Ok(receipt) => {
            receipt.wait().await.unwrap();
            1
        }
        Err(ProcessAdmissionError::Closed) => 0,
        Err(_) => panic!("unexpected scope-expiry outcome"),
    };
    let report = running.shutdown().await.unwrap();
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, 1 + children);
}

pub async fn forced_descendant(case: Case) {
    let second = Duration::from_secs(1);
    let forced = ShutdownBudget::new(
        Duration::ZERO,
        second,
        second,
        CleanupBudget::new(second, second, second).unwrap(),
    )
    .unwrap();
    let supervisor = Supervisor::with_process_capacity(forced, 2).unwrap();
    let process = supervisor.process_handle().unwrap();
    supervisor.handle().mark_ready();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let (started, ready) = oneshot::channel();
    let receipt = process
        .try_spawn("forced-ancestor", move |scope| async move {
            started.send(()).unwrap();
            scope.signal().cancelled().await;
            case.event("force-observed");
            assert!(matches!(
                scope.try_spawn("forbidden-child", |_| async { Ok::<_, Infallible>(()) }),
                Err(ProcessAdmissionError::Closed)
            ));
            Ok::<_, Infallible>(())
        })
        .unwrap();
    ready.await.unwrap();
    running.handle().request();
    receipt.wait().await.unwrap();
    let report = running.wait().await.unwrap();
    assert!(report.forced_cancellation && report.is_success());
    assert_eq!(report.completed_process_tasks, 1);
}
