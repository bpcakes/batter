use agent_consumer_exercise::{contract::*, launch, register_native};
use batter::{
    cleanup::CleanupBudget,
    command::CommandCause,
    lifecycle::{ShutdownBudget, Supervisor},
    operation::OperationContext,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
fn cleanup() -> CleanupBudget {
    CleanupBudget::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_millis(100),
    )
    .unwrap()
}
fn context() -> OperationContext {
    OperationContext::new(Duration::from_secs(10)).unwrap()
}
fn resource(work: Work, count: Arc<AtomicUsize>, fail: bool) -> Acquire {
    Box::new(move || {
        Box::pin(async move {
            Ok(Acquired {
                work,
                finish: Box::new(move || {
                    Box::pin(async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        if fail {
                            Err(std::io::Error::other("cleanup sentinel").into())
                        } else {
                            Ok(())
                        }
                    })
                }),
            })
        })
    })
}
#[tokio::test]
async fn original_failure_and_cleanup_failure_are_both_retained() {
    let count = Arc::new(AtomicUsize::new(0));
    let owner = launch(
        context(),
        cleanup(),
        resource(
            Box::pin(async { Err(Failure::Domain("original")) }),
            count.clone(),
            true,
        ),
    );
    let report = owner.wait().await.unwrap();
    assert!(matches!(
        report.work,
        Err(CommandCause::Failed(Failure::Domain("original")))
    ));
    assert!(!report.cleanup.as_ref().unwrap().is_success());
    assert!(!report.is_success());
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn owner_loss_cancels_work_but_not_finalization_or_parent() {
    let entered = Arc::new(Notify::new());
    let notify = entered.clone();
    let count = Arc::new(AtomicUsize::new(0));
    let parent = context();
    let owner = launch(
        parent.clone(),
        cleanup(),
        resource(
            Box::pin(async move {
                notify.notify_one();
                std::future::pending().await
            }),
            count.clone(),
            false,
        ),
    );
    entered.notified().await;
    let observer = owner.observer();
    drop(owner);
    let report = tokio::time::timeout(Duration::from_secs(2), observer.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(report.work, Err(CommandCause::Interrupted(_))));
    assert!(report.cleanup.as_ref().unwrap().is_success());
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(parent.check().is_ok());
}
#[tokio::test]
async fn cancelled_borrowed_wait_does_not_cancel_work() {
    let entered = Arc::new(Notify::new());
    let notify = entered.clone();
    let release = Arc::new(Notify::new());
    let gate = release.clone();
    let count = Arc::new(AtomicUsize::new(0));
    let owner = launch(
        context(),
        cleanup(),
        resource(
            Box::pin(async move {
                notify.notify_one();
                gate.notified().await;
                Ok(23)
            }),
            count.clone(),
            false,
        ),
    );
    entered.notified().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), owner.wait())
            .await
            .is_err()
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    release.notify_one();
    let report = owner.wait().await.unwrap();
    assert_eq!(report.work.as_ref().ok(), Some(&23));
    assert!(report.is_success());
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn panic_after_acquisition_still_finalizes() {
    let count = Arc::new(AtomicUsize::new(0));
    let owner = launch(
        context(),
        cleanup(),
        resource(
            Box::pin(async { panic!("controlled consumer panic") }),
            count.clone(),
            false,
        ),
    );
    let report = owner.wait().await.unwrap();
    assert!(matches!(report.work, Err(CommandCause::Panicked(_))));
    assert!(report.cleanup.as_ref().unwrap().is_success());
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
fn config() -> runledger_runtime::config::JobsConfig {
    runledger_runtime::config::JobsConfig {
        worker_id: "consumer".into(),
        poll_interval: Duration::from_millis(50),
        claim_batch_size: 1,
        lease_ttl_seconds: 30,
        max_global_concurrency: 1,
        reaper_interval: Duration::from_secs(1),
        schedule_poll_interval: Duration::from_secs(1),
        reaper_retry_delay_ms: 100,
    }
}
#[tokio::test(start_paused = true)]
async fn native_registration_is_inert_and_rejected_registration_stays_inert() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    pool.close().await;
    tokio::task::yield_now().await;
    let rt = tokio::runtime::Handle::current();
    let before = rt.metrics().num_alive_tasks();
    let second = Duration::from_secs(1);
    let mut process =
        Supervisor::new(ShutdownBudget::new(second, second, second, cleanup()).unwrap());
    register_native(
        &mut process,
        &pool,
        config(),
        runledger_runtime::registry::JobRegistry::new(),
        context(),
    )
    .unwrap();
    assert!(
        register_native(
            &mut process,
            &pool,
            config(),
            runledger_runtime::registry::JobRegistry::new(),
            context()
        )
        .is_err()
    );
    tokio::task::yield_now().await;
    assert_eq!(rt.metrics().num_alive_tasks(), before);
    let running = process.start();
    assert_ne!(
        running.handle().readiness(),
        batter::lifecycle::Readiness::Ready
    );
    running.handle().mark_ready();
    running.handle().wait_ready().await.unwrap();
    let report = running.shutdown().await.unwrap();
    assert!(report.is_success(), "{report}");
    assert!(
        report.managed[0]
            .outcome
            .settlement
            .as_ref()
            .unwrap()
            .downcast_ref::<batter_runledger::NativeReport>()
            .unwrap()
            .native
            .is_cooperatively_stopped()
    );
}
