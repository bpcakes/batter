//! Explicit live checks: external provisioning, no schema changes or test bypass.
use super::*;

async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        panic!("live checks require DATABASE_URL for an externally provisioned test database")
    });
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(3))
        .connect(&url)
        .await
        .unwrap_or_else(|_| panic!("live checks could not connect to the test database"))
}

async fn assert_closed(pool: &PgPool) {
    assert!(pool.is_closed());
    assert_eq!(pool.size(), 0);
    assert!(matches!(pool.acquire().await, Err(sqlx::Error::PoolClosed)));
}

fn assert_division_error(error: &BoxError) {
    let error = error.downcast_ref::<sqlx::Error>().unwrap();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("22012")
    );
}

#[tokio::test]
#[ignore = "requires an externally provisioned DATABASE_URL; run tests::live:: -- --ignored"]
async fn success_closes_the_live_pool_before_exit() {
    let pool = pool().await;
    let one: i32 = sqlx::query_scalar("SELECT 1::integer")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(one, 1);
    let mut supervisor = supervisor(false);
    register_pool_close(&mut supervisor, &pool).unwrap();
    let result = complete_startup(supervisor, Ok(())).await;
    assert_closed(&pool).await;
    assert_exit(result, ExitCode::SUCCESS);
}

#[tokio::test]
#[ignore = "requires an externally provisioned DATABASE_URL; run tests::live:: -- --ignored"]
async fn startup_database_error_retains_failed_cleanup_and_closes_the_pool() {
    let pool = pool().await;
    let mut supervisor = supervisor(false);
    register_pool_close(&mut supervisor, &pool).unwrap();
    supervisor
        .on_cleanup("failed-finalizer", || async {
            Err(std::io::Error::other("cleanup-credential-marker").into())
        })
        .unwrap();
    let cause = sqlx::query("SELECT 1 / 0")
        .execute(&pool)
        .await
        .unwrap_err();
    let result = complete_startup(supervisor, Err(cause.into())).await;
    let failure = startup_failure(result.as_ref().unwrap_err());
    assert_division_error(application_cause(failure));
    assert_eq!(failure.cleanup.records.len(), 2);
    assert_eq!(failure.cleanup.records[0].name, "failed-finalizer");
    assert_eq!(failure.cleanup.records[0].outcome, CleanupOutcome::Failed);
    assert_eq!(failure.cleanup.records[1].name, "postgres.pool");
    assert_eq!(
        failure.cleanup.records[1].outcome,
        CleanupOutcome::Succeeded
    );
    assert_closed(&pool).await;
    assert_exit(result, ExitCode::FAILURE);
}

#[tokio::test]
#[ignore = "requires an externally provisioned DATABASE_URL; run tests::live:: -- --ignored"]
async fn task_database_error_survives_shutdown_and_pool_close() {
    let pool = pool().await;
    let mut supervisor = Supervisor::new(support::shutdown_budget());
    register_pool_close(&mut supervisor, &pool).unwrap();
    let task_pool = pool.clone();
    supervisor
        .register("database-task", move |shutdown| async move {
            shutdown.mark_started();
            sqlx::query("SELECT 1 / 0").execute(&task_pool).await?;
            Ok(())
        })
        .unwrap();
    let result = complete_startup(supervisor, Ok(())).await;
    let report = result
        .as_ref()
        .unwrap_err()
        .downcast_ref::<ShutdownFailure>()
        .map(shutdown_report)
        .unwrap();
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
    assert_division_error(report.tasks[0].error.as_ref().unwrap());
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    assert_closed(&pool).await;
    assert_exit(result, ExitCode::FAILURE);
}
