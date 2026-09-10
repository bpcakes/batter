//! A direct component join is meaningful only with an application-owned child protocol.

use batter::{
    cleanup::{CleanupBudget, CleanupOutcome, SkipReason},
    lifecycle::{
        Readiness, ShutdownBudget, ShutdownCause, ShutdownReport, Supervisor, TaskOutcome,
    },
};
use std::{
    future::{Future, pending},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

type Events = Arc<Mutex<Vec<&'static str>>>;

fn snapshot(events: &Events) -> Vec<&'static str> {
    events.lock().unwrap().clone()
}

async fn bounded(future: impl Future<Output = ()>) {
    // Paused Tokio time bounds yielding deadlocks, not non-yielding execution.
    // Non-yielding evidence belongs to the independent subprocess suite.
    tokio::time::timeout(Duration::from_secs(10), future)
        .await
        .expect("component comparison exceeded virtual-time bound");
}

#[tokio::test(start_paused = true)]
#[should_panic(expected = "component comparison exceeded virtual-time bound")]
async fn unresolved_comparison_is_rejected() {
    bounded(pending()).await;
}

fn budget() -> ShutdownBudget {
    ShutdownBudget::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(1),
        CleanupBudget::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap(),
    )
    .unwrap()
}

struct Lifetime(Events);

impl Drop for Lifetime {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("child-dropped");
    }
}

#[tokio::test(start_paused = true)]
async fn conforming_component_acknowledges_initialization_and_joins_before_cleanup() {
    bounded(conforming_component()).await;
}

async fn conforming_component() {
    let events = Events::default();
    let mut supervisor = Supervisor::new(budget());
    let handle = supervisor.handle();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (initialize_tx, initialize_rx) = oneshot::channel();
    let (initialized_tx, initialized_rx) = oneshot::channel();
    let (drained_tx, drained_rx) = oneshot::channel();
    let (stop_tx, stop_rx) = oneshot::channel();
    let component_events = events.clone();
    supervisor
        .register("component", move |signal| async move {
            let child_events = component_events.clone();
            let child = tokio::spawn(async move {
                let _lifetime = Lifetime(child_events.clone());
                entered_tx.send(()).unwrap();
                initialize_rx.await.unwrap();
                child_events.lock().unwrap().push("initialized");
                initialized_tx.send(()).unwrap();
                stop_rx.await.unwrap();
            });
            initialized_rx.await.unwrap();
            assert!(signal.mark_started());
            component_events.lock().unwrap().push("acknowledged");
            signal.draining().await;
            drained_tx.send(()).unwrap();
            child.await.unwrap();
            component_events.lock().unwrap().push("child-joined");
            Ok(())
        })
        .unwrap();
    let cleanup_events = events.clone();
    supervisor
        .on_cleanup("dependency", move || async move {
            cleanup_events.lock().unwrap().push("cleanup");
            Ok(())
        })
        .unwrap();
    handle.mark_ready();
    let running = supervisor.start();
    entered_rx.await.unwrap();
    assert_eq!(handle.readiness(), Readiness::Starting);
    initialize_tx.send(()).unwrap();
    handle.wait_ready().await.unwrap();
    handle.request();
    drained_rx.await.unwrap();
    assert_eq!(handle.readiness(), Readiness::Draining);
    assert_eq!(snapshot(&events), ["initialized", "acknowledged"]);
    stop_tx.send(()).unwrap();
    let report = running.wait().await.unwrap();
    assert!(report.is_success(), "{report:?}");
    assert!(!report.forced_cancellation);
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Stopped);
    assert_eq!(
        snapshot(&events),
        [
            "initialized",
            "acknowledged",
            "child-dropped",
            "child-joined",
            "cleanup"
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn nonconforming_wrapper_can_report_success_and_cleanup_while_child_still_executes() {
    bounded(nonconforming_component()).await;
}

async fn nonconforming_component() {
    let events = Events::default();
    let mut supervisor = Supervisor::new(budget());
    let handle = supervisor.handle();
    let (child_tx, child_rx) = oneshot::channel();
    let (ping_tx, mut ping_rx) = mpsc::channel::<oneshot::Sender<()>>(1);
    let (stop_tx, stop_rx) = oneshot::channel();
    let child_events = events.clone();
    supervisor
        .register("component", move |signal| async move {
            let (initialized_tx, initialized_rx) = oneshot::channel();
            let child = tokio::spawn(async move {
                let _lifetime = Lifetime(child_events.clone());
                initialized_tx.send(()).unwrap();
                tokio::pin!(stop_rx);
                loop {
                    tokio::select! {
                        _ = &mut stop_rx => break,
                        reply = ping_rx.recv() => {
                            child_events.lock().unwrap().push("child-replied");
                            reply.unwrap().send(()).unwrap();
                        }
                    }
                }
            });
            initialized_rx.await.unwrap();
            // Only the test retains this handle. The registered wrapper never joins it.
            child_tx.send(child).unwrap();
            signal.mark_started();
            signal.draining().await;
            Ok(())
        })
        .unwrap();
    let cleanup_events = events.clone();
    supervisor
        .on_cleanup("dependency", move || async move {
            cleanup_events.lock().unwrap().push("cleanup");
            Ok(())
        })
        .unwrap();
    handle.mark_ready();
    let running = supervisor.start();
    let child = child_rx.await.unwrap();
    handle.wait_ready().await.unwrap();
    let report = running.shutdown().await.unwrap();
    assert!(report.is_success());
    assert!(!report.forced_cancellation);
    assert!(report.all_direct_tasks_joined());
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Stopped);
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    assert_eq!(snapshot(&events), ["cleanup"]);
    let (reply_tx, reply_rx) = oneshot::channel();
    ping_tx.send(reply_tx).await.unwrap();
    reply_rx.await.unwrap();
    assert!(!child.is_finished());
    assert_eq!(snapshot(&events), ["cleanup", "child-replied"]);
    stop_tx.send(()).unwrap();
    child.await.unwrap();
    assert_eq!(snapshot(&events).last(), Some(&"child-dropped"));
}

#[derive(Clone, Copy)]
enum Exit {
    Early,
    Error,
    Panic,
    Abort,
}

#[derive(Debug, thiserror::Error)]
#[error("component fixture failure")]
struct ComponentFailure;

#[derive(Debug, thiserror::Error)]
#[error("cleanup fixture failure")]
struct CleanupFailure;

async fn check_exit(exit: Exit) {
    let events = Events::default();
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("component", move |signal| async move {
            signal.mark_started();
            match exit {
                Exit::Early => Ok(()),
                Exit::Error => Err(ComponentFailure.into()),
                Exit::Panic => panic!("fixed component fixture panic"),
                Exit::Abort => pending().await,
            }
        })
        .unwrap();
    let cleanup_events = events.clone();
    supervisor
        .on_cleanup("dependency", move || async move {
            cleanup_events.lock().unwrap().push("cleanup");
            Err(CleanupFailure.into())
        })
        .unwrap();
    let report = supervisor
        .run_until(async {
            if !matches!(exit, Exit::Abort) {
                pending::<()>().await;
            }
        })
        .await;
    assert!(!report.is_success());
    assert!(report.all_direct_tasks_joined());
    assert_eq!(report.tasks.len(), 1);
    let task = &report.tasks[0];
    assert_eq!(task.name, "component");
    let expected = match exit {
        Exit::Early => TaskOutcome::UnexpectedExit,
        Exit::Error => TaskOutcome::Failed,
        Exit::Panic => TaskOutcome::Panicked,
        Exit::Abort => TaskOutcome::Aborted,
    };
    assert_eq!(task.outcome, expected);
    if matches!(exit, Exit::Abort) {
        assert_eq!(report.cause, ShutdownCause::Requested);
        assert_eq!(report.abort_requested, ["component"]);
        assert!(report.forced_cancellation);
    } else {
        assert_eq!(report.cause, ShutdownCause::ComponentExit("component"));
        assert!(report.abort_requested.is_empty());
    }
    check_cleanup(exit, &report, &events);
}

fn check_cleanup(exit: Exit, report: &ShutdownReport, events: &Events) {
    let task = &report.tasks[0];
    if matches!(exit, Exit::Panic | Exit::Abort) {
        assert!(
            task.error
                .as_ref()
                .unwrap()
                .downcast_ref::<tokio::task::JoinError>()
                .is_some()
        );
        assert!(snapshot(events).is_empty());
        assert!(report.cleanup.records.is_empty());
        assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
    } else {
        assert_eq!(snapshot(events), ["cleanup"]);
        assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Failed);
        assert!(
            report.cleanup.records[0]
                .error
                .as_ref()
                .unwrap()
                .is::<CleanupFailure>()
        );
        if matches!(exit, Exit::Error) {
            assert!(task.error.as_ref().unwrap().is::<ComponentFailure>());
        } else {
            assert!(task.error.is_none());
        }
    }
}

#[tokio::test(start_paused = true)]
async fn early_success_retains_failed_cleanup() {
    bounded(check_exit(Exit::Early)).await;
}

#[tokio::test(start_paused = true)]
async fn returned_failure_and_cleanup_failure_remain_concrete() {
    bounded(check_exit(Exit::Error)).await;
}

#[tokio::test(start_paused = true)]
async fn component_panic_retains_join_error_and_skips_cleanup() {
    bounded(check_exit(Exit::Panic)).await;
}

#[tokio::test(start_paused = true)]
async fn component_abort_retains_join_error_and_skips_cleanup() {
    bounded(check_exit(Exit::Abort)).await;
}
