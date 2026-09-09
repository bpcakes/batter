use super::*;
use batter::{cleanup::CleanupOutcome, lifecycle::TaskOutcome};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[path = "tests/errors.rs"]
mod errors;

#[path = "tests/live.rs"]
mod live;

fn supervisor(task_failure: bool) -> Supervisor {
    let mut supervisor = Supervisor::new(support::shutdown_budget());
    let handle = supervisor.handle();
    supervisor
        .register("component", move |shutdown| async move {
            shutdown.mark_started();
            if task_failure {
                Err(std::io::Error::other("task-credential-marker").into())
            } else {
                handle.request();
                Ok(())
            }
        })
        .unwrap();
    supervisor
}

fn assert_exit(result: Result<(), BoxError>, expected: ExitCode) {
    let mut output = Vec::new();
    assert_eq!(report_exit(result, &mut output), expected);
    if expected == ExitCode::SUCCESS {
        assert!(output.is_empty());
    } else {
        assert_eq!(output, b"Error: process failed\n");
    }
}

#[tokio::test]
async fn success_awaits_cleanup_before_returning_exit_success() {
    let mut supervisor = supervisor(false);
    let closed = Arc::new(AtomicBool::new(false));
    let flag = closed.clone();
    supervisor
        .on_cleanup("resource", move || async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let result = complete_startup(supervisor, Ok(())).await;
    assert!(closed.load(Ordering::SeqCst));
    assert_exit(result, ExitCode::SUCCESS);
}

#[tokio::test]
async fn startup_failure_retains_cause_and_failed_cleanup_before_exit() {
    let mut supervisor = supervisor(false);
    supervisor
        .on_cleanup("resource", || async {
            Err(std::io::Error::other("cleanup-credential-marker").into())
        })
        .unwrap();
    let result = complete_startup(
        supervisor,
        Err(std::io::Error::other("startup-credential-marker").into()),
    )
    .await;
    let failure = result
        .as_ref()
        .unwrap_err()
        .downcast_ref::<StartupFailure>()
        .unwrap();
    assert_eq!(
        failure
            .cause
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .to_string(),
        "startup-credential-marker"
    );
    assert_eq!(failure.cleanup.records.len(), 1);
    assert_eq!(failure.cleanup.records[0].outcome, CleanupOutcome::Failed);
    assert_eq!(
        failure.cleanup.records[0]
            .error
            .as_ref()
            .unwrap()
            .to_string(),
        "cleanup-credential-marker"
    );
    assert_exit(result, ExitCode::FAILURE);
}

#[tokio::test]
async fn task_failure_survives_successful_coordination_and_exits_failure() {
    let result = complete_startup(supervisor(true), Ok(())).await;
    let report = result
        .as_ref()
        .unwrap_err()
        .downcast_ref::<ShutdownFailure>()
        .map(|failure| &failure.report)
        .unwrap();
    assert!(!report.is_success());
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
    assert_eq!(
        report.tasks[0].error.as_ref().unwrap().to_string(),
        "task-credential-marker"
    );
    assert_exit(result, ExitCode::FAILURE);
}

#[tokio::test]
async fn cleanup_failure_survives_successful_tasks_and_exits_failure() {
    let mut supervisor = supervisor(false);
    supervisor
        .on_cleanup("resource", || async {
            Err(std::io::Error::other("cleanup-credential-marker").into())
        })
        .unwrap();
    let result = complete_startup(supervisor, Ok(())).await;
    let report = result
        .as_ref()
        .unwrap_err()
        .downcast_ref::<ShutdownFailure>()
        .map(|failure| &failure.report)
        .unwrap();
    assert!(!report.is_success());
    assert!(
        report
            .tasks
            .iter()
            .all(|task| task.outcome == TaskOutcome::Stopped)
    );
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Failed);
    assert_eq!(
        report.cleanup.records[0]
            .error
            .as_ref()
            .unwrap()
            .to_string(),
        "cleanup-credential-marker"
    );
    assert_exit(result, ExitCode::FAILURE);
}

#[test]
fn unavailable_diagnostic_sink_does_not_change_failure_status() {
    let result = Err(std::io::Error::other("credential-marker").into());
    // A zero-length slice is a writer that cannot accept the diagnostic.
    assert_eq!(report_exit(result, &mut &mut [][..]), ExitCode::FAILURE);
}

#[test]
fn exit_boundary_never_formats_or_traverses_an_unknown_error() {
    struct PrivateError;
    impl std::fmt::Debug for PrivateError {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("the CLI must not call Debug")
        }
    }
    impl std::fmt::Display for PrivateError {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("the CLI must not call Display")
        }
    }
    impl std::error::Error for PrivateError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            panic!("the CLI must not traverse unknown sources")
        }
    }
    assert_exit(Err(Box::new(PrivateError)), ExitCode::FAILURE);
}
