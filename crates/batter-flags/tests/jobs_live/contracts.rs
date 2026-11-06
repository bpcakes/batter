use super::support::*;
use batter_flags::{
    core::{CatalogBuilder, SettingsPrefix, SwitchState, Toggle, Version},
    jobs::DirectWhenTripped,
    test::TestFlags,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::atomic::Ordering;

pub async fn continuation_and_release(pool: &PgPool) -> Result {
    const NAME: &str = "flags.live.continuation";
    define(pool, NAME, 3).await?;
    let payload = json!({"work": "continuation"});
    let first = enqueue_job_with_execution_resource(pool, &request(NAME, &payload), "flags:shared")
        .await?
        .job_id;
    let sibling =
        enqueue_job_with_execution_resource(pool, &request(NAME, &payload), "flags:shared")
            .await?
            .job_id;
    let checkpoint = json!({"cursor": 42});
    sqlx::query("UPDATE job_queue SET checkpoint=$2 WHERE id=$1")
        .bind(first)
        .bind(&checkpoint)
        .execute(pool)
        .await?;
    let (flags, halt) = flags(SwitchState::Engaged)?;
    let probe = Probe::new(NAME);
    let calls = probe.calls.clone();
    let body_pool = pool.clone();
    // Native completion schedules from clock_timestamp(), whereas an attempt's
    // finished_at is transaction-start now(). Bracket the completion clock with
    // server observations; subtracting finished_at adds transaction elapsed time.
    let before: String = sqlx::query_scalar("SELECT clock_timestamp()::text")
        .fetch_one(pool)
        .await?;
    drive(pool, flags.gated_direct(probe, &halt, policy(DirectWhenTripped::Defer(recheck())))?, async move {
        wait_row(&body_pool, first, "PENDING", 2).await?;
        wait_row(&body_pool, sibling, "PENDING", 2).await?;
        for id in [first, sibling] {
            let row: (i32, String, bool) = sqlx::query_as("SELECT attempt, stage, next_run_at BETWEEN $2::text::timestamptz + interval '30 seconds' AND clock_timestamp() + interval '37.5 seconds' FROM job_queue WHERE id=$1").bind(id).bind(&before).fetch_one(&body_pool).await?;
            assert_eq!((row.0, row.1.as_str()), (0, "queued"));
            assert!(row.2, "continuation outside the observed completion-clock window");
            let claims: i64 = sqlx::query_scalar("SELECT count(*) FROM job_execution_resource_claims WHERE job_id=$1").bind(id).fetch_one(&body_pool).await?;
            assert_eq!(claims, 0);
        }
        let retained: Value = sqlx::query_scalar("SELECT checkpoint FROM job_queue WHERE id=$1").bind(first).fetch_one(&body_pool).await?;
        assert_eq!(retained, checkpoint);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        Ok(())
    }).await?;
    // Rebuild the fixture as released; the next native claims run the retained
    // jobs at their persisted next_run_at, with no clock or queue manipulation.
    let (released, halt) = super::support::flags(SwitchState::Released)?;
    let probe = Probe::new(NAME);
    let calls = probe.calls.clone();
    let body_pool = pool.clone();
    drive(
        pool,
        released.gated_direct(probe, &halt, policy(DirectWhenTripped::Defer(recheck())))?,
        async move {
            wait_row(&body_pool, first, "SUCCEEDED", 2).await?;
            wait_row(&body_pool, sibling, "SUCCEEDED", 2).await?;
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            Ok(())
        },
    )
    .await
}

pub async fn retry_exhaustion(pool: &PgPool) -> Result {
    const NAME: &str = "flags.live.retry";
    define(pool, NAME, 2).await?;
    let id = enqueue_job(pool, &request(NAME, &Value::Null)).await?;
    let (flags, halt) = flags(SwitchState::Engaged)?;
    let probe = Probe::new(NAME);
    let calls = probe.calls.clone();
    let dead = probe.dead.clone();
    let body_pool = pool.clone();
    drive(pool, flags.gated_direct(probe, &halt, policy(DirectWhenTripped::DeferAttempt(recheck())))?, async move {
        wait_row(&body_pool, id, "DEAD_LETTERED", 1).await?;
        let attempt: i32 = sqlx::query_scalar("SELECT attempt FROM job_queue WHERE id=$1").bind(id).fetch_one(&body_pool).await?;
        assert_eq!(attempt, 2);
        let errors: Vec<String> = sqlx::query_scalar("SELECT error_code FROM job_attempts WHERE job_id=$1 ORDER BY attempt").bind(id).fetch_all(&body_pool).await?;
        assert_eq!(errors, ["flags.job_type_deferred", "flags.job_type_deferred"]);
        let gap: f64 = sqlx::query_scalar("SELECT extract(epoch FROM b.started_at-a.finished_at)::double precision FROM job_attempts a JOIN job_attempts b ON b.job_id=a.job_id AND b.attempt=2 WHERE a.job_id=$1 AND a.attempt=1").bind(id).fetch_one(&body_pool).await?;
        assert!(gap >= 30.0, "retry lower bound");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        Ok(())
    }).await?;
    assert_eq!(dead.load(Ordering::SeqCst), 1);
    Ok(())
}

pub async fn drop_and_workflow_refusal(pool: &PgPool) -> Result {
    const DROP: &str = "flags.live.drop";
    define(pool, DROP, 3).await?;
    let id = enqueue_job(pool, &request(DROP, &Value::Null)).await?;
    let (flags, halt) = flags(SwitchState::Engaged)?;
    let probe = Probe::new(DROP);
    let dead = probe.dead.clone();
    let calls = probe.calls.clone();
    let body_pool = pool.clone();
    drive(
        pool,
        flags.gated_direct(probe, &halt, policy(DirectWhenTripped::Drop))?,
        async move {
            wait_row(&body_pool, id, "DEAD_LETTERED", 1).await?;
            let code: String =
                sqlx::query_scalar("SELECT last_error_code FROM job_queue WHERE id=$1")
                    .bind(id)
                    .fetch_one(&body_pool)
                    .await?;
            assert_eq!(code, "flags.job_type_dropped");
            Ok(())
        },
    )
    .await?;
    assert_eq!(dead.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    const STEP: &str = "flags.live.workflow";
    define(pool, STEP, 3).await?;
    let payload = Value::Null;
    let step = WorkflowStepEnqueueBuilder::new(StepKey::new("work"), JobType::new(STEP), &payload)
        .try_build()?;
    let request =
        WorkflowRunEnqueueBuilder::new(WorkflowType::new("flags.live.workflow"), &payload)
            .step(step)
            .try_build()?;
    let workflow = enqueue_workflow_run(pool, &request).await?;
    let id = list_workflow_steps(pool, None, workflow.id).await?[0]
        .job_id
        .unwrap();
    let body_pool = pool.clone();
    drive(
        pool,
        flags.gated_direct(
            Probe::new(STEP),
            &halt,
            policy(DirectWhenTripped::Defer(recheck())),
        )?,
        async move {
            wait_row(&body_pool, id, "DEAD_LETTERED", 1).await?;
            let code: String =
                sqlx::query_scalar("SELECT last_error_code FROM job_queue WHERE id=$1")
                    .bind(id)
                    .fetch_one(&body_pool)
                    .await?;
            assert_eq!(code, "job.workflow_handler_continuation_not_enabled");
            let status: String =
                sqlx::query_scalar("SELECT status::text FROM workflow_runs WHERE id=$1")
                    .bind(workflow.id)
                    .fetch_one(&body_pool)
                    .await?;
            assert_eq!(status, "COMPLETED_WITH_ERRORS");
            let step: (String, bool) = sqlx::query_as(
                "SELECT status::text, finished_at IS NOT NULL FROM workflow_steps WHERE job_id=$1",
            )
            .bind(id)
            .fetch_one(&body_pool)
            .await?;
            assert_eq!(step, ("FAILED".into(), true));
            Ok(())
        },
    )
    .await
}

fn pinned(value: Toggle, version: u64) -> Result<batter_flags::core::Pinned<Toggle>> {
    let mut b = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
    let worker = b.process_kind("worker")?;
    b.environment_class("hosted")?;
    let mode = b.mode_flag("mode", Toggle::Off).declare()?;
    let test = TestFlags::builder(b.build()?, worker)?
        .set(&mode, value)
        .version(Version::new(version))
        .build()?;
    Ok(test.view().pin(&mode))
}
pub async fn keyed_pin_identity(pool: &PgPool) -> Result {
    const NAME: &str = "flags.live.pin";
    define(pool, NAME, 1).await?;
    let pin = pinned(Toggle::On, 1)?;
    let retained = serde_json::to_value(&pin)?;
    let mut request = request(NAME, &retained);
    request.idempotency_key = Some("flags:retained-pin");
    let inserted = enqueue_job_with_outcome(pool, &request).await?;
    assert_eq!(inserted.disposition, JobEnqueueDisposition::Inserted);
    let replayed = enqueue_job_with_outcome(pool, &request).await?;
    assert_eq!(replayed.disposition, JobEnqueueDisposition::Existing);
    assert_eq!(replayed.job_id, inserted.job_id);
    let same_value = pinned(Toggle::On, 2)?;
    assert!(pin.same_decision(&same_value));
    for changed in [pinned(Toggle::Off, 1)?, same_value] {
        let payload = serde_json::to_value(changed)?;
        let changed_request = JobEnqueue {
            payload: &payload,
            ..request.clone()
        };
        let error = enqueue_job_with_outcome(pool, &changed_request)
            .await
            .unwrap_err();
        let postgres::Error::QueryError(error) = error else {
            panic!("expected classified conflict")
        };
        assert_eq!(error.code(), "job.idempotency_conflict");
    }
    Ok(())
}
