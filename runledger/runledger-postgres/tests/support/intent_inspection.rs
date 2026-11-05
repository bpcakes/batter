//! Administrative inspection for tests that assert full promotion diagnostics.
//! Consumer correlation uses the protected atomic reader instead.
use runledger_postgres::jobs::{
    JobEnqueueIntentReadListFilter, JobEnqueueIntentRecord, JobReadScope,
    list_job_enqueue_intents_with_scope,
};
use runledger_postgres::{DbPool, Result};
use sqlx::types::Uuid;

pub async fn inspect_intent(
    pool: &DbPool,
    organization: Option<Uuid>,
    id: Uuid,
) -> Result<Option<JobEnqueueIntentRecord>> {
    let scope = organization.map_or(JobReadScope::Admin, JobReadScope::Organization);
    let mut offset = 0;
    loop {
        let rows = list_job_enqueue_intents_with_scope(
            pool,
            &JobEnqueueIntentReadListFilter::new(scope, 100, offset),
        )
        .await?;
        let count = rows.len();
        if let Some(row) = rows.into_iter().find(|row| row.id == id) {
            return Ok(Some(row));
        }
        if count < 100 {
            return Ok(None);
        }
        offset += 100;
    }
}

pub async fn record_postgres_server_version(pool: &sqlx::PgPool, diagnostic: &str) {
    let server_version = sqlx::query_scalar::<_, String>("SHOW server_version")
        .fetch_one(pool)
        .await
        .expect("read PostgreSQL server_version");
    let server_version_num =
        sqlx::query_scalar::<_, i32>("SELECT current_setting('server_version_num')::int")
            .fetch_one(pool)
            .await
            .expect("read PostgreSQL server_version_num");
    eprintln!(
        "{diagnostic} PostgreSQL server_version={server_version}, server_version_num={server_version_num}"
    );
}
