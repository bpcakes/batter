use super::*;
use batter::{cleanup::CleanupOutcome, operation::Interruption};

fn context() -> OperationContext {
    OperationContext::new(Duration::from_secs(5)).unwrap()
}

fn assert_closed(report: &CommandReport) {
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].name, "socket");
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    assert!(report.cleanup.skipped.is_empty());
}

#[tokio::test]
async fn successful_command_retains_native_reply_and_cleanup() {
    let report = run(&context(), Work::Echo, false).await;
    assert!(report.is_success());
    assert_eq!(report.work.as_ref().unwrap(), b"prepare");
    assert_closed(&report);
}

#[tokio::test]
async fn work_and_cleanup_failures_both_remain_inspectable() {
    let report = run(&context(), Work::Fail, true).await;
    assert!(!report.is_success());
    assert!(matches!(
        report.work,
        Err(OperationError::Failed(CommandError::Work))
    ));
    assert_eq!(report.cleanup.records.len(), 1);
    let record = &report.cleanup.records[0];
    assert_eq!(record.outcome, CleanupOutcome::Failed);
    assert!(matches!(
        record.error.as_ref().unwrap().downcast_ref(),
        Some(CommandError::Cleanup)
    ));
    assert!(report.cleanup.skipped.is_empty());
}

#[tokio::test]
async fn cleanup_failure_keeps_the_successful_work_value() {
    let report = run(&context(), Work::Echo, true).await;
    assert!(!report.is_success());
    assert_eq!(report.work.as_ref().unwrap(), b"prepare");
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Failed);
}

#[tokio::test]
async fn work_failure_still_awaits_successful_cleanup() {
    let report = run(&context(), Work::Fail, false).await;
    assert!(!report.is_success());
    assert!(matches!(
        report.work,
        Err(OperationError::Failed(CommandError::Work))
    ));
    assert_closed(&report);
}

#[tokio::test]
async fn cancelled_command_still_awaits_successful_cleanup() {
    let context = context();
    let report = run(&context, Work::Cancel, false).await;
    assert_eq!(context.check(), Err(Interruption::Cancelled));
    assert!(matches!(
        report.work,
        Err(OperationError::Interrupted(Interruption::Cancelled))
    ));
    assert_closed(&report);
}

#[tokio::test(start_paused = true)]
async fn expired_command_still_has_a_separate_cleanup_budget() {
    let context = context();
    let report = run(&context, Work::Deadline, false).await;
    assert_eq!(context.check(), Err(Interruption::DeadlineExceeded));
    assert!(matches!(
        report.work,
        Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
    ));
    assert_closed(&report);
}
