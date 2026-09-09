use runledger_core::jobs::{JobStage, JobType};
use runledger_postgres::jobs::{JobEnqueue, JobEnqueueDisposition, enqueue_job_with_outcome_tx};
use serde_json::{Value, json};
use sqlx::{
    PgPool, Postgres, Transaction,
    types::{
        Uuid,
        chrono::{TimeZone, Utc},
    },
};

use super::{ProbeResult, worker::catalog};

pub fn request(payload: &Value, owner: Uuid) -> JobEnqueue<'_> {
    JobEnqueue {
        job_type: JobType::new("jobs.compatibility.witness"),
        organization_id: Some(owner),
        payload,
        priority: None,
        max_attempts: None,
        timeout_seconds: None,
        next_run_at: None,
        idempotency_key: Some("compatibility-key"),
        stage: None,
    }
}

pub async fn read_committed(pool: &PgPool) -> Result<Transaction<'_, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *tx)
        .await?;
    let isolation: String = sqlx::query_scalar("SHOW transaction_isolation")
        .fetch_one(&mut *tx)
        .await?;
    assert_eq!(isolation, "read committed");
    Ok(tx)
}

pub async fn probe(pool: PgPool) -> ProbeResult {
    assert!(
        runledger_postgres::ensure_schema_compatible_after_idempotency_cutover(&pool)
            .await
            .is_err()
    );
    runledger_postgres::migrate_after_idempotency_cutover(&pool).await?;
    let mut migrations = sqlx::migrate!("./migrations");
    migrations.set_ignore_missing(true);
    migrations.run(&pool).await?;
    let before: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await?;
    assert!(before.len() > 1);
    runledger_postgres::migrate_after_idempotency_cutover(&pool).await?;
    migrations.run(&pool).await?;
    runledger_postgres::ensure_schema_compatible_after_idempotency_cutover(&pool).await?;
    let after: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await?;
    assert_eq!(before, after);
    catalog(None).sync_definitions(&pool).await?;
    enqueue_probe(&pool).await
}

async fn enqueue_probe(pool: &PgPool) -> ProbeResult {
    let payload = json!({"value": 1});
    let owner = Uuid::from_u128(1);
    rollback_probe(pool, &payload, owner).await?;
    let mut tx = read_committed(pool).await?;
    let inserted = enqueue_job_with_outcome_tx(&mut tx, &request(&payload, owner)).await?;
    assert_eq!(inserted.disposition, JobEnqueueDisposition::Inserted);
    tx.commit().await?;
    let mut tx = read_committed(pool).await?;
    let existing = enqueue_job_with_outcome_tx(&mut tx, &request(&payload, owner)).await?;
    assert_eq!(existing.disposition, JobEnqueueDisposition::Existing);
    assert_eq!(existing.job_id, inserted.job_id);
    tx.commit().await?;

    canonical_conflicts(pool, owner, inserted.job_id).await?;
    let mut tx = read_committed(pool).await?;
    let other =
        enqueue_job_with_outcome_tx(&mut tx, &request(&payload, Uuid::from_u128(2))).await?;
    assert_eq!(other.disposition, JobEnqueueDisposition::Inserted);
    assert_ne!(other.job_id, inserted.job_id);
    tx.commit().await?;
    Ok(())
}

async fn rollback_probe(pool: &PgPool, payload: &Value, owner: Uuid) -> ProbeResult {
    let mut tx = read_committed(pool).await?;
    let row = sqlx::query("INSERT INTO compatibility_probe (value) VALUES (1)")
        .execute(&mut *tx)
        .await?;
    assert_eq!(row.rows_affected(), 1);
    let first = enqueue_job_with_outcome_tx(&mut tx, &request(payload, owner)).await?;
    assert_eq!(first.disposition, JobEnqueueDisposition::Inserted);
    tx.rollback().await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM compatibility_probe")
        .fetch_one(pool)
        .await?;
    assert_eq!(count, 0);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM job_queue WHERE id = $1")
        .bind(first.job_id)
        .fetch_one(pool)
        .await?;
    assert_eq!(count, 0);

    Ok(())
}

async fn canonical_conflicts(pool: &PgPool, owner: Uuid, id: Uuid) -> ProbeResult {
    let payload = json!({"value": 1});
    let changed = json!({"value": 2});
    let original = request(&payload, owner);
    let mut cases = vec![original.clone(); 6];
    cases[0].payload = &changed;
    cases[1].priority = Some(12);
    cases[2].max_attempts = Some(2);
    cases[3].timeout_seconds = Some(30);
    cases[4].next_run_at = Some(
        Utc.timestamp_opt(1_800_000_000, 0)
            .single()
            .expect("fixed timestamp"),
    );
    cases[5].stage = Some(JobStage::Running);
    for case in cases {
        let mut tx = read_committed(pool).await?;
        let conflict = enqueue_job_with_outcome_tx(&mut tx, &case).await;
        assert!(
            matches!(&conflict, Err(runledger_postgres::Error::QueryError(error))
            if error.code() == "job.idempotency_conflict"),
            "canonical field change must report an idempotency conflict"
        );
        tx.rollback().await?;
    }
    snapshot_after_mutation(pool, &original, id).await?;
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;
    let rejected = enqueue_job_with_outcome_tx(&mut tx, &original).await;
    assert!(
        matches!(&rejected, Err(runledger_postgres::Error::QueryError(error))
        if error.code() == "job.enqueue_idempotency_unsupported_isolation")
    );
    tx.rollback().await?;
    Ok(())
}

async fn snapshot_after_mutation(
    pool: &PgPool,
    original: &JobEnqueue<'_>,
    id: Uuid,
) -> ProbeResult {
    let updated = sqlx::query("UPDATE job_queue SET priority = 321 WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    assert_eq!(updated.rows_affected(), 1);
    let mut tx = read_committed(pool).await?;
    let existing = enqueue_job_with_outcome_tx(&mut tx, original).await?;
    assert_eq!(existing.job_id, id);
    assert_eq!(existing.disposition, JobEnqueueDisposition::Existing);
    tx.commit().await?;
    let stored: Value = sqlx::query_scalar("SELECT enqueue_request FROM job_queue WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await?;
    assert_eq!(
        stored,
        json!({"payload":{"value":1}, "priority":null,
        "max_attempts":null, "timeout_seconds":null, "next_run_at":null, "stage":"queued"})
    );
    Ok(())
}
