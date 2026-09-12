use super::*;
use batter::{ConfigurationError, cleanup::SkipReason};
use tokio::time::Instant;

#[tokio::test(start_paused = true)]
async fn expired_work_is_inert_and_total_reserve_is_validated_before_work() {
    let command = Command::new(OperationContext::at(Instant::now()), budget(), |_| {
        panic!("expired work factory must remain inert");
        #[allow(unreachable_code)]
        Box::pin(async { Ok::<_, Infallible>(()) })
    })
    .start();
    assert!(matches!(
        command.wait().await.unwrap().work,
        Err(CommandCause::Interrupted(Interruption::DeadlineExceeded))
    ));
    let result = Command::within(
        OperationContext::new(Duration::from_secs(1)).unwrap(),
        budget(),
        |_| {
            panic!("invalid total must never run work");
            #[allow(unreachable_code)]
            Box::pin(async { Ok::<_, Infallible>(()) })
        },
    );
    assert!(matches!(result, Err(ConfigurationError::InvalidReserve)));
}

#[tokio::test(start_paused = true)]
async fn expired_work_gets_a_separate_complete_cleanup_budget() {
    let started = Instant::now();
    let command = Command::new(
        OperationContext::new(Duration::from_secs(2)).unwrap(),
        budget(),
        |scope| {
            Box::pin(async move {
                scope
                    .reserve_cleanup("resource")
                    .unwrap()
                    .register(|| async {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        Ok(())
                    });
                pending::<()>().await;
                Ok::<_, Infallible>(())
            })
        },
    )
    .start();
    let report = command.wait().await.unwrap();
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::DeadlineExceeded))
    ));
    assert!(report.cleanup.as_ref().unwrap().is_success());
    assert_eq!(Instant::now() - started, Duration::from_millis(2500));
}

#[tokio::test(start_paused = true)]
async fn total_reserve_includes_cleanup_abort_observation_without_parent_cancellation() {
    let total = OperationContext::new(Duration::from_secs(5)).unwrap();
    let end = total.deadline();
    let command = Command::within(total.clone(), budget(), move |scope| {
        Box::pin(async move {
            assert_eq!(scope.context().deadline(), end - Duration::from_secs(2));
            scope
                .reserve_cleanup("resource")
                .unwrap()
                .register(|| async {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    Ok(())
                });
            pending::<()>().await;
            Ok::<_, Infallible>(())
        })
    })
    .unwrap()
    .start();
    let report = command.wait().await.unwrap();
    assert_eq!(Instant::now(), end - Duration::from_millis(1500));
    assert!(report.cleanup.as_ref().unwrap().is_success());
    assert!(total.check().is_ok());
}

#[tokio::test(start_paused = true)]
async fn cleanup_timeout_keeps_work_value_and_skips_later_hooks_within_total() {
    let end = Instant::now() + Duration::from_secs(5);
    let command = Command::within(OperationContext::at(end), budget(), |scope| {
        Box::pin(async move {
            scope
                .reserve_cleanup("prerequisite")
                .unwrap()
                .register(|| async {
                    panic!("cleanup work budget exhausted");
                    #[allow(unreachable_code)]
                    Ok(())
                });
            scope
                .reserve_cleanup("dependent")
                .unwrap()
                .register(|| async { pending().await });
            Ok::<_, Infallible>(42)
        })
    })
    .unwrap()
    .start();
    let report = command.wait().await.unwrap();
    assert_eq!(*report.work.as_ref().unwrap(), 42);
    let cleanup = report.cleanup.as_ref().unwrap();
    assert_eq!(cleanup.records[0].outcome, CleanupOutcome::TimedOut);
    assert_eq!(cleanup.skipped[0].reason, SkipReason::BudgetExhausted);
    assert!(Instant::now() <= end);
    assert!(!report.is_success());
}
