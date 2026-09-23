//! Opt-in external PostgreSQL coverage for opaque native database composition.
use batter_core::operation::OperationContext;
use batter_runledger::{PgAtomicError, run_atomic, verify_schema};
use runledger_core::jobs::JobType;
use runledger_postgres::jobs::JobEnqueueIntent;
use sqlx::Connection;
use std::time::Duration;
use tokio::time::{Instant, timeout};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test]
#[ignore = "requires a dedicated disposable PostgreSQL 18 database"]
async fn opaque_session_composes_intents_and_native_schema_verification() -> Result {
    let url = std::env::var("DATABASE_URL")?;
    // Retain an independent cleanup connection even if the body panics or closes its pool.
    let mut control = sqlx::PgConnection::connect(&url).await?;
    let suffix: String = sqlx::query_scalar("SELECT replace(uuidv7()::text, '-', '')")
        .fetch_one(&mut control)
        .await?;
    let table = format!("opaque_bridge_audit_{suffix}");
    let options: sqlx::postgres::PgConnectOptions = url.parse()?;
    let login = options.get_username();
    let profile = batter_runledger::PgSessionProfile::new(
        login,
        login,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )?;
    let database = batter_runledger::RunledgerDatabase::connect_lazy(
        options,
        profile,
        sqlx::postgres::PgPoolOptions::new().max_connections(1),
    )?;
    let pool = database.pool().clone();
    let body_pool = pool.clone();
    let body_table = table.clone();
    let body = tokio::spawn(async move {
        let pool = body_pool;
        let table = body_table;
        let context =
            batter_core::operation::OperationOwner::new(Duration::from_secs(30))?.into_context();
        runledger_postgres::migrate_after_idempotency_cutover(&database).await?;
        let _snapshot = verify_schema(&database).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE TABLE {table} (id integer PRIMARY KEY)"
        )))
        .execute(&pool)
        .await?;
        let payload = serde_json::json!({"synthetic": true});
        for (id, commit) in [(1, true), (2, false)] {
            let key = format!("{table}-{id}");
            let intent =
                JobEnqueueIntent::new(JobType::new("batter.opaque.intent"), &payload, &key);
            let result = run_atomic(&database, async |mut scope| {
                scope
                    .application(async |sql| {
                        sqlx::query(sqlx::AssertSqlSafe(format!(
                            "INSERT INTO {table} VALUES ($1)"
                        )))
                        .bind(id)
                        .execute(sql.executor())
                        .await?;
                        Ok::<_, sqlx::Error>(())
                    })
                    .await
                    .expect("application write");
                let recorded = scope
                    .record_required_job_enqueue_intent(&intent)
                    .await
                    .expect("intent");
                if commit { Ok(recorded) } else { Err("reject") }
            })
            .await;
            if commit {
                assert!(result.is_ok(), "commit acknowledged");
            } else {
                assert!(matches!(result, Err(PgAtomicError::Rejected("reject"))));
            }
            let retained: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM job_enqueue_intents WHERE idempotency_key=$1)",
            )
            .bind(&key)
            .fetch_one(&pool)
            .await?;
            let audit: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT EXISTS(SELECT 1 FROM {table} WHERE id=$1)"
            )))
            .bind(id)
            .fetch_one(&pool)
            .await?;
            assert_eq!((retained, audit), (commit, commit));
        }
        schema_cancellation_retires(&database, &context).await?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })
    .await
    .unwrap_or_else(|error| Err(error.into()));
    pool.close().await;
    let cleanup = cleanup_fixture(&mut control, &table).await;
    let close = control.close().await.map_err(Into::into);
    combine(body, combine(cleanup, close))
}

fn combine(body: Result, cleanup: Result) -> Result {
    batter_test_support::finish(
        body.map_err(std::io::Error::other),
        cleanup.map_err(std::io::Error::other),
    )
    .map_err(Into::into)
}

async fn cleanup_fixture(control: &mut sqlx::PgConnection, table: &str) -> Result {
    let keys = [format!("{table}-1"), format!("{table}-2")];
    let intents: Result = async {
        // A migration failure may precede creation of the intent table.
        let exists: bool = sqlx::query_scalar("SELECT to_regclass('job_enqueue_intents') IS NOT NULL")
            .fetch_one(&mut *control).await?;
        if exists {
            sqlx::query("DELETE FROM job_enqueue_intents WHERE job_type='batter.opaque.intent' AND organization_id IS NULL AND idempotency_key=ANY($1)")
                .bind(&keys[..]).execute(&mut *control).await?;
            let retained: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM job_enqueue_intents WHERE idempotency_key=ANY($1))")
                .bind(&keys[..]).fetch_one(&mut *control).await?;
            if retained { return Err("fixture intent survived cleanup".into()); }
        }
        Ok(())
    }.await;
    let audit: Result = async {
        // The identifier is generated above from PostgreSQL's hexadecimal UUID.
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP TABLE IF EXISTS {table}")))
            .execute(&mut *control)
            .await?;
        let retained: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(table)
            .fetch_one(&mut *control)
            .await?;
        if retained {
            return Err("fixture audit table survived cleanup".into());
        }
        Ok(())
    }
    .await;
    combine(intents, audit)
}

// Block the actual verifier on history access, then prove that cancellation
// retires that same backend before its server-side lock wait can complete.
async fn schema_cancellation_retires(
    database: &batter_runledger::RunledgerDatabase,
    _context: &OperationContext,
) -> Result {
    let pool = database.pool();
    let url = std::env::var("DATABASE_URL")?;
    let mut blocker = sqlx::PgConnection::connect(&url).await?;
    let mut observer = sqlx::PgConnection::connect(&url).await?;
    let mut lock = blocker.begin().await?;
    sqlx::query("LOCK TABLE _sqlx_migrations IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(pool)
        .await?;
    let body: Result = async {
        let mut check = Box::pin(verify_schema(database));
        tokio::select! {
            result = &mut check => { result?; return Err("schema verification did not block".into()); }
            result = observe_backend(&mut observer, pid, true) => result?,
        }
        drop(check);
        let replacement: i32 = timeout(Duration::from_secs(2),
            sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(pool)).await??;
        assert_ne!(pid, replacement, "cancelled schema session returned to the pool");
        timeout(Duration::from_secs(2), pool.close()).await?;
        assert!(pool.is_closed());
        // The client has retired, even though PostgreSQL may still be waiting.
        observe_backend(&mut observer, pid, true).await?;
        Ok(())
    }.await;
    let unlock = lock.rollback().await;
    let blocker_close = blocker.close().await;
    let disappeared = observe_backend(&mut observer, pid, false).await;
    let observer_close = observer.close().await;
    body?;
    unlock?;
    blocker_close?;
    disappeared?;
    observer_close?;
    Ok(())
}

async fn observe_backend(observer: &mut sqlx::PgConnection, pid: i32, blocked: bool) -> Result {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let observed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND (NOT $2 OR wait_event_type='Lock'))",
        )
        .bind(pid)
        .bind(blocked)
        .fetch_one(&mut *observer)
        .await?;
        if observed == blocked {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("schema backend did not reach the expected lock state".into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
