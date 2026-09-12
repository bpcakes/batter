use super::{ProbeResult, transactions};
use batter::{
    BoxError,
    cleanup::CleanupBudget,
    command::{CommandCause, SharedCommandReport},
    operation::OperationContext,
};
use batter_example_reference_service::retirement::{
    self, DatabaseIdentity, RetirementError, RetirementReport,
};
use runledger_core::{
    jobs::{JobCompletion, JobContext, JobFailure, JobType},
    prelude::async_trait,
};
use runledger_runtime::{catalog::JobCatalog, registry::JobHandler};
use sqlx::{PgPool, postgres::PgConnectOptions, types::Uuid};
use std::time::Duration;

mod cases;
mod failures;
mod proxy;
pub use cases::{
    history_and_disable, pending_transaction, prepared_transaction, restricted_visibility,
    wrong_identity,
};
pub use failures::{commit_error_and_readback, lost_commit_acknowledgement};

const LEGACY: &str = "jobs.startup.control";
const SECOND: Duration = Duration::from_secs(1);
type Report = SharedCommandReport<RetirementReport, RetirementError>;

struct Legacy;
#[async_trait]
impl JobHandler for Legacy {
    fn job_type(&self) -> JobType<'static> {
        JobType::new(LEGACY)
    }
    async fn execute(
        &self,
        _: JobContext,
        _: serde_json::Value,
    ) -> Result<JobCompletion, JobFailure> {
        panic!("legacy catalog is test-only and must never execute")
    }
}

fn catalog() -> JobCatalog {
    JobCatalog::new().handler(Legacy)
}

async fn setup(pool: &PgPool) -> Result<(PgConnectOptions, DatabaseIdentity), BoxError> {
    batter_example_reference_service::schema::initialize_schema(pool).await?;
    catalog().sync_definitions(pool).await?;
    let (system, oid): (i64, i64) = sqlx::query_as(
        "SELECT system_identifier, oid::bigint FROM pg_control_system(), pg_database WHERE datname=current_database()"
    ).fetch_one(pool).await?;
    Ok((
        pool.connect_options().as_ref().clone(),
        DatabaseIdentity::new(system, u32::try_from(oid)?)?,
    ))
}

async fn enqueue_tx(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<Uuid, BoxError> {
    let payload = serde_json::json!({"retirement": true});
    let mut request = transactions::request(&payload, Uuid::nil());
    request.job_type = JobType::new(LEGACY);
    request.organization_id = None;
    request.idempotency_key = None;
    Ok(
        runledger_postgres::jobs::enqueue_job_with_outcome_tx(tx, &request)
            .await?
            .job_id,
    )
}

async fn enqueue(pool: &PgPool) -> Result<Uuid, BoxError> {
    let mut tx = transactions::read_committed(pool).await?;
    let job = enqueue_tx(&mut tx).await?;
    tx.commit().await?;
    Ok(job)
}

async fn retire(options: PgConnectOptions, identity: DatabaseIdentity) -> Result<Report, BoxError> {
    let command = retirement::prepare(
        options,
        identity,
        OperationContext::new(SECOND * 10)?,
        CleanupBudget::new(SECOND * 3, SECOND * 3, SECOND)?,
    )
    .start();
    let report = command.wait().await?;
    assert!(
        report
            .cleanup
            .as_ref()
            .is_ok_and(|cleanup| cleanup.is_success()),
        "maintenance cleanup must complete"
    );
    Ok(report)
}

fn failure(report: &Report) -> &RetirementError {
    match &report.work {
        Err(CommandCause::Failed(error)) => error,
        _ => panic!("expected concrete retirement failure: {report:?}"),
    }
}

async fn wait_idle(options: &PgConnectOptions) -> ProbeResult {
    use sqlx::Connection;
    let mut observer =
        sqlx::PgConnection::connect_with(&options.clone().database("postgres")).await?;
    let result = tokio::time::timeout(SECOND * 5, async {
        loop {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=$1 AND backend_type IS DISTINCT FROM 'autovacuum worker'")
                .bind(options.get_database()).fetch_one(&mut observer).await?;
            if count == 0 { return Ok::<_, sqlx::Error>(()); }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await;
    observer.close().await?;
    result??;
    Ok(())
}
