use super::*;
use batter::{
    cleanup::CleanupOutcome,
    command::{CommandCause, SharedCommandReport},
    operation::Interruption,
};
type CommandReport = SharedCommandReport<Vec<u8>, CommandError>;

fn context() -> OperationContext {
    OperationContext::new(Duration::from_secs(5)).unwrap()
}

fn assert_closed(report: &CommandReport) {
    assert_eq!(report.cleanup.as_ref().unwrap().records.len(), 1);
    assert_eq!(report.cleanup.as_ref().unwrap().records[0].name, "socket");
    assert_eq!(
        report.cleanup.as_ref().unwrap().records[0].outcome,
        CleanupOutcome::Succeeded
    );
    assert!(report.cleanup.as_ref().unwrap().skipped.is_empty());
}

#[tokio::test]
async fn successful_command_retains_native_reply_and_cleanup() {
    let report = start(context(), Work::Echo, false).wait().await.unwrap();
    assert!(report.is_success());
    assert_eq!(report.work.as_ref().unwrap(), b"prepare");
    assert_closed(&report);
}

#[tokio::test]
async fn work_and_cleanup_failures_both_remain_inspectable() {
    let report = start(context(), Work::Fail, true).wait().await.unwrap();
    assert!(!report.is_success());
    assert!(matches!(
        report.work,
        Err(CommandCause::Failed(CommandError::Work))
    ));
    assert_eq!(report.cleanup.as_ref().unwrap().records.len(), 1);
    let record = &report.cleanup.as_ref().unwrap().records[0];
    assert_eq!(record.outcome, CleanupOutcome::Failed);
    assert!(matches!(
        record.error.as_ref().unwrap().downcast_ref(),
        Some(CommandError::Cleanup)
    ));
    assert!(report.cleanup.as_ref().unwrap().skipped.is_empty());
}

#[tokio::test]
async fn cleanup_failure_keeps_the_successful_work_value() {
    let report = start(context(), Work::Echo, true).wait().await.unwrap();
    assert!(!report.is_success());
    assert_eq!(report.work.as_ref().unwrap(), b"prepare");
    assert_eq!(
        report.cleanup.as_ref().unwrap().records[0].outcome,
        CleanupOutcome::Failed
    );
}

#[tokio::test]
async fn work_failure_still_awaits_successful_cleanup() {
    let report = start(context(), Work::Fail, false).wait().await.unwrap();
    assert!(!report.is_success());
    assert!(matches!(
        report.work,
        Err(CommandCause::Failed(CommandError::Work))
    ));
    assert_closed(&report);
}

#[tokio::test]
async fn cancelled_command_still_awaits_successful_cleanup() {
    let context = context();
    let report = start(context.clone(), Work::Cancel, false)
        .wait()
        .await
        .unwrap();
    assert!(
        context.check().is_ok(),
        "command cancellation cannot reach its parent"
    );
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
    assert_closed(&report);
}

#[tokio::test(start_paused = true)]
async fn expired_command_still_has_a_separate_cleanup_budget() {
    let context = context();
    let report = start(context.clone(), Work::Deadline, false)
        .wait()
        .await
        .unwrap();
    assert_eq!(context.check(), Err(Interruption::DeadlineExceeded));
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::DeadlineExceeded))
    ));
    assert_closed(&report);
}
