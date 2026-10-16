//! Explicit live checks: external provisioning and no test bypass. Only the
//! schema-stage case applies the example's own migration and managed objects.
//! Each pool is created inside protected startup through the example's owner.
use super::*;
use batter::sqlx::verification::FindingKind;
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

/// The managed constraint trigger checks a note when its writer commits, so a
/// transaction can still correct a blank note, while a blank commit fails.
async fn note_checked_at_commit(pool: &sqlx::PgPool) -> Result<(), BoxError> {
    let mut transaction = pool.begin().await?;
    let id: i64 =
        sqlx::query_scalar("INSERT INTO lifecycle.records (note) VALUES ('') RETURNING id")
            .fetch_one(&mut *transaction)
            .await?;
    sqlx::query("UPDATE lifecycle.records SET note = 'corrected' WHERE id = $1")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    let blank = sqlx::query("INSERT INTO lifecycle.records (note) VALUES (' ')")
        .execute(pool)
        .await
        .expect_err("a blank note must fail when its transaction commits");
    match blank.as_database_error().and_then(|error| error.code()) {
        Some(code) if code == "23514" => Ok(()),
        _ => Err("the managed constraint trigger did not reject a blank note".into()),
    }
}

#[tokio::test]
#[ignore = "requires an externally provisioned DATABASE_URL; run tests::live:: -- --ignored"]
async fn schema_stage_applies_managed_objects_and_gates_readiness() {
    let connection = connection();
    let (publish, published) = oneshot::channel();
    let result = owned_startup(supervisor(false), move |scope| {
        Box::pin(async move {
            let result: Result<(), BoxError> = async {
                let pool = database_pool(scope, connection)?;
                let _ = publish.send(pool.clone());
                let context =
                    batter::operation::OperationOwner::new(Duration::from_secs(10))?.into_context();
                let objects = schema::managed_objects()?;
                let _ = schema::apply(&pool, &context, &objects).await?;
                schema::verify_release(&pool, &context, &objects).await?;
                // The managed trigger keeps updated_at current.
                let id: i64 = sqlx::query_scalar(
                    "INSERT INTO lifecycle.records (note, updated_at)
                     VALUES ('created', '2000-01-01') RETURNING id",
                )
                .fetch_one(&pool)
                .await?;
                let touched: bool = sqlx::query_scalar(
                    "UPDATE lifecycle.records SET note = 'updated' WHERE id = $1
                     RETURNING updated_at > '2000-01-01'",
                )
                .bind(id)
                .fetch_one(&pool)
                .await?;
                if !touched {
                    return Err("managed trigger did not run".into());
                }
                note_checked_at_commit(&pool).await?;
                // A repeated startup re-executes nothing.
                let again = schema::apply(&pool, &context, &objects).await?;
                if again.created_or_replaced() != 0 {
                    return Err("an unchanged startup re-executed managed objects".into());
                }
                // Out-of-band DDL fails the readiness gate until the next apply.
                sqlx::query("DROP TRIGGER records_touch_updated_at ON lifecycle.records")
                    .execute(&pool)
                    .await?;
                let refused = schema::verify_release(&pool, &context, &objects)
                    .await
                    .expect_err("drift must fail the readiness gate");
                let mismatch = refused
                    .downcast_ref::<schema::SchemaMismatch>()
                    .ok_or("readiness gate failed for another reason")?;
                if !mismatch
                    .report
                    .findings()
                    .iter()
                    .any(|finding| finding.kind == FindingKind::ManagedObjectDrift)
                {
                    return Err("dropped managed trigger was not reported as drift".into());
                }
                let repaired = schema::apply(&pool, &context, &objects).await?;
                if repaired.created_or_replaced() != 1 {
                    return Err("apply did not restore exactly the dropped trigger".into());
                }
                schema::verify_release(&pool, &context, &objects).await
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
