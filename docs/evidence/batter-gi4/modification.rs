use agent_consumer_exercise::{contract::*, launch_within};
use batter::{
    cleanup::CleanupBudget,
    command::CommandCause,
    operation::{Interruption, OperationContext},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
#[tokio::test(start_paused = true)]
async fn insufficient_total_rejects_before_acquisition() {
    let count = Arc::new(AtomicUsize::new(0));
    let acquired = count.clone();
    let acquire: Acquire = Box::new(move || {
        Box::pin(async move {
            acquired.fetch_add(1, Ordering::SeqCst);
            Err(Failure::Domain("should not run"))
        })
    });
    let cleanup = CleanupBudget::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap();
    assert!(
        launch_within(
            OperationContext::new(Duration::from_secs(1)).unwrap(),
            cleanup,
            acquire
        )
        .is_err()
    );
    tokio::task::yield_now().await;
    assert_eq!(count.load(Ordering::SeqCst), 0);
}
#[tokio::test(start_paused = true)]
async fn total_reserves_cleanup_and_does_not_cancel_parent() {
    let count = Arc::new(AtomicUsize::new(0));
    let finalized = count.clone();
    let acquire: Acquire = Box::new(move || {
        Box::pin(async move {
            Ok(Acquired {
                work: Box::pin(std::future::pending()),
                finish: Box::new(move || {
                    Box::pin(async move {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        finalized.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                }),
            })
        })
    });
    let total = OperationContext::new(Duration::from_millis(100)).unwrap();
    let cleanup = CleanupBudget::new(
        Duration::from_millis(30),
        Duration::from_millis(30),
        Duration::from_millis(10),
    )
    .unwrap();
    let start = tokio::time::Instant::now();
    let owner = launch_within(total.clone(), cleanup, acquire).unwrap();
    let report = owner.wait().await.unwrap();
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::DeadlineExceeded))
    ));
    assert!(report.cleanup.as_ref().unwrap().is_success());
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(tokio::time::Instant::now() - start < Duration::from_millis(100));
    assert!(
        total.check().is_ok(),
        "work deadline or cleanup cancelled the total parent"
    );
}
