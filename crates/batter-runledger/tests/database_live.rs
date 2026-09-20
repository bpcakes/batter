//! Opt-in external PostgreSQL coverage for opaque native database composition.
use batter_core::operation::OperationContext;
use batter_runledger::{RunledgerTransaction, verify_schema};
use batter_sqlx::PgLease;
use runledger_core::jobs::JobType;
use runledger_postgres::jobs::{JobEnqueueIntent, record_job_enqueue_intent_in_transaction};
use std::time::Duration;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test]
#[ignore = "requires a dedicated disposable PostgreSQL 18 database"]
async fn opaque_session_composes_intents_and_native_schema_verification() -> Result {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("DATABASE_URL")?)
        .await?;
    let context = OperationContext::new(Duration::from_secs(30))?;
    let result: Result = async {
        runledger_postgres::migrate_after_idempotency_cutover(&pool).await?;
        PgLease::acquire(&pool, &context)
            .await?
            .with_connection(async |session| verify_schema(session).await)
            .await?;
        sqlx::query("CREATE TABLE opaque_bridge_audit (id integer PRIMARY KEY)")
            .execute(&pool)
            .await?;
        let payload = serde_json::json!({"synthetic": true});
        for (id, commit) in [(1, true), (2, false)] {
            let key = format!("batter-opaque-intent-{id}");
            let intent =
                JobEnqueueIntent::new(JobType::new("batter.opaque.intent"), &payload, &key);
            let intent_id = PgLease::acquire(&pool, &context)
                .await?
                .with_connection(async |session| {
                    let mut transaction = RunledgerTransaction::begin(session).await?;
                    sqlx::query("INSERT INTO opaque_bridge_audit VALUES ($1)")
                        .bind(id)
                        .execute(transaction.executor())
                        .await?;
                    let recorded =
                        record_job_enqueue_intent_in_transaction(&mut transaction, &intent).await?;
                    if commit {
                        transaction.commit().await?;
                    } else {
                        transaction.rollback().await?;
                    }
                    Ok::<_, Box<dyn std::error::Error + Send + Sync>>(recorded.intent_id)
                })
                .await?;
            let retained: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM job_enqueue_intents WHERE id=$1)")
                    .bind(intent_id)
                    .fetch_one(&pool)
                    .await?;
            let audit: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM opaque_bridge_audit WHERE id=$1)")
                    .bind(id)
                    .fetch_one(&pool)
                    .await?;
            assert_eq!((retained, audit), (commit, commit));
        }
        Ok(())
    }
    .await;
    pool.close().await;
    result
}
