#[path = "process_ownership/error_sources.rs"]
mod error_sources;

#[path = "process_ownership/report_usage.rs"]
mod report_usage;

#[path = "process_ownership/terminal_admission.rs"]
mod terminal_admission;

use batter::{
    BoxError,
    cleanup::{CleanupBudget, SkipReason},
    lifecycle::{
        ProcessAdmissionError, ProcessTaskError, Readiness, ShutdownBudget, ShutdownCause,
        ShutdownReport, Supervisor, TaskOutcome,
    },
};
use std::{
    convert::Infallible,
    future::{Future, pending, poll_fn},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::Poll,
    time::Duration,
};
use tokio::sync::{Barrier, oneshot};

fn budget() -> ShutdownBudget {
    let second = Duration::from_secs(1);
    ShutdownBudget::new(
        second,
        second,
        second,
        CleanupBudget::new(second, second, second).unwrap(),
    )
    .unwrap()
}

fn finite_supervisor(capacity: usize) -> Supervisor {
    let supervisor = Supervisor::with_process_capacity(budget(), capacity).unwrap();
    assert!(supervisor.handle().mark_ready());
    supervisor
}

async fn poll_driver_once(driver: Pin<&mut impl Future<Output = ShutdownReport>>) {
    let mut driver = driver;
    poll_fn(|cx| {
        assert!(driver.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn completed_work_is_observed_before_abort_after_coordinator_delay() {
    let finalized = Arc::new(AtomicBool::new(false));
    let flag = finalized.clone();
    let mut supervisor = finite_supervisor(1);
    supervisor
        .on_cleanup("resource", move || async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let handle = supervisor.handle();
    let process = supervisor.process_handle().unwrap();
    let driver = supervisor.run_until(pending());
    tokio::pin!(driver);
    poll_driver_once(driver.as_mut()).await;
    let (finish, finished) = oneshot::channel();
    let receipt = process
        .try_spawn("finished", |_| async move {
            finished.await.unwrap();
            Ok::<_, Infallible>(())
        })
        .unwrap();
    poll_driver_once(driver.as_mut()).await;
    handle.request();
    poll_driver_once(driver.as_mut()).await;
    // Enter cancellation at its absolute boundary; completion still precedes abort.
    tokio::time::advance(Duration::from_secs(1)).await;
    poll_driver_once(driver.as_mut()).await;
    finish.send(()).unwrap();
    receipt.wait().await.unwrap();
    // Work finished during the allowance; its coordinator resumes afterward.
    tokio::time::advance(Duration::from_secs(2)).await;
    let report = driver.await;
    assert_eq!(report.completed_process_tasks, 1);
    assert!(report.abort_requested.is_empty());
    assert!(report.unjoined.is_empty());
    assert!(finalized.load(Ordering::SeqCst));
    assert!(report.is_success());
}

#[tokio::test(start_paused = true)]
async fn only_unfinished_work_is_aborted_and_real_abort_still_skips_cleanup() {
    let mut supervisor = finite_supervisor(2);
    supervisor
        .on_cleanup("resource", || async {
            panic!("genuine abort must keep dependency cleanup conservative");
            #[allow(unreachable_code)]
            Ok(())
        })
        .unwrap();
    let handle = supervisor.handle();
    let process = supervisor.process_handle().unwrap();
    let driver = supervisor.run_until(pending());
    tokio::pin!(driver);
    poll_driver_once(driver.as_mut()).await;
    let (finish, finished) = oneshot::channel();
    let receipt = process
        .try_spawn("finished", |_| async move {
            finished.await.unwrap();
            Ok::<_, Infallible>(())
        })
        .unwrap();
    let pending_receipt = process
        .try_spawn("pending", |_| pending::<Result<(), Infallible>>())
        .unwrap();
    poll_driver_once(driver.as_mut()).await;
    handle.request();
    poll_driver_once(driver.as_mut()).await;
    // Enter cancellation at its absolute boundary; completion still precedes abort.
    tokio::time::advance(Duration::from_secs(1)).await;
    poll_driver_once(driver.as_mut()).await;
    finish.send(()).unwrap();
    receipt.wait().await.unwrap();
    // Resume after cancellation expired, with observation time left for actual aborts.
    tokio::time::advance(Duration::from_millis(1500)).await;
    let report = driver.await;
    assert_eq!(report.completed_process_tasks, 1);
    assert_eq!(report.abort_requested, ["pending"]);
    assert!(matches!(
        pending_receipt.wait().await,
        Err(ProcessTaskError::Terminated)
    ));
    assert_eq!(report.tasks[0].name, "pending");
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Aborted);
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn completed_failures_retain_causes_when_observed_after_deadline() {
    for panic_on_finish in [false, true] {
        let finalized = Arc::new(AtomicBool::new(false));
        let flag = finalized.clone();
        let mut supervisor = finite_supervisor(1);
        supervisor
            .on_cleanup("resource", move || async move {
                flag.store(true, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        let handle = supervisor.handle();
        let process = supervisor.process_handle().unwrap();
        let driver = supervisor.run_until(pending());
        tokio::pin!(driver);
        poll_driver_once(driver.as_mut()).await;
        let (finish, finished) = oneshot::channel();
        let receipt = process
            .try_spawn("failed", move |_| async move {
                finished.await.unwrap();
                assert!(!panic_on_finish, "original task panic");
                Err::<(), _>(std::io::Error::other("original task error"))
            })
            .unwrap();
        poll_driver_once(driver.as_mut()).await;
        handle.request();
        poll_driver_once(driver.as_mut()).await;
        tokio::time::advance(Duration::from_secs(1)).await;
        poll_driver_once(driver.as_mut()).await;
        finish.send(()).unwrap();
        let failure = receipt.wait().await.unwrap_err();
        tokio::time::advance(Duration::from_secs(2)).await;
        let report = driver.await;
        assert_eq!(report.cause, ShutdownCause::Requested);
        assert!(report.abort_requested.is_empty());
        assert!(report.unjoined.is_empty());
        assert!(!report.is_success());
        let record = &report.tasks[0];
        match failure {
            ProcessTaskError::Failed(original) => {
                assert!(!panic_on_finish);
                assert_eq!(record.outcome, TaskOutcome::Failed);
                let reported = record
                    .error
                    .as_ref()
                    .unwrap()
                    .source()
                    .unwrap()
                    .downcast_ref::<std::io::Error>()
                    .unwrap();
                assert!(std::ptr::eq(original.as_ref(), reported));
                assert_eq!(reported.to_string(), "original task error");
                assert!(finalized.load(Ordering::SeqCst));
            }
            ProcessTaskError::Terminated => {
                assert!(panic_on_finish);
                assert_eq!(record.outcome, TaskOutcome::Panicked);
                assert!(
                    record
                        .error
                        .as_ref()
                        .unwrap()
                        .downcast_ref::<tokio::task::JoinError>()
                        .unwrap()
                        .is_panic()
                );
                assert!(!finalized.load(Ordering::SeqCst));
                assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn completed_critical_stop_is_observed_after_deadline_without_false_abort() {
    let finalized = Arc::new(AtomicBool::new(false));
    let flag = finalized.clone();
    let mut supervisor = finite_supervisor(1);
    let (finish, finished) = oneshot::channel();
    let (stopped, stopped_rx) = oneshot::channel();
    supervisor
        .register("component", move |signal| async move {
            signal.mark_started();
            finished.await.unwrap();
            stopped.send(()).unwrap();
            Ok(())
        })
        .unwrap();
    supervisor
        .on_cleanup("resource", move || async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let handle = supervisor.handle();
    let driver = supervisor.run_until(pending());
    tokio::pin!(driver);
    poll_driver_once(driver.as_mut()).await;
    handle.wait_ready().await.unwrap();
    handle.request();
    poll_driver_once(driver.as_mut()).await;
    // Enter cancellation at its absolute boundary; completion still precedes abort.
    tokio::time::advance(Duration::from_secs(1)).await;
    poll_driver_once(driver.as_mut()).await;
    finish.send(()).unwrap();
    stopped_rx.await.unwrap();
    tokio::time::advance(Duration::from_secs(2)).await;
    let report = driver.await;
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].name, "component");
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Stopped);
    assert!(report.abort_requested.is_empty());
    assert!(report.unjoined.is_empty());
    assert!(finalized.load(Ordering::SeqCst));
    assert!(report.is_success());
}

#[tokio::test]
async fn unstarted_admission_is_inert_and_capacity_is_validated() {
    assert!(Supervisor::with_process_capacity(budget(), 0).is_err());
    let supervisor = finite_supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let called = Arc::new(AtomicBool::new(false));
    let in_factory = called.clone();
    let result = process.try_spawn("before-start", move |_| {
        in_factory.store(true, Ordering::SeqCst);
        async { Ok::<_, Infallible>(()) }
    });
    assert!(matches!(result, Err(ProcessAdmissionError::NotRunning)));
    assert!(!called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn readiness_waits_for_every_component_acknowledgement() {
    let mut supervisor = finite_supervisor(1);
    let handle = supervisor.handle();
    let process = supervisor.process_handle().unwrap();
    let (first_tx, first_rx) = oneshot::channel();
    let (initialize_tx, initialize_rx) = oneshot::channel();
    supervisor
        .register("first", move |signal| async move {
            assert!(signal.mark_started());
            assert!(!signal.mark_started());
            first_tx.send(()).unwrap();
            signal.draining().await;
            Ok(())
        })
        .unwrap();
    supervisor
        .register("second", move |signal| async move {
            initialize_rx.await.unwrap();
            signal.mark_started();
            signal.draining().await;
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    first_rx.await.unwrap();
    assert_eq!(handle.readiness(), Readiness::Starting);
    assert!(matches!(
        process.try_spawn("too-soon", |_| async { Ok::<_, Infallible>(()) }),
        Err(ProcessAdmissionError::NotReady)
    ));
    initialize_tx.send(()).unwrap();
    handle.wait_ready().await.unwrap();
    assert_eq!(handle.readiness(), Readiness::Ready);
    assert!(running.shutdown().await.unwrap().is_success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_request_waiter_keeps_work_and_permit_owned_until_finish() {
    let supervisor = finite_supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let (started_tx, started_rx) = oneshot::channel();
    let (finish_tx, finish_rx) = oneshot::channel();
    let receipt = process
        .try_spawn("admitted-payment", move |_| async move {
            started_tx.send(()).unwrap();
            finish_rx.await.unwrap();
            Ok::<_, Infallible>(42)
        })
        .unwrap();
    let request = tokio::spawn(receipt.wait());
    started_rx.await.unwrap();
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    assert!(matches!(
        process.try_spawn("must-still-be-full", |_| async { Ok::<_, Infallible>(()) }),
        Err(ProcessAdmissionError::Full)
    ));
    finish_tx.send(()).unwrap();
    let report = running.shutdown().await.unwrap();
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, 1);
}

#[tokio::test]
async fn normal_business_denial_is_a_typed_value_and_does_not_stop_process() {
    #[derive(Debug, PartialEq)]
    enum Denial {
        Quota,
    }
    let supervisor = finite_supervisor(2);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let denial = process
        .try_spawn("quota-check", |_| async {
            Ok::<Result<(), Denial>, Infallible>(Err(Denial::Quota))
        })
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_eq!(denial, Err(Denial::Quota));
    assert!(!running.handle().is_draining());
    let value = process
        .try_spawn("next-task", |_| async { Ok::<_, Infallible>(23) })
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_eq!(value, 23);
    let report = running.shutdown().await.unwrap();
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, 2);
    assert_eq!(report.cause, ShutdownCause::Requested);
    assert!(
        report.tasks.is_empty(),
        "successful task history must not grow without bound"
    );
}

#[tokio::test]
async fn unobserved_task_failure_initiates_drain_and_retains_original_source() {
    let supervisor = finite_supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let receipt = process
        .try_spawn("failed-task", |_| async {
            Err::<(), _>(std::io::Error::other("private failure cause"))
        })
        .unwrap();
    drop(receipt);
    let report = running.wait().await.unwrap();
    assert_eq!(report.cause, ShutdownCause::FiniteTaskExit("failed-task"));
    assert!(!report.is_success());
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
    let source = report.tasks[0]
        .error
        .as_ref()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(source.to_string(), "private failure cause");
    assert!(matches!(
        process.try_spawn("after-failure", |_| async { Ok::<_, Infallible>(()) }),
        Err(ProcessAdmissionError::Closed)
    ));
}

#[tokio::test]
async fn finite_factory_panic_is_observed_and_skips_dependent_cleanup() {
    let mut supervisor = finite_supervisor(1);
    supervisor
        .on_cleanup("dependency", || async {
            panic!("must skip dependency");
            #[allow(unreachable_code)]
            Ok(())
        })
        .unwrap();
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let receipt = process
        .try_spawn(
            "factory-panic",
            |_| -> std::future::Ready<Result<(), Infallible>> { panic!("task factory panic") },
        )
        .unwrap();
    assert!(matches!(
        receipt.wait().await,
        Err(ProcessTaskError::Terminated)
    ));
    let report = running.wait().await.unwrap();
    assert_eq!(report.cause, ShutdownCause::FiniteTaskExit("factory-panic"));
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Panicked);
    assert!(
        report.tasks[0]
            .error
            .as_ref()
            .unwrap()
            .downcast_ref::<tokio::task::JoinError>()
            .unwrap()
            .is_panic()
    );
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
}

#[tokio::test]
async fn admitted_ancestor_can_submit_bounded_children_during_drain() {
    let supervisor = finite_supervisor(2);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let (parent_tx, parent_rx) = oneshot::channel();
    let (child_tx, child_rx) = oneshot::channel();
    let (finish_tx, finish_rx) = oneshot::channel();
    let parent = process
        .try_spawn("ancestor", move |scope| async move {
            parent_tx.send(()).unwrap();
            scope.signal().draining().await;
            let child = scope
                .try_spawn("descendant", move |_| async move {
                    child_tx.send(()).unwrap();
                    finish_rx.await.unwrap();
                    Ok::<_, Infallible>(17)
                })
                .unwrap();
            Ok::<_, Infallible>(child.wait().await.unwrap())
        })
        .unwrap();
    parent_rx.await.unwrap();
    running.handle().request();
    child_rx.await.unwrap();
    assert!(matches!(
        process.try_spawn("new-root", |_| async { Ok::<_, Infallible>(()) }),
        Err(ProcessAdmissionError::Closed)
    ));
    finish_tx.send(()).unwrap();
    assert_eq!(parent.wait().await.unwrap(), 17);
    let report = running.wait().await.unwrap();
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, 2);
}

#[tokio::test]
async fn escaped_scope_expires_when_its_actual_task_finishes() {
    let supervisor = finite_supervisor(2);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let scope = process
        .try_spawn("escapes-scope", |scope| async {
            Ok::<_, Infallible>(scope)
        })
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(matches!(
        scope.try_spawn("expired-ancestor", |_| async { Ok::<_, Infallible>(()) }),
        Err(ProcessAdmissionError::Closed)
    ));
    assert!(running.shutdown().await.unwrap().is_success());
}

#[tokio::test(start_paused = true)]
async fn forced_cancellation_closes_descendant_admission() {
    let second = Duration::from_secs(1);
    let shutdown_budget = ShutdownBudget::new(
        Duration::ZERO,
        second,
        second,
        CleanupBudget::new(second, second, second).unwrap(),
    )
    .unwrap();
    let supervisor = Supervisor::with_process_capacity(shutdown_budget, 2).unwrap();
    let process = supervisor.process_handle().unwrap();
    supervisor.handle().mark_ready();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let (started_tx, started_rx) = oneshot::channel();
    let task = process
        .try_spawn("cancelled-ancestor", move |scope| async move {
            started_tx.send(()).unwrap();
            scope.signal().cancelled().await;
            assert!(matches!(
                scope.try_spawn("forbidden-child", |_| async { Ok::<_, Infallible>(()) }),
                Err(ProcessAdmissionError::Closed)
            ));
            Ok::<_, Infallible>(())
        })
        .unwrap();
    started_rx.await.unwrap();
    running.handle().request();
    task.wait().await.unwrap();
    let report = running.wait().await.unwrap();
    assert!(report.forced_cancellation);
    assert!(report.is_success());
}

#[tokio::test]
async fn cancelling_shutdown_waiter_cannot_cancel_started_cleanup() {
    let mut supervisor = finite_supervisor(1);
    let (cleanup_tx, cleanup_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let finalized = Arc::new(AtomicBool::new(false));
    let in_cleanup = finalized.clone();
    supervisor
        .on_cleanup("owned-cleanup", move || async move {
            cleanup_tx.send(()).unwrap();
            release_rx.await.unwrap();
            in_cleanup.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    let handle = running.handle();
    handle.wait_ready().await.unwrap();
    let observer = running.observer();
    let other_owner = running.clone();
    drop(running);
    assert!(
        !handle.is_draining(),
        "one remaining driver owner keeps process running"
    );
    let waiter = tokio::spawn(async move { other_owner.shutdown().await });
    cleanup_rx.await.unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    release_tx.send(()).unwrap();
    let first_report = observer.wait().await.unwrap();
    let second_report = observer.clone().wait().await.unwrap();
    assert!(first_report.is_success());
    assert!(std::ptr::eq(&*first_report, &*second_report));
    assert!(finalized.load(Ordering::SeqCst));
    assert_eq!(handle.readiness(), Readiness::Stopped);
}

#[tokio::test]
async fn last_owner_drop_requests_shutdown_and_observer_retains_report() {
    let mut supervisor = finite_supervisor(1);
    let cleaned = Arc::new(AtomicBool::new(false));
    let in_cleanup = cleaned.clone();
    supervisor
        .on_cleanup("cleanup", move || async move {
            in_cleanup.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let observer = running.observer();
    drop(running);
    let report = observer.wait().await.unwrap();
    assert!(report.is_success());
    assert!(cleaned.load(Ordering::SeqCst));
}

#[tokio::test]
async fn component_startup_failure_never_publishes_ready_and_still_cleans_up() {
    let mut supervisor = finite_supervisor(1);
    let handle = supervisor.handle();
    supervisor
        .register("initialization-fails", |_| async {
            Err(std::io::Error::other("startup failure").into())
        })
        .unwrap();
    let cleaned = Arc::new(AtomicBool::new(false));
    let in_cleanup = cleaned.clone();
    supervisor
        .on_cleanup("already-acquired", move || async move {
            in_cleanup.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    assert!(matches!(
        handle.wait_ready().await,
        Err(Readiness::Draining | Readiness::Stopped)
    ));
    let report = running.wait().await.unwrap();
    assert!(!report.is_success());
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
    assert!(report.cleanup.is_success());
    assert!(cleaned.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn owned_driver_panic_is_retained_for_observers_without_hanging() {
    struct PanickingDrop;
    impl Drop for PanickingDrop {
        fn drop(&mut self) {
            panic!("coordinator-owned native destructor panic")
        }
    }
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("abort-me", |_| pending::<Result<(), BoxError>>())
        .unwrap();
    let value = PanickingDrop;
    supervisor
        .on_cleanup("skip-panics", move || async move {
            drop(value);
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    let observer = running.observer();
    running.handle().request();
    let outcome = tokio::time::timeout(Duration::from_secs(5), observer.wait())
        .await
        .unwrap();
    let error = outcome.unwrap_err();
    assert!(error.is_panic());
    let again = observer.wait().await.unwrap_err();
    assert!(Arc::ptr_eq(&error, &again));
}

#[tokio::test]
async fn cancelling_explicit_coordinator_aborts_owned_task_and_skips_async_cleanup() {
    struct NotifyDrop(Option<oneshot::Sender<()>>);
    impl Drop for NotifyDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    let mut supervisor = Supervisor::new(budget());
    let handle = supervisor.handle();
    let token = handle.operation_token();
    let (started_tx, started_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    supervisor
        .register("direct-task", move |signal| async move {
            let _guard = NotifyDrop(Some(dropped_tx));
            signal.mark_started();
            started_tx.send(()).unwrap();
            pending::<Result<(), BoxError>>().await
        })
        .unwrap();
    let cleanup_called = Arc::new(AtomicBool::new(false));
    let in_cleanup = cleanup_called.clone();
    supervisor
        .on_cleanup("not-driven", move || async move {
            in_cleanup.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let coordinator = tokio::spawn(supervisor.run_until(pending()));
    started_rx.await.unwrap();
    coordinator.abort();
    assert!(coordinator.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(1), dropped_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(token.is_cancelled());
    assert!(handle.is_draining());
    assert!(!cleanup_called.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admission_racing_drain_is_either_rejected_or_included_in_shutdown() {
    for _ in 0..64 {
        let supervisor = finite_supervisor(1);
        let process = supervisor.process_handle().unwrap();
        let running = supervisor.start();
        running.handle().wait_ready().await.unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let ran = Arc::new(AtomicUsize::new(0));
        let submit_barrier = barrier.clone();
        let in_task = ran.clone();
        let submit = tokio::spawn(async move {
            submit_barrier.wait().await;
            process.try_spawn("racing-admission", move |_| async move {
                in_task.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Infallible>(())
            })
        });
        let drain_barrier = barrier.clone();
        let handle = running.handle();
        let drain = tokio::spawn(async move {
            drain_barrier.wait().await;
            handle.request();
        });
        barrier.wait().await;
        drain.await.unwrap();
        let accepted = match submit.await.unwrap() {
            Ok(receipt) => {
                receipt.wait().await.unwrap();
                1
            }
            Err(ProcessAdmissionError::Closed) => 0,
            Err(error) => panic!("unexpected admission result: {error}"),
        };
        let report = running.wait().await.unwrap();
        assert!(report.is_success());
        assert_eq!(report.completed_process_tasks, accepted);
        assert_eq!(ran.load(Ordering::SeqCst), accepted as usize);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn startup_acknowledgement_racing_drain_cannot_restore_readiness() {
    for _ in 0..64 {
        let mut supervisor = finite_supervisor(1);
        let handle = supervisor.handle();
        let barrier = Arc::new(Barrier::new(3));
        let startup_barrier = barrier.clone();
        let (started_tx, started_rx) = oneshot::channel();
        supervisor
            .register("starting-component", move |signal| async move {
                startup_barrier.wait().await;
                signal.mark_started();
                started_tx.send(()).unwrap();
                signal.draining().await;
                Ok(())
            })
            .unwrap();
        let running = supervisor.start();
        assert_eq!(handle.readiness(), Readiness::Starting);
        let drain_barrier = barrier.clone();
        let requesting = handle.clone();
        let drain = tokio::spawn(async move {
            drain_barrier.wait().await;
            requesting.request();
        });
        barrier.wait().await;
        drain.await.unwrap();
        started_rx.await.unwrap();
        assert!(matches!(
            handle.readiness(),
            Readiness::Draining | Readiness::Stopped
        ));
        assert!(!handle.mark_ready());
        assert!(handle.wait_ready().await.is_err());
        assert!(running.wait().await.unwrap().is_success());
    }
}
