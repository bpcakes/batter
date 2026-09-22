use batter_core::lifecycle::ComponentExit;
use batter_core::{
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
fn one_shot_readiness_approval_cannot_revive_a_draining_process() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    assert_eq!(handle.status().readiness(), Readiness::Starting);
    handle.request();
    approval.approve();
    assert_eq!(handle.status().readiness(), Readiness::Draining);
}

#[tokio::test]
async fn unapproved_owner_cannot_revive_a_stopped_process() {
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("short-lived", |startup| async { Ok(startup.abandon()) })
        .unwrap();
    let pending = supervisor.start_unapproved();
    let status = pending.status();
    let report = pending.wait().await.unwrap();
    assert_eq!(report.cause, ShutdownCause::ComponentExit("short-lived"));
    assert_eq!(status.readiness(), Readiness::Stopped);

    let running = pending.approve_readiness();
    assert_eq!(running.status().readiness(), Readiness::Stopped);
}

#[tokio::test(start_paused = true)]
async fn graceful_shutdown_stops_work_before_closing_dependencies() {
    let stopped = Arc::new(AtomicBool::new(false));
    let mut supervisor = Supervisor::new(budget());
    let flag = stopped.clone();
    supervisor
        .register("worker", move |startup| async move {
            let shutdown = startup.acknowledge_started();
            shutdown.draining().await;
            assert!(!shutdown.is_cancelled());
            flag.store(true, Ordering::SeqCst);
            Ok(shutdown.stopped())
        })
        .unwrap();
    supervisor
        .on_cleanup("pool", move || async move {
            assert!(stopped.load(Ordering::SeqCst));
            Ok(())
        })
        .unwrap();
    let handle = supervisor.handle();
    let report = supervisor.run_until(async {}).await;
    assert!(report.is_success());
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Stopped);
    assert_eq!(handle.status().readiness(), Readiness::Stopped);
    handle.request();
    assert_eq!(handle.status().readiness(), Readiness::Stopped);
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
    supervisor
        .register("early", |startup| async { Ok(startup.abandon()) })
        .unwrap();
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
            let _shutdown = signal.acknowledge_started();
            completed_rx.await.unwrap();
            assert!(!requesting.status().is_draining());
            requesting.request();
            Ok(_shutdown.stopped())
        })
        .unwrap();
    supervisor
        .register("early", move |signal| async move {
            let _shutdown = signal.acknowledge_started();
            completed_tx.send(()).unwrap();
            Ok(_shutdown.stopped())
        })
        .unwrap();
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
        .register("panic", |startup| async move {
            // Typed exit on a branch that is never taken; the poll then panics.
            if startup.shutdown().is_draining() {
                return Ok(startup.abandon());
            }
            panic!("critical defect")
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
        .register(
            "factory",
            |_| -> std::future::Ready<Result<ComponentExit, BoxError>> { panic!("factory defect") },
        )
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
        .register("drain-resistant", |startup| async move {
            let shutdown = startup.acknowledge_started();
            shutdown.cancelled().await;
            assert!(shutdown.is_draining());
            Ok(shutdown.stopped())
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
        .register("ignores-signals", |_| {
            pending::<Result<ComponentExit, BoxError>>()
        })
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
    let admitted = Arc::new(std::sync::Mutex::new(None::<OperationContext>));
    let inside = admitted.clone();
    supervisor
        .register("admitted", |startup| async move {
            let shutdown = startup.acknowledge_started();
            shutdown.draining().await;
            assert!(inside.lock().unwrap().take().unwrap().check().is_ok());
            Ok(shutdown.stopped())
        })
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let owner = running
        .operation_admission()
        .admit_root(batter_core::operation::RootDeadline::at(
            tokio::time::Instant::now() + Duration::from_secs(1),
        ))
        .unwrap();
    let context = owner.context().clone();
    *admitted.lock().unwrap() = Some(context.clone());
    let report = running.shutdown().await.unwrap();
    assert!(report.is_success());
    assert_eq!(context.check(), Err(Interruption::Cancelled));
}

#[tokio::test(start_paused = true)]
async fn cleanup_failure_makes_otherwise_clean_shutdown_unsuccessful() {
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("worker", |startup| async move {
            let shutdown = startup.acknowledge_started();
            shutdown.draining().await;
            Ok(shutdown.stopped())
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
        .register("worker", move |startup| {
            flag.store(true, Ordering::SeqCst);
            async move {
                let shutdown = startup.acknowledge_started();
                shutdown.draining().await;
                Ok(shutdown.stopped())
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
    supervisor
        .register("worker", |startup| async { Ok(startup.abandon()) })
        .unwrap();
    assert!(
        supervisor
            .register("worker", |startup| async { Ok(startup.abandon()) })
            .is_err()
    );
    assert!(
        supervisor
            .register("", |startup| async { Ok(startup.abandon()) })
            .is_err()
    );
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

fn assert_unpolled_caller_owned_drop(unapproved: bool) {
    let called = Arc::new(AtomicBool::new(false));
    let cleanup_called = Arc::new(AtomicBool::new(false));
    let mut supervisor = Supervisor::new(budget());
    let flag = called.clone();
    supervisor
        .register("inert-component", move |startup| {
            flag.store(true, Ordering::SeqCst);
            async { Ok(startup.abandon()) }
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
    let cancellation = handle.signal();
    assert_eq!(handle.status().readiness(), Readiness::Starting);
    assert!(!cancellation.is_cancelled());
    if unapproved {
        drop(supervisor.run_until_unapproved(pending()));
    } else {
        drop(supervisor.run_until(pending()));
    }
    assert_eq!(handle.status().readiness(), Readiness::Draining);
    assert!(cancellation.is_cancelled());
    assert!(!called.load(Ordering::SeqCst));
    assert!(!cleanup_called.load(Ordering::SeqCst));
}

#[test]
fn dropping_unpolled_ordinary_driver_signals_before_application_captures() {
    assert_unpolled_caller_owned_drop(false);
}

#[test]
fn dropping_unpolled_unapproved_driver_signals_before_application_captures() {
    assert_unpolled_caller_owned_drop(true);
}

#[tokio::test(start_paused = true)]
async fn dropping_unapproved_started_owner_requests_drain_before_driver_poll() {
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("drain-observer", |startup| async move {
            let shutdown = startup.acknowledge_started();
            shutdown.draining().await;
            Ok(shutdown.stopped())
        })
        .unwrap();
    let status = supervisor.status();
    let pending = supervisor.start_unapproved();
    let observer = pending.observer();
    assert_eq!(status.readiness(), Readiness::Starting);

    drop(pending);
    assert_eq!(status.readiness(), Readiness::Draining);
    assert!(observer.wait().await.unwrap().is_success());
    assert_eq!(status.readiness(), Readiness::Stopped);
}

#[tokio::test]
async fn aborting_driver_before_first_poll_notifies_readiness_waiters() {
    let supervisor = Supervisor::with_process_capacity(
        budget(),
        batter_core::lifecycle::ProcessCapacity::new(1).unwrap(),
    );
    let handle = supervisor.handle();
    let cancellation = handle.signal();
    // On this current-thread runtime, abort runs before the coordinator polls.
    let task = tokio::spawn(supervisor.run_until(pending()));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let readiness = tokio::time::timeout(Duration::from_millis(100), handle.status().wait_ready())
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
        .register("component", |startup| async move {
            let signal = startup.acknowledge_started();
            signal.draining().await;
            Ok(signal.stopped())
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
