use batter::{
    BoxError,
    cleanup::CleanupBudget,
    lifecycle::{ProcessTaskError, ShutdownBudget, ShutdownCause, Supervisor, TaskOutcome},
};
use std::{
    convert::Infallible,
    future::{Future, pending, poll_fn},
    pin::Pin,
    task::Poll,
    time::Duration,
};
use tokio::{sync::oneshot, time::timeout};

fn supervisor(capacity: usize) -> Supervisor {
    let second = Duration::from_secs(1);
    let budget = ShutdownBudget::new(
        second,
        second,
        second,
        CleanupBudget::new(second, second, second).unwrap(),
    )
    .unwrap();
    let supervisor = Supervisor::with_process_capacity(budget, capacity).unwrap();
    assert!(supervisor.handle().mark_ready());
    supervisor
}

async fn bounded(future: impl Future<Output = ()>) {
    timeout(Duration::from_secs(10), future)
        .await
        .expect("shutdown cause scenario must finish within its watchdog");
}

fn register_healthy_component(supervisor: &mut Supervisor) {
    supervisor
        .register("component", |signal| async move {
            assert!(signal.mark_started());
            signal.draining().await;
            Ok(())
        })
        .unwrap();
}

async fn poll_once(mut driver: Pin<&mut impl Future>) {
    poll_fn(|cx| {
        assert!(driver.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn ready_request_precedes_an_unobserved_finite_error() {
    bounded(async {
        let supervisor = supervisor(1);
        let handle = supervisor.handle();
        let process = supervisor.process_handle().unwrap();
        let driver = supervisor.run_until(pending());
        tokio::pin!(driver);
        poll_once(driver.as_mut()).await;
        let receipt = process
            .try_spawn("finite", |_| async {
                Err::<(), _>(std::io::Error::other("finite error"))
            })
            .unwrap();
        poll_once(driver.as_mut()).await;
        assert!(matches!(
            receipt.wait().await,
            Err(ProcessTaskError::Failed(_))
        ));
        // The task completed while its coordinator was not polled.
        assert!(!handle.is_draining());
        handle.request();
        let report = driver.await;
        assert_eq!(report.cause, ShutdownCause::Requested);
        assert_eq!(report.tasks.len(), 1);
        assert_eq!(report.tasks[0].name, "finite");
        assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
        assert!(report.tasks[0].error.is_some());
        assert!(!report.is_success());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn ready_request_precedes_an_unobserved_critical_error() {
    bounded(async {
        let mut supervisor = supervisor(1);
        let (failed, observed) = oneshot::channel();
        supervisor
            .register("component", move |signal| async move {
                assert!(signal.mark_started());
                failed.send(()).unwrap();
                Err(std::io::Error::other("component error").into())
            })
            .unwrap();
        let handle = supervisor.handle();
        let driver = supervisor.run_until(pending());
        tokio::pin!(driver);
        poll_once(driver.as_mut()).await;
        // On this current-thread runtime, the component returns before the
        // receiver resumes. The coordinator has not observed that return.
        observed.await.unwrap();
        assert!(!handle.is_draining());
        handle.request();
        let report = driver.await;
        assert_eq!(report.cause, ShutdownCause::Requested);
        assert_eq!(report.tasks.len(), 1);
        assert_eq!(report.tasks[0].name, "component");
        assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
        assert!(report.tasks[0].error.is_some());
        assert!(!report.is_success());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn finite_error_and_panic_keep_their_kind_with_a_healthy_component() {
    for panics in [false, true] {
        bounded(async {
            let mut supervisor = supervisor(1);
            register_healthy_component(&mut supervisor);
            let process = supervisor.process_handle().unwrap();
            let running = supervisor.start();
            running.handle().wait_ready().await.unwrap();
            let receipt = process
                .try_spawn("finite", move |_| async move {
                    assert!(!panics, "finite task panic");
                    Err::<(), _>(std::io::Error::other("finite task error"))
                })
                .unwrap();
            let failure = receipt.wait().await.unwrap_err();
            assert_eq!(matches!(failure, ProcessTaskError::Terminated), panics);
            let report = running.wait().await.unwrap();
            assert_eq!(report.cause, ShutdownCause::FiniteTaskExit("finite"));
            assert_eq!(report.tasks.len(), 2);
            assert_eq!(report.tasks[0].name, "finite");
            assert_eq!(
                report.tasks[0].outcome,
                if panics {
                    TaskOutcome::Panicked
                } else {
                    TaskOutcome::Failed
                }
            );
            assert_eq!(report.tasks[1].name, "component");
            assert_eq!(report.tasks[1].outcome, TaskOutcome::Stopped);
            assert!(!report.is_success());
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn first_failure_is_retained_when_the_other_task_kind_fails_during_drain() {
    for finite_first in [false, true] {
        bounded(async {
            let mut supervisor = supervisor(1);
            let (release, admitted) = oneshot::channel();
            supervisor
                .register("component", move |signal| async move {
                    assert!(signal.mark_started());
                    admitted.await.unwrap();
                    if finite_first {
                        signal.draining().await;
                    }
                    Err::<(), BoxError>(std::io::Error::other("component error").into())
                })
                .unwrap();
            let process = supervisor.process_handle().unwrap();
            let running = supervisor.start();
            running.handle().wait_ready().await.unwrap();
            let receipt = process
                .try_spawn("finite", move |scope| async move {
                    if !finite_first {
                        scope.signal().draining().await;
                    }
                    Err::<(), _>(std::io::Error::other("finite error"))
                })
                .unwrap();
            release.send(()).unwrap();
            let report = running.wait().await.unwrap();
            let (cause, names) = if finite_first {
                (
                    ShutdownCause::FiniteTaskExit("finite"),
                    ["finite", "component"],
                )
            } else {
                (
                    ShutdownCause::ComponentExit("component"),
                    ["component", "finite"],
                )
            };
            assert_eq!(report.cause, cause);
            assert_eq!(report.tasks.len(), 2);
            for (record, name) in report.tasks.iter().zip(names) {
                assert_eq!(record.name, name);
                assert_eq!(record.outcome, TaskOutcome::Failed);
                assert!(record.error.is_some());
            }
            assert!(matches!(
                receipt.wait().await,
                Err(ProcessTaskError::Failed(_))
            ));
            assert!(report.abort_requested.is_empty());
            assert!(report.unjoined.is_empty());
            assert!(!report.is_success());
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn descendant_failure_reports_its_own_finite_label() {
    bounded(async {
        let mut supervisor = supervisor(2);
        register_healthy_component(&mut supervisor);
        let process = supervisor.process_handle().unwrap();
        let running = supervisor.start();
        running.handle().wait_ready().await.unwrap();
        let ancestor = process
            .try_spawn("ancestor", |scope| async move {
                let descendant = scope
                    .try_spawn("descendant", |_| async {
                        Err::<(), _>(std::io::Error::other("descendant error"))
                    })
                    .unwrap();
                Ok::<_, Infallible>(descendant)
            })
            .unwrap();
        let descendant = ancestor.wait().await.unwrap();
        assert!(matches!(
            descendant.wait().await,
            Err(ProcessTaskError::Failed(_))
        ));
        let report = running.wait().await.unwrap();
        assert_eq!(report.cause, ShutdownCause::FiniteTaskExit("descendant"));
        assert_eq!(report.completed_process_tasks, 1);
        assert_eq!(report.tasks.len(), 2);
        assert_eq!(report.tasks[0].name, "descendant");
        assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
        assert_eq!(report.tasks[1].outcome, TaskOutcome::Stopped);
        assert!(!report.is_success());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn finite_shutdown_abort_is_an_outcome_and_preserves_requested_cause() {
    bounded(async {
        let mut supervisor = supervisor(1);
        register_healthy_component(&mut supervisor);
        let process = supervisor.process_handle().unwrap();
        let running = supervisor.start();
        running.handle().wait_ready().await.unwrap();
        let (started, entered) = oneshot::channel();
        let receipt = process
            .try_spawn("finite", |_| async move {
                started.send(()).unwrap();
                std::future::pending::<Result<(), Infallible>>().await
            })
            .unwrap();
        entered.await.unwrap();
        let report = running.shutdown().await.unwrap();
        assert_eq!(report.cause, ShutdownCause::Requested);
        assert_eq!(report.abort_requested, ["finite"]);
        assert_eq!(report.tasks.len(), 2);
        let finite = report
            .tasks
            .iter()
            .find(|task| task.name == "finite")
            .unwrap();
        assert_eq!(finite.outcome, TaskOutcome::Aborted);
        assert!(report.unjoined.is_empty());
        assert!(matches!(
            receipt.wait().await,
            Err(ProcessTaskError::Terminated)
        ));
        assert!(!report.is_success());
    })
    .await;
}
