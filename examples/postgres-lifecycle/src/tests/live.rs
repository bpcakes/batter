//! Explicit live checks: external provisioning, no schema changes or test bypass.
//! Each pool is created inside protected startup through the example's owner.
use super::*;
use tokio::sync::oneshot;

fn connection() -> PgConnectOptions {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        panic!("live checks require DATABASE_URL for an externally provisioned test database")
    });
    url.parse()
        .unwrap_or_else(|_| panic!("live checks require a valid DATABASE_URL"))
}

async fn owned_pool(receiver: oneshot::Receiver<PgPool>) -> PgPool {
    receiver
        .await
        .unwrap_or_else(|_| panic!("initializer did not publish its owned pool"))
}

fn assert_success_before_pool_inspection(result: &Result<(), BoxError>) {
    let Err(error) = result else {
        return;
    };
    if let Some(StartupError::Failed(failure)) =
        error.downcast_ref::<StartupError<InitializationError<ProcessFailure>>>()
    {
        panic!(
            "live startup failed before pool inspection at stage {}",
            failure.stage
        );
    }
    panic!("live process failed after startup before pool inspection");
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
    let connection = connection();
    let (publish, published) = oneshot::channel();
    let result = owned_startup(supervisor(false), move |scope| {
        Box::pin(async move {
            let result: Result<(), BoxError> = async {
                let pool = database_pool(scope, connection)?;
                let _ = publish.send(pool.clone());
                let one: i32 = sqlx::query_scalar("SELECT 1::integer")
                    .fetch_one(&pool)
                    .await?;
                if one != 1 {
                    return Err("unexpected live query result".into());
                }
                Ok(())
            }
            .await;
            process_result(result)
        })
    })
    .await;
    assert_success_before_pool_inspection(&result);
    let pool = owned_pool(published).await;
    assert_closed(&pool).await;
    assert_exit(result, ExitCode::SUCCESS);
}

#[tokio::test]
#[ignore = "requires an externally provisioned DATABASE_URL; run tests::live:: -- --ignored"]
async fn startup_database_error_retains_failed_cleanup_and_closes_the_pool() {
    let connection = connection();
    let (publish, published) = oneshot::channel();
    let result = owned_startup(supervisor(false), move |scope| {
        Box::pin(async move {
            let result: Result<(), BoxError> = async {
                let pool = database_pool(scope, connection)?;
                let _ = publish.send(pool.clone());
                scope
                    .reserve_cleanup("failed-finalizer")?
                    .register(|| async {
                        Err(std::io::Error::other("cleanup-credential-marker").into())
                    });
                sqlx::query("SELECT 1 / 0").execute(&pool).await?;
                Ok(())
            }
            .await;
            process_result(result)
        })
    })
    .await;
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
    let pool = owned_pool(published).await;
    assert_closed(&pool).await;
    assert_exit(result, ExitCode::FAILURE);
}

#[tokio::test]
#[ignore = "requires an externally provisioned DATABASE_URL; run tests::live:: -- --ignored"]
async fn task_database_error_survives_shutdown_and_pool_close() {
    let connection = connection();
    let (publish, published) = oneshot::channel();
    let supervisor = Supervisor::new(support::shutdown_budget());
    let result = owned_startup(supervisor, move |scope| {
        Box::pin(async move {
            let result: Result<(), BoxError> = async {
                let pool = database_pool(scope, connection)?;
                let _ = publish.send(pool.clone());
                scope
                    .registration()
                    .register("database-task", move |startup| async move {
                        let _shutdown = startup.acknowledge_started();
                        sqlx::query("SELECT 1 / 0").execute(&pool).await?;
                        Ok(_shutdown.stopped())
                    })?;
                Ok(())
            }
            .await;
            process_result(result)
        })
    })
    .await;
    let report = result
        .as_ref()
        .unwrap_err()
        .downcast_ref::<ShutdownFailure>()
        .map(shutdown_report)
        .unwrap();
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].name, "database-task");
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
    assert_division_error(report.tasks[0].error.as_ref().unwrap());
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].name, "postgres.pool");
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    let pool = owned_pool(published).await;
    assert_closed(&pool).await;
    assert_exit(result, ExitCode::FAILURE);
}
