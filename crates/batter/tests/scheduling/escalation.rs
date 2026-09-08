use super::support::Case;
use batter::{
    BoxError,
    cleanup::{CleanupBudget, SkipReason},
    lifecycle::{ProcessTaskError, ShutdownBudget, ShutdownReport, Supervisor, TaskOutcome},
};
use std::{
    convert::Infallible,
    future::{Future, pending},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;

pub const PHASE_ALLOWANCE: Duration = Duration::from_millis(25);

fn supervisor() -> Supervisor {
    let second = Duration::from_secs(1);
    let budget = ShutdownBudget::new(
        PHASE_ALLOWANCE,
        PHASE_ALLOWANCE,
        second,
        CleanupBudget::new(second, second, second).unwrap(),
    )
    .unwrap();
    let supervisor = Supervisor::with_process_capacity(budget, 2).unwrap();
    supervisor.handle().mark_ready();
    supervisor
}

struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// mode: 0 drain, 1 forced cooperative stop, 2 panic, 3 actual unfinished abort.
pub async fn outcomes(
    case: Case,
    mode: usize,
    completion: impl Future<Output = ()> + Send + 'static,
) -> Arc<ShutdownReport> {
    let mut supervisor = supervisor();
    let finalized = Arc::new(AtomicBool::new(false));
    let cleanup_flag = finalized.clone();
    supervisor
        .on_cleanup("dependent-resource", move || async move {
            cleanup_flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    process
        .try_spawn("completed", |_| async { Ok::<_, Infallible>(()) })
        .unwrap()
        .wait()
        .await
        .unwrap();
    let (started, ready) = oneshot::channel();
    let dropped = Arc::new(AtomicBool::new(false));
    let task_flag = dropped.clone();
    let receipt = process
        .try_spawn("affected-task", move |scope| async move {
            let _guard = Dropped(task_flag);
            started.send(()).unwrap();
            match mode {
                0 => scope.signal().draining().await,
                1 => scope.signal().cancelled().await,
                2 => panic!("generic scheduling panic fixture"),
                3 => pending::<()>().await,
                _ => unreachable!(),
            }
            completion.await;
            Ok::<_, Infallible>(())
        })
        .unwrap();
    ready.await.unwrap();
    running.handle().request();
    let result = receipt.wait().await;
    let report = running.wait().await.unwrap();
    assert!(dropped.load(Ordering::SeqCst));
    assert!(report.unjoined.is_empty());
    assert!(!report.abort_requested.contains(&"completed"));
    if mode < 2 && result.is_ok() {
        successful_exit(&report, mode == 1);
        assert_eq!(
            finalized.load(Ordering::SeqCst),
            report.abort_requested.is_empty()
        );
        case.event("cooperative-task-completed");
    } else {
        // A live worker may miss either short phase even though its task
        // cooperates. Exact success/phase assertions use a paused clock; live
        // termination must still retain the actual abort and skip cleanup.
        assert!(matches!(result, Err(ProcessTaskError::Terminated)));
        uncertain_exit(&report, mode == 2);
        if mode != 2 {
            assert!(report.forced_cancellation);
        }
        assert!(!finalized.load(Ordering::SeqCst));
        if mode < 2 {
            case.event("cooperative-task-aborted-after-deadline");
        }
    }
    case.event("escalation-outcomes-and-cleanup-reconciled");
    report
}

fn successful_exit(report: &ShutdownReport, forced: bool) {
    assert_eq!(report.completed_process_tasks, 2);
    assert!(report.tasks.is_empty());
    if forced || !report.abort_requested.is_empty() {
        assert!(report.forced_cancellation);
    }
    if report.abort_requested.is_empty() {
        assert!(report.is_success());
    } else {
        // An abort request can race with a task's final poll. Preserve the
        // successful result, but never turn that request into clean shutdown.
        assert_eq!(report.abort_requested, ["affected-task"]);
        assert!(!report.is_success());
        skipped_cleanup(report);
    }
}

fn uncertain_exit(report: &ShutdownReport, panicked: bool) {
    assert!(!report.is_success());
    assert_eq!(report.completed_process_tasks, 1);
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].name, "affected-task");
    assert_eq!(
        report.tasks[0].outcome,
        if panicked {
            TaskOutcome::Panicked
        } else {
            TaskOutcome::Aborted
        }
    );
    let error = report.tasks[0]
        .error
        .as_ref()
        .unwrap()
        .downcast_ref::<tokio::task::JoinError>()
        .unwrap();
    assert_eq!(error.is_panic(), panicked);
    assert_eq!(error.is_cancelled(), !panicked);
    // A panic in the final poll can also race with an abort request.
    if !panicked || !report.abort_requested.is_empty() {
        assert_eq!(report.abort_requested, ["affected-task"]);
        assert!(report.forced_cancellation);
    }
    skipped_cleanup(report);
}

fn skipped_cleanup(report: &ShutdownReport) {
    assert_eq!(report.cleanup.skipped.len(), 1);
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
}

pub async fn coordinator_failure(case: Case) {
    struct PanicOnDrop;
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("generic coordinator destructor fixture");
        }
    }
    let mut supervisor = supervisor();
    supervisor
        .register("pending-component", |_| pending::<Result<(), BoxError>>())
        .unwrap();
    let capture = PanicOnDrop;
    supervisor
        .on_cleanup("skipped-capture", move || async move {
            drop(capture);
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    let observer = running.observer();
    running.handle().request();
    let error = observer.wait().await.unwrap_err();
    assert!(error.is_panic());
    let again = observer.wait().await.unwrap_err();
    assert!(Arc::ptr_eq(&error, &again));
    case.event("coordinator-join-error-retained");
}

pub async fn unjoined_finite(case: Case) {
    let mut supervisor = supervisor();
    // The abort observation allowance is bounded, but the finite task never
    // returns from poll. The parent process owns termination of this fixture.
    let finalized = Arc::new(AtomicBool::new(false));
    let flag = finalized.clone();
    supervisor
        .on_cleanup("unjoined-dependent", move || async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let (entered, entry) = oneshot::channel();
    let dropped = Arc::new(AtomicBool::new(false));
    let flag = dropped.clone();
    let receipt = process
        .try_spawn("unjoined-finite", |_| async move {
            let _guard = Dropped(flag);
            entered.send(()).unwrap();
            loop {
                std::thread::park();
            }
            #[allow(unreachable_code)]
            Ok::<_, Infallible>(())
        })
        .unwrap();
    entry.await.unwrap();
    let report = running.shutdown().await.unwrap();
    assert!(!report.is_success());
    assert_eq!(report.abort_requested, ["unjoined-finite"]);
    assert_eq!(report.unjoined, ["unjoined-finite"]);
    assert_eq!(report.completed_process_tasks, 0);
    assert!(
        report.tasks.is_empty(),
        "unjoined work has no fabricated observed result"
    );
    assert_eq!(report.cleanup.skipped.len(), 1);
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
    assert!(!finalized.load(Ordering::SeqCst));
    assert!(!dropped.load(Ordering::SeqCst));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), receipt.wait())
            .await
            .is_err(),
        "receipt must remain pending while actual work cannot terminate"
    );
    case.event("unjoined-report-and-pending-receipt-reconciled");
    case.event("runtime-drop-after-unjoined");
}
