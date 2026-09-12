use super::{
    CancellationFailure, DatabaseIdentity, DefinitionState, RetirementError, RetirementReport,
};
use batter::command::CommandScope;
use runledger_core::jobs::JobType;
use runledger_postgres::jobs::{
    JobCancellationScope, JobDefinitionUpdate, cancel_job_with_scope, update_job_definition,
};
use sqlx::{PgPool, postgres::PgConnectOptions, types::Uuid};

pub(super) const LEGACY_TYPE: &str = "jobs.startup.control";

pub(super) async fn run(
    scope: &mut CommandScope,
    options: PgConnectOptions,
    expected: DatabaseIdentity,
) -> Result<RetirementReport, RetirementError> {
    scope.stage("retirement.identity")?;
    let session = super::session::Session::new(scope, options)?;
    let result = retire(scope, &session, expected).await;
    session.finish(result)
}

async fn retire(
    scope: &mut CommandScope,
    session: &super::session::Session,
    expected: DatabaseIdentity,
) -> Result<RetirementReport, RetirementError> {
    let actual = session.verify(expected).await?;
    let pool = &session.pool;
    scope.stage("retirement.quiescence")?;
    quiescence(pool, 0).await?;

    scope.stage("retirement.disable")?;
    update_job_definition(
        pool,
        JobType::new(LEGACY_TYPE),
        &JobDefinitionUpdate {
            max_attempts: None,
            default_timeout_seconds: None,
            default_priority: None,
            is_enabled: Some(false),
        },
    )
    .await?;
    scope.stage("retirement.quiescence")?;
    quiescence(pool, 0).await?;

    let mut cancelled = 0;
    loop {
        scope.stage("retirement.cancel")?;
        quiescence(pool, cancelled).await?;
        let jobs: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM job_queue WHERE job_type = $1 AND organization_id IS NULL AND status IN ('PENDING', 'LEASED') ORDER BY id LIMIT 100"
        ).bind(LEGACY_TYPE).fetch_all(pool).await?;
        if jobs.is_empty() {
            break;
        }
        for job in jobs {
            if let Err(native) = cancel_job_with_scope(
                pool,
                JobCancellationScope::Global,
                job,
                Some("legacy startup control retired"),
            )
            .await
            {
                return Err(RetirementError::Cancellation(Box::new(
                    CancellationFailure {
                        job_id: job,
                        cancelled_before_failure: cancelled,
                        native,
                    },
                )));
            }
            cancelled += 1;
        }
    }
    scope.stage("retirement.verify")?;
    quiescence(pool, cancelled).await?;
    let enabled: Option<bool> =
        sqlx::query_scalar("SELECT is_enabled FROM job_definitions WHERE job_type = $1")
            .bind(LEGACY_TYPE)
            .fetch_optional(pool)
            .await?;
    let definition = match enabled {
        Some(true) => return Err(RetirementError::DefinitionEnabled),
        Some(false) => DefinitionState::Disabled,
        None => DefinitionState::Absent,
    };
    Ok(RetirementReport {
        identity: actual,
        definition,
        cancelled,
    })
}

async fn quiescence(pool: &PgPool, cancelled: u64) -> Result<(), RetirementError> {
    // Clear the statistics snapshot on every observation. A pool has exactly one
    // maintenance session, so every other target-database client is outside it.
    sqlx::query("SELECT pg_stat_clear_snapshot()")
        .execute(pool)
        .await?;
    let (other_backends, prepared_transactions): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND backend_type IS DISTINCT FROM 'autovacuum worker' AND pid <> pg_backend_pid()), (SELECT count(*) FROM pg_prepared_xacts WHERE database = current_database())"
    ).fetch_one(pool).await?;
    if other_backends != 0 || prepared_transactions != 0 {
        return Err(RetirementError::NotOffline {
            other_backends,
            prepared_transactions,
            cancelled,
        });
    }
    Ok(())
}
