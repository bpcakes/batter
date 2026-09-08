use super::support::{Case, yields};
use batter::{
    cleanup::{CleanupBudget, CleanupOutcome, SkipReason},
    lifecycle::{
        ProcessAdmissionError, ProcessHandle, ProcessTaskError, ShutdownBudget, ShutdownHandle,
        ShutdownReport, Supervisor, TaskOutcome,
    },
};
use std::{
    convert::Infallible,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Barrier, oneshot};

pub async fn descendant_closure(
    case: Case,
    failure: bool,
    delay: u64,
    after_closure: impl Future<Output = ()> + Send + 'static,
) -> Arc<ShutdownReport> {
    let second = Duration::from_secs(1);
    let budget = ShutdownBudget::new(
        Duration::ZERO,
        second,
        second,
        CleanupBudget::new(second, second, second).unwrap(),
    )
    .unwrap();
    let mut supervisor = Supervisor::with_process_capacity(budget, 3).unwrap();
    let finalized = Arc::new(AtomicBool::new(false));
    let flag = finalized.clone();
    supervisor
        .on_cleanup("closure-dependent", move || async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    supervisor.handle().mark_ready();
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let (scope_tx, scope_rx) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let parent = process
        .try_spawn("active-ancestor", |scope| async move {
            scope_tx.send(scope).ok().unwrap();
            released.await.unwrap();
            Ok::<_, Infallible>(())
        })
        .unwrap();
    let scope = scope_rx.await.unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let closure_barrier = barrier.clone();
    let handle = running.handle();
    let closer = tokio::spawn(async move {
        let error = close_admission(case, failure, delay, process, handle, closure_barrier).await;
        after_closure.await;
        error
    });
    let submit_barrier = barrier.clone();
    let submitting_scope = scope.clone();
    let submit = tokio::spawn(async move {
        submit_barrier.wait().await;
        yields(delay.rotate_left(5)).await;
        case.event("descendant-racing-closure");
        submitting_scope.try_spawn("racing-descendant", |_| async { Ok::<_, Infallible>(43) })
    });
    barrier.wait().await;
    let original_error = closer.await.unwrap();
    assert!(matches!(
        scope.try_spawn("after-closure", |_| async { Ok::<_, Infallible>(()) }),
        Err(ProcessAdmissionError::Closed)
    ));
    let mut completed = 0;
    let mut terminated = Vec::new();
    let child_admitted = match submit.await.unwrap() {
        Ok(receipt) => {
            match receipt.wait().await {
                Ok(value) => {
                    assert_eq!(value, 43);
                    completed += 1;
                }
                Err(ProcessTaskError::Terminated) => terminated.push("racing-descendant"),
                Err(_) => panic!("unexpected descendant task error"),
            }
            true
        }
        Err(ProcessAdmissionError::Closed) => false,
        Err(_) => panic!("unexpected descendant/closure race outcome"),
    };
    // The deadline can drop the receiver before the test releases it. A
    // successful send also does not exclude an abort racing the final poll.
    let released = release.send(()).is_ok();
    match parent.wait().await {
        Ok(()) => {
            assert!(released);
            completed += 1;
        }
        Err(ProcessTaskError::Terminated) => terminated.push("active-ancestor"),
        Err(_) => panic!("unexpected ancestor task error"),
    }
    let report = running.wait().await.unwrap();
    assert_closure_report(
        &report,
        completed,
        &terminated,
        original_error.as_ref(),
        child_admitted,
        finalized.load(Ordering::SeqCst),
    );
    case.event("closure-receipts-report-and-cleanup-reconciled");
    report
}

async fn close_admission(
    case: Case,
    failure: bool,
    delay: u64,
    process: ProcessHandle,
    handle: ShutdownHandle,
    closure_barrier: Arc<Barrier>,
) -> Option<Arc<std::io::Error>> {
    let (fail_tx, fail_rx) = oneshot::channel();
    let receipt = if failure {
        Some(
            process
                .try_spawn("closing-failure", |_| async move {
                    fail_rx.await.unwrap();
                    Err::<(), _>(std::io::Error::other("generic failure fixture"))
                })
                .unwrap(),
        )
    } else {
        None
    };
    closure_barrier.wait().await;
    yields(delay).await;
    if let Some(receipt) = receipt {
        fail_tx.send(()).unwrap();
        let Err(ProcessTaskError::Failed(error)) = receipt.wait().await else {
            panic!("closure task failure lost");
        };
        case.event("task-failure-published");
        Some(error)
    } else {
        handle.request();
        handle.signal().cancelled().await;
        case.event("forced-closure-observed");
        None
    }
}

fn assert_closure_report(
    report: &ShutdownReport,
    completed: u64,
    terminated: &[&'static str],
    original_error: Option<&Arc<std::io::Error>>,
    child_admitted: bool,
    finalized: bool,
) {
    let mut expected: Vec<_> = terminated
        .iter()
        .map(|name| (*name, TaskOutcome::Aborted))
        .collect();
    if original_error.is_some() {
        expected.push(("closing-failure", TaskOutcome::Failed));
    }
    let mut observed: Vec<_> = report.tasks.iter().map(|t| (t.name, t.outcome)).collect();
    expected.sort_unstable_by_key(|entry| entry.0);
    observed.sort_unstable_by_key(|entry| entry.0);
    assert_eq!(observed, expected);
    assert_eq!(report.completed_process_tasks, completed);
    assert!(report.unjoined.is_empty());
    assert_closure_errors(report, original_error);
    for name in &report.abort_requested {
        assert!(
            *name == "active-ancestor" || (child_admitted && *name == "racing-descendant"),
            "abort requested for an unadmitted or already-failed task"
        );
    }
    let aborted = !report.abort_requested.is_empty();
    assert_eq!(report.is_success(), original_error.is_none() && !aborted);
    assert_eq!(finalized, !aborted);
    if aborted {
        assert!(report.forced_cancellation);
        assert!(report.cleanup.records.is_empty());
        assert_eq!(report.cleanup.skipped.len(), 1);
        assert_eq!(report.cleanup.skipped[0].name, "closure-dependent");
        assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
    } else {
        assert!(report.cleanup.skipped.is_empty());
        assert_eq!(report.cleanup.records.len(), 1);
        assert_eq!(report.cleanup.records[0].name, "closure-dependent");
        assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
        assert!(report.cleanup.records[0].error.is_none());
    }
}

fn assert_closure_errors(report: &ShutdownReport, original_error: Option<&Arc<std::io::Error>>) {
    for task in &report.tasks {
        let mut cause: &(dyn std::error::Error + 'static) = task.error.as_deref().unwrap();
        if task.outcome == TaskOutcome::Aborted {
            assert!(report.abort_requested.contains(&task.name));
            let join = cause.downcast_ref::<tokio::task::JoinError>().unwrap();
            assert!(join.is_cancelled() && !join.is_panic());
        } else {
            while cause.downcast_ref::<std::io::Error>().is_none() {
                cause = cause.source().expect("missing retained closure failure");
            }
            assert!(std::ptr::eq(
                original_error.unwrap().as_ref(),
                cause.downcast_ref::<std::io::Error>().unwrap()
            ));
        }
    }
}
