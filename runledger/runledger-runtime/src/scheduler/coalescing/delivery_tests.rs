use crate::scheduler::materialize_due_schedules;
use crate::test_logs::{LogBuffer, capture_logs};
use crate::{Error, SchedulerError};
use runledger_core::jobs::JobType;
use runledger_postgres::jobs::{JobDefinitionUpsert, upsert_job_definition_tx};
use runledger_test_support::{
    setup_ephemeral_pool_with_untracked_migrations as setup_ephemeral_pool, teardown_ephemeral_pool,
};
use sqlx::types::Uuid;

const JOB_TYPE: &str = "jobs.test.coalescing.delivery";
const MESSAGE: &str = "coalescing dropped missed occurrences";

async fn overdue_schedule(pool: &sqlx::PgPool) -> Uuid {
    let mut tx = pool.begin().await.expect("definition transaction");
    upsert_job_definition_tx(
        &mut tx,
        &JobDefinitionUpsert {
            job_type: JobType::new(JOB_TYPE),
            version: 1,
            max_attempts: 3,
            default_timeout_seconds: 60,
            default_priority: 100,
            is_enabled: true,
        },
    )
    .await
    .expect("definition");
    tx.commit().await.expect("definition committed");
    sqlx::query_scalar(
        "INSERT INTO job_schedules (name, job_type, cron_expr, next_fire_at)
         VALUES ('coalescing-delivery', $1, '* * * * * * *', now() - interval '1 day')
         RETURNING id",
    )
    .bind(JOB_TYPE)
    .fetch_one(pool)
    .await
    .expect("overdue schedule")
}

async fn materialized_count(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM job_queue WHERE job_type = $1")
        .bind(JOB_TYPE)
        .fetch_one(pool)
        .await
        .expect("durable job count")
}

#[tokio::test]
async fn coalescing_warning_follows_successful_materialization_commit() {
    let (pool, database) = setup_ephemeral_pool("coalescing_warning_commit", 3).await;
    let schedule = overdue_schedule(&pool).await;
    let log = LogBuffer::default();
    {
        let _capture = capture_logs(&log);
        assert_eq!(
            materialize_due_schedules(&pool, 8)
                .await
                .expect("commit pass"),
            1
        );
    }
    let text = log.text();
    let events: Vec<_> = text.lines().filter(|line| line.contains(MESSAGE)).collect();
    assert_eq!(events.len(), 1, "{text}");
    for field in [
        format!("schedule_id={schedule}"),
        "schedule_name=coalescing-delivery".to_owned(),
        "dropped_occurrences=1000".to_owned(),
        "dropped_occurrences_capped=true".to_owned(),
    ] {
        assert!(
            events[0].split_whitespace().any(|part| part == field),
            "{text}"
        );
    }
    assert_eq!(materialized_count(&pool).await, 1);
    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn rejected_commit_emits_no_coalescing_warning() {
    let (pool, database) = setup_ephemeral_pool("coalescing_warning_rejected_commit", 3).await;
    overdue_schedule(&pool).await;
    sqlx::raw_sql(
        "CREATE FUNCTION test_reject_schedule_commit() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'schedule commit rejected' USING ERRCODE = '23514'; END $$;
         CREATE CONSTRAINT TRIGGER test_reject_schedule_commit AFTER UPDATE ON job_schedules
         DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
         EXECUTE FUNCTION test_reject_schedule_commit();",
    )
    .execute(&pool)
    .await
    .expect("deferred commit failure");
    let log = LogBuffer::default();
    let result = {
        let _capture = capture_logs(&log);
        materialize_due_schedules(&pool, 8).await
    };
    assert!(
        matches!(result, Err(Error::Scheduler(SchedulerError::CommitTransaction {
        source: runledger_postgres::Error::CommitUnconfirmed(ref source),
    })) if source.sqlstate().as_deref() == Some("23514")),
        "{result:?}"
    );
    assert!(
        !log.text().contains(MESSAGE),
        "no durable coalescing: {}",
        log.text()
    );
    assert_eq!(materialized_count(&pool).await, 0);
    teardown_ephemeral_pool(pool, database).await;
}
