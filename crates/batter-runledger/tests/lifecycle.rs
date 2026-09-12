use batter::{
    cleanup::CleanupBudget,
    lifecycle::{Readiness, ShutdownBudget, Supervisor},
    operation::OperationContext,
};
use batter_runledger::{NativeReport, register};
use runledger_runtime::{config::JobsConfig, registry::JobRegistry};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

fn process() -> Supervisor {
    let second = Duration::from_secs(1);
    Supervisor::new(
        ShutdownBudget::new(
            second,
            second,
            second,
            CleanupBudget::new(second, second, second).unwrap(),
        )
        .unwrap(),
    )
}
fn context() -> OperationContext {
    OperationContext::new(Duration::from_secs(3)).unwrap()
}

fn config() -> JobsConfig {
    JobsConfig {
        worker_id: "adapter-test".into(),
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
async fn native_local_initialization_requires_no_database_or_durable_witness() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    pool.close().await;
    let mut process = process();
    let prepared = runledger_runtime::Supervisor::builder(&pool, config())
        .unwrap()
        .with_registry(JobRegistry::new())
        .disable_scheduler()
        .disable_reaper()
        .prepare()
        .unwrap();
    register(&mut process, "native", context(), prepared).unwrap();
    let running = process.start();
    assert_eq!(running.handle().readiness(), Readiness::Starting);
    running.handle().mark_ready();
    running.handle().wait_ready().await.unwrap();
    let report = running.shutdown().await.unwrap();
    assert!(report.is_success(), "{report}");
    let native = report.managed[0]
        .outcome
        .settlement
        .as_ref()
        .unwrap()
        .downcast_ref::<NativeReport>()
        .unwrap();
    assert_eq!(native.native.loops.len(), 2);
    assert!(native.native.is_cooperatively_stopped());
}

#[tokio::test(start_paused = true)]
async fn rejected_registration_and_unstarted_drop_never_build_native_work() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    pool.close().await;
    tokio::task::yield_now().await;
    let runtime = tokio::runtime::Handle::current();
    let before = runtime.metrics().num_alive_tasks();
    let prepare = || {
        runledger_runtime::Supervisor::builder(&pool, config())
            .unwrap()
            .with_registry(JobRegistry::new())
            .prepare()
            .unwrap()
    };
    let mut process = process();
    register(&mut process, "native", context(), prepare()).unwrap();
    assert!(register(&mut process, "native", context(), prepare()).is_err());
    tokio::task::yield_now().await;
    assert_eq!(runtime.metrics().num_alive_tasks(), before);
    drop(process);
    tokio::task::yield_now().await;
    assert_eq!(runtime.metrics().num_alive_tasks(), before);
}

#[tokio::test(start_paused = true)]
async fn native_validation_failure_keeps_original_error_and_runs_dependency_cleanup() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let mut process = process();
    let closed = pool.clone();
    process
        .on_cleanup("pool", move || async move {
            closed.close().await;
            Ok(())
        })
        .unwrap();
    let building = pool.clone();
    let second = Duration::from_secs(1);
    let mut starting = batter::startup::Startup::new(
        process,
        context(),
        CleanupBudget::new(second, second, second).unwrap(),
        move |scope| {
            Box::pin(async move {
                let prepared =
                    runledger_runtime::Supervisor::builder(&building, config())?.prepare()?;
                register(scope.supervisor(), "native", context(), prepared)?;
                Ok::<(), batter::BoxError>(())
            })
        },
    )
    .start();
    let error = match starting.wait().await {
        Ok(running) => {
            assert!(running.shutdown().await.unwrap().is_success());
            panic!("invalid native configuration accepted")
        }
        Err(error) => error,
    };
    let batter::startup::StartupError::Failed(report) = error else {
        panic!("startup report missing")
    };
    let batter::startup::StartupCause::Failed(error) = &report.cause else {
        panic!("native cause missing")
    };
    assert!(matches!(
        error.downcast_ref::<runledger_runtime::RuntimeError>(),
        Some(runledger_runtime::RuntimeError::MissingRegistry { .. })
    ));
    assert!(pool.is_closed());
    assert!(report.cleanup.is_success());
}
