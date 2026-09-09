//! Compile controls: require the precise lint, rather than any compiler error.
#![deny(unfulfilled_lint_expectations)]

use batter::{
    BoxError,
    cleanup::{CleanupBudget, CleanupOutcome, CleanupStack},
    lifecycle::{RunningSupervisor, ShutdownReport, Supervisor, SupervisorObserver, TaskOutcome},
};
use std::sync::Arc;
use tokio::task::JoinError;

// These functions are typechecked, not executed: waiting is irrelevant to the
// lint contract. A missing must_use attribute leaves an unfulfilled expectation.
#[allow(dead_code)]
async fn ignored_raw_reports(stack: CleanupStack, budget: CleanupBudget, supervisor: Supervisor) {
    #[expect(unused_must_use)]
    stack.close(budget).await;
    #[expect(unused_must_use)]
    supervisor.run_until(async {}).await;
}

#[allow(dead_code)]
async fn ignored_after_unwrap(running: RunningSupervisor, observer: SupervisorObserver) {
    #[expect(unused_must_use)]
    running.wait().await.unwrap();
    #[expect(unused_must_use)]
    running.shutdown().await.unwrap();
    #[expect(unused_must_use)]
    observer.wait().await.unwrap();
}

#[allow(dead_code)]
async fn ignored_after_question_mark(
    running: RunningSupervisor,
    observer: SupervisorObserver,
) -> Result<(), Arc<JoinError>> {
    #[expect(unused_must_use)]
    running.wait().await?;
    #[expect(unused_must_use)]
    running.shutdown().await?;
    #[expect(unused_must_use)]
    observer.wait().await?;
    Ok(())
}

#[tokio::test]
async fn shared_report_retains_task_and_cleanup_failures_across_owners() {
    let mut supervisor = super::finite_supervisor(1);
    supervisor
        .on_cleanup("failed-cleanup", || async {
            Err(std::io::Error::other("cleanup-marker").into())
        })
        .unwrap();
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let observer = running.observer();
    let receipt = process
        .try_spawn("failed-work", |_| async {
            Err::<(), _>(std::io::Error::other("task-marker"))
        })
        .unwrap();
    assert!(receipt.wait().await.is_err());
    let report = running.wait().await.unwrap();
    let observed = observer.wait().await.unwrap();
    let cloned = report.clone();
    assert!(std::ptr::eq(&*report, &*observed));
    assert!(std::ptr::eq(&*report, &*cloned));
    drop((running, observer, report, observed));

    assert!(!cloned.is_success());
    assert_eq!(cloned.tasks[0].outcome, TaskOutcome::Failed);
    let task_error = cloned.tasks[0].error.as_ref().unwrap();
    assert_eq!(
        task_error
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .to_string(),
        "task-marker"
    );
    assert_eq!(cloned.cleanup.records[0].outcome, CleanupOutcome::Failed);
    assert_eq!(
        cloned.cleanup.records[0]
            .error
            .as_ref()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .to_string(),
        "cleanup-marker"
    );
    let summary = cloned.to_string();
    assert_eq!(summary, "owned shutdown report");
    assert!(!summary.contains("task-marker"));
    assert!(!summary.contains("cleanup-marker"));
    let boxed: BoxError = Box::new(cloned.clone());
    let source = boxed
        .source()
        .unwrap()
        .downcast_ref::<ShutdownReport>()
        .unwrap();
    assert!(std::ptr::eq(source, &*cloned));
    let mut chain = vec![boxed.to_string()];
    let mut next = boxed.source();
    while let Some(error) = next {
        chain.push(error.to_string());
        next = error.source();
    }
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0], "owned shutdown report");
    let rendered = chain.join("\nCaused by: ");
    assert_eq!(rendered.matches("1 task failure(s)").count(), 1);
    assert_eq!(rendered.matches("cleanup: 1 unsuccessful").count(), 1);
    assert!(!rendered.contains("task-marker"));
    assert!(!rendered.contains("cleanup-marker"));
}
