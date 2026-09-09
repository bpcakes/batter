use batter::{
    BoxError,
    cleanup::{CleanupBudget, CleanupOutcome, SkipReason},
    lifecycle::{
        Readiness, ShutdownBudget, ShutdownCause, ShutdownHandle, Supervisor, TaskOutcome,
    },
    operation::{Interruption, OperationContext},
};
use std::{
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn budget() -> ShutdownBudget {
    ShutdownBudget::new(
        Duration::from_millis(10),
        Duration::from_millis(10),
        Duration::from_millis(10),
        CleanupBudget::new(
            Duration::from_secs(1),
            Duration::from_millis(100),
            Duration::from_millis(10),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn readiness_cannot_revive_a_draining_process() {
    let handle = ShutdownHandle::new();
    assert_eq!(handle.readiness(), Readiness::Starting);
    assert!(handle.mark_ready());
    assert!(!handle.mark_ready());
    handle.request();
    assert!(!handle.mark_ready());
    assert_eq!(handle.readiness(), Readiness::Draining);
}

#[tokio::test(start_paused = true)]
async fn graceful_shutdown_stops_work_before_closing_dependencies() {
    let stopped = Arc::new(AtomicBool::new(false));
    let mut supervisor = Supervisor::new(budget());
    let flag = stopped.clone();
    supervisor
        .register("worker", move |shutdown| async move {
            shutdown.draining().await;
            assert!(!shutdown.is_cancelled());
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    supervisor
        .on_cleanup("pool", move || async move {
            assert!(stopped.load(Ordering::SeqCst));
            Ok(())
        })
        .unwrap();
    let handle = supervisor.handle();
    handle.mark_ready();
    let report = supervisor.run_until(async {}).await;
    assert!(report.is_success());
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Stopped);
    assert_eq!(handle.readiness(), Readiness::Stopped);
    handle.request();
    assert_eq!(handle.readiness(), Readiness::Stopped);
}

#[tokio::test(start_paused = true)]
async fn all_component_errors_survive_shutdown() {
    let mut supervisor = Supervisor::new(budget());
    for name in ["first", "second"] {
        supervisor
            .register(name, move |_| async move {
                Err(std::io::Error::other(name).into())
            })
            .unwrap();
    }
    let report = supervisor.run_until(pending()).await;
    assert!(!report.is_success());
    assert_eq!(report.tasks.len(), 2);
    assert_eq!(
        report.cause,
        ShutdownCause::ComponentExit(report.tasks[0].name)
    );
    assert!(
        report
            .tasks
            .iter()
            .all(|task| task.outcome == TaskOutcome::Failed && task.error.is_some())
    );
}

#[tokio::test(start_paused = true)]
async fn early_success_is_a_critical_failure() {
    let mut supervisor = Supervisor::new(budget());
    supervisor.register("early", |_| async { Ok(()) }).unwrap();
    let report = supervisor.run_until(pending()).await;
    assert_eq!(report.cause, ShutdownCause::ComponentExit("early"));
    assert_eq!(report.tasks[0].outcome, TaskOutcome::UnexpectedExit);
    assert!(!report.is_success());
}

#[tokio::test]
async fn early_exit_remains_a_failure_when_drain_precedes_join_observation() {
    let mut supervisor = Supervisor::new(budget());
    let handle = supervisor.handle();
    let (completed_tx, completed_rx) = tokio::sync::oneshot::channel();
    let requesting = handle.clone();
    supervisor
        .register("requestor", move |signal| async move {
            signal.mark_started();
            completed_rx.await.unwrap();
            assert!(!requesting.is_draining());
            requesting.request();
            Ok(())
        })
        .unwrap();
    supervisor
        .register("early", move |signal| async move {
            signal.mark_started();
            completed_tx.send(()).unwrap();
            Ok(())
        })
        .unwrap();
    handle.mark_ready();
    let report = supervisor.run_until(pending()).await;
    assert_eq!(
        report
            .tasks
            .iter()
            .find(|task| task.name == "early")
            .unwrap()
            .outcome,
        TaskOutcome::UnexpectedExit
    );
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn component_panic_conservatively_skips_resource_finalizers() {
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("panic", |_| async {
            panic!("critical defect");
            #[allow(unreachable_code)]
            Ok(())
        })
        .unwrap();
    supervisor
        .on_cleanup("pool", || async {
            panic!("must be skipped");
            #[allow(unreachable_code)]
            Ok(())
        })
        .unwrap();
    let report = supervisor.run_until(pending()).await;
    assert_eq!(report.cause, ShutdownCause::ComponentExit("panic"));
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Panicked);
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn component_factory_panic_is_observed_by_name() {
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("factory", |_| -> std::future::Ready<Result<(), BoxError>> {
            panic!("factory defect")
        })
        .unwrap();
    let report = supervisor.run_until(pending()).await;
    assert_eq!(report.cause, ShutdownCause::ComponentExit("factory"));
    assert_eq!(report.tasks[0].name, "factory");
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Panicked);
}

#[tokio::test(start_paused = true)]
async fn forced_cooperative_cancellation_can_still_join_cleanly() {
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("drain-resistant", |shutdown| async move {
            shutdown.cancelled().await;
            assert!(shutdown.is_draining());
            Ok(())
        })
        .unwrap();
    let report = supervisor.run_until(async {}).await;
    assert!(report.forced_cancellation);
    assert!(report.abort_requested.is_empty());
    assert!(report.is_success());
}

#[tokio::test(start_paused = true)]
async fn noncooperative_async_task_is_aborted_and_reported() {
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("ignores-signals", |_| pending::<Result<(), BoxError>>())
        .unwrap();
    supervisor
        .on_cleanup("pool", || async {
            panic!("must skip after abort");
            #[allow(unreachable_code)]
            Ok(())
        })
        .unwrap();
    let report = supervisor.run_until(async {}).await;
    assert_eq!(report.abort_requested, vec!["ignores-signals"]);
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Aborted);
    assert!(report.all_direct_tasks_joined());
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn drain_does_not_cancel_previously_admitted_contexts() {
    let mut supervisor = Supervisor::new(budget());
    let handle = supervisor.handle();
    let token = handle.operation_token();
    let context =
        OperationContext::under(tokio::time::Instant::now() + Duration::from_secs(1), &token);
    let inside = context.clone();
    supervisor
        .register("admitted", |shutdown| async move {
            shutdown.draining().await;
            assert!(inside.check().is_ok());
            Ok(())
        })
        .unwrap();
    let report = supervisor.run_until(async {}).await;
    assert!(report.is_success());
    assert_eq!(context.check(), Err(Interruption::Cancelled));
}

#[tokio::test(start_paused = true)]
async fn cleanup_failure_makes_otherwise_clean_shutdown_unsuccessful() {
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("worker", |shutdown| async move {
            shutdown.draining().await;
            Ok(())
        })
        .unwrap();
    supervisor
        .on_cleanup("pool", || async {
            Err(std::io::Error::other("cleanup").into())
        })
        .unwrap();
    let report = supervisor.run_until(async {}).await;
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Failed);
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn no_tasks_is_not_silent_success() {
    let report = Supervisor::new(budget()).run_until(pending()).await;
    assert_eq!(report.cause, ShutdownCause::EmptySupervisor);
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn registration_is_inert_until_run() {
    let started = Arc::new(AtomicBool::new(false));
    let flag = started.clone();
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("worker", move |shutdown| {
            flag.store(true, Ordering::SeqCst);
            async move {
                shutdown.draining().await;
                Ok(())
            }
        })
        .unwrap();
    assert!(!started.load(Ordering::SeqCst));
    assert!(supervisor.run_until(async {}).await.is_success());
    assert!(started.load(Ordering::SeqCst));
}

#[test]
fn component_registration_rejects_duplicate_names() {
    let mut supervisor = Supervisor::new(budget());
    supervisor.register("worker", |_| async { Ok(()) }).unwrap();
    assert!(supervisor.register("worker", |_| async { Ok(()) }).is_err());
    assert!(supervisor.register("", |_| async { Ok(()) }).is_err());
}

#[tokio::test(start_paused = true)]
async fn partial_startup_can_extract_and_close_registered_resources() {
    let mut supervisor = Supervisor::new(budget());
    supervisor.on_cleanup("pool", || async { Ok(()) }).unwrap();
    let stack = supervisor.take_cleanup();
    assert_eq!(stack.len(), 1);
    let report = stack
        .close(
            CleanupBudget::new(
                Duration::from_secs(1),
                Duration::from_secs(1),
                Duration::from_millis(10),
            )
            .unwrap(),
        )
        .await;
    assert!(report.is_success());
    assert!(supervisor.take_cleanup().is_empty());
}

#[test]
fn dropping_unpolled_driver_signals_shutdown_without_starting_factories() {
    let called = Arc::new(AtomicBool::new(false));
    let cleanup_called = Arc::new(AtomicBool::new(false));
    let mut supervisor = Supervisor::new(budget());
    let flag = called.clone();
    supervisor
        .register("inert-component", move |_| {
            flag.store(true, Ordering::SeqCst);
            async { Ok(()) }
        })
        .unwrap();
    let flag = cleanup_called.clone();
    supervisor
        .on_cleanup("inert-finalizer", move || {
            flag.store(true, Ordering::SeqCst);
            async { Ok(()) }
        })
        .unwrap();
    let handle = supervisor.handle();
    let cancellation = handle.operation_token();
    handle.mark_ready();
    let driver = supervisor.run_until(pending());
    assert_eq!(handle.readiness(), Readiness::Starting);
    assert!(!cancellation.is_cancelled());
    drop(driver);
    assert_eq!(handle.readiness(), Readiness::Draining);
    assert!(cancellation.is_cancelled());
    assert!(!called.load(Ordering::SeqCst));
    assert!(!cleanup_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn aborting_driver_before_first_poll_notifies_readiness_waiters() {
    let supervisor = Supervisor::with_process_capacity(budget(), 1).unwrap();
    let handle = supervisor.handle();
    let cancellation = handle.operation_token();
    handle.mark_ready();
    // On this current-thread runtime, abort runs before the coordinator polls.
    let task = tokio::spawn(supervisor.run_until(pending()));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let readiness = tokio::time::timeout(Duration::from_millis(100), handle.wait_ready())
        .await
        .expect("readiness waiters must be notified when their driver is dropped");
    assert_eq!(readiness, Err(Readiness::Draining));
    assert!(cancellation.is_cancelled());
}

#[tokio::test]
async fn driver_still_accepts_a_borrowed_non_send_shutdown_future() {
    let mut requested = std::rc::Rc::new(false);
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("component", |signal| async move {
            signal.draining().await;
            Ok(())
        })
        .unwrap();
    let report = supervisor
        .run_until(async {
            *std::rc::Rc::make_mut(&mut requested) = true;
        })
        .await;
    assert!(*requested);
    assert!(report.is_success());
}

#[tokio::test]
async fn component_failure_retains_shutdown_future_until_cleanup_finishes() {
    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let guard = DropFlag(dropped.clone());
    let shutdown = async move {
        let _guard = guard;
        pending::<()>().await;
    };
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("component", |_| async {
            Err(std::io::Error::other("component failed").into())
        })
        .unwrap();
    let observed = dropped.clone();
    supervisor
        .on_cleanup("dependency", move || async move {
            assert!(!observed.load(Ordering::SeqCst));
            Ok(())
        })
        .unwrap();

    let report = supervisor.run_until(shutdown).await;
    assert_eq!(report.cause, ShutdownCause::ComponentExit("component"));
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
    assert!(report.tasks[0].error.is_some());
    assert_eq!(report.cleanup.records.len(), 1);
    assert!(report.cleanup.is_success());
    assert!(dropped.load(Ordering::SeqCst));
}
