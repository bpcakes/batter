use super::*;
use runledger_postgres::jobs::{JobEnqueue, enqueue_job, reap_expired_leases_with_diagnostics};
use sqlx::postgres::PgPoolOptions;

async fn pair(pool: &DbPool, kind: JobType<'static>, max_attempts: i32) -> (Uuid, [Uuid; 2]) {
    let payload = json!({});
    let metadata = json!({});
    let mut builder = WorkflowRunEnqueueBuilder::new(
        WorkflowType::new("workflow.test.reaper_preparation"),
        &metadata,
    );
    for key in ["first", "second"] {
        builder = builder.step(
            WorkflowStepEnqueueBuilder::new(StepKey::new(key), kind, &payload)
                .max_attempts(max_attempts)
                .try_build()
                .expect("step"),
        );
    }
    let run = enqueue_workflow_run(pool, &builder.try_build().expect("workflow"))
        .await
        .expect("enqueue workflow");
    let steps = list_workflow_steps(pool, None, run.id)
        .await
        .expect("steps");
    let ids = ["first", "second"].map(|key| {
        steps
            .iter()
            .find(|s| s.step_key.as_str() == key)
            .and_then(|s| s.job_id)
            .expect("job id")
    });
    (run.id, ids)
}

async fn expire(pool: &DbPool, ids: &[Uuid]) {
    for (index, id) in ids.iter().enumerate() {
        sqlx::query(
            "UPDATE job_queue SET lease_expires_at = now() -
            ($2::bigint * interval '1 second') WHERE id = $1",
        )
        .bind(id)
        .bind(100 - index as i64)
        .execute(pool)
        .await
        .expect("expire in order");
    }
}

async fn direct(pool: &DbPool, job_type: JobType<'static>) -> Uuid {
    enqueue_job(
        pool,
        &JobEnqueue {
            job_type,
            organization_id: None,
            payload: &json!({}),
            priority: None,
            max_attempts: None,
            timeout_seconds: None,
            next_run_at: None,
            idempotency_key: None,
            stage: None,
        },
    )
    .await
    .expect("direct job")
}

#[tokio::test]
async fn busy_run_defers_only_its_rows_and_preserves_other_work() {
    let (pool, database) = setup_ephemeral_pool("workflow_reaper_busy_run", 4).await;
    let kind = JobType::new("jobs.test.reaper_preparation");
    register_job_definition(&pool, kind).await;
    let (busy, busy_jobs) = pair(&pool, kind, 3).await;
    let (_, healthy_jobs) = pair(&pool, kind, 3).await;
    let direct = direct(&pool, kind).await;
    assert_eq!(
        claim_jobs_for_types(&pool, "prep-worker", 30, 5, &[kind])
            .await
            .expect("claim")
            .len(),
        5
    );
    expire(
        &pool,
        &[
            busy_jobs[0],
            busy_jobs[1],
            healthy_jobs[0],
            healthy_jobs[1],
            direct,
        ],
    )
    .await;
    let mut gate = pool.begin().await.expect("run gate");
    sqlx::query("SELECT id FROM workflow_runs WHERE id = $1 FOR UPDATE")
        .bind(busy)
        .execute(&mut *gate)
        .await
        .expect("hold run row");
    let result = reap_expired_leases_with_diagnostics(&pool, 5, 60_000)
        .await
        .expect("reap");
    gate.rollback().await.expect("release gate");
    assert_eq!(result.summary.processed, 3);
    assert_eq!(result.reaped_leases.len(), 3);
    assert_eq!(result.deferred_row_error_count, 2);
    for error in &result.deferred_row_errors {
        assert!(busy_jobs.contains(&error.job_id));
        assert_eq!(error.sqlstate.as_deref(), Some("55P03"));
    }
    let untouched: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_queue WHERE id = ANY($1) AND status = 'LEASED'
           AND attempt = 1 AND lease_expires_at > now()",
    )
    .bind(&busy_jobs[..])
    .fetch_one(&pool)
    .await
    .expect("deferred identity");
    assert_eq!(untouched, 2);
    let failed_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_events WHERE job_id = ANY($1) AND event_type = 'FAILED'",
    )
    .bind(&busy_jobs[..])
    .fetch_one(&pool)
    .await
    .expect("no partial events");
    assert_eq!(failed_events, 0);
    expire(&pool, &busy_jobs).await;
    assert_eq!(
        reap_expired_leases_with_diagnostics(&pool, 5, 60_000)
            .await
            .expect("progress after contention")
            .summary
            .processed,
        2
    );
    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn first_row_rollback_keeps_the_prepared_run_lock_for_later_rows() {
    let (pool, database) = setup_ephemeral_pool("workflow_reaper_row_rollback", 5).await;
    let kind = JobType::new("jobs.test.reaper_rollback");
    register_job_definition(&pool, kind).await;
    let (run, [first, second]) = pair(&pool, kind, 3).await;
    claim_jobs_for_types(&pool, "rollback-worker", 30, 2, &[kind])
        .await
        .expect("claim");
    expire(&pool, &[first, second]).await;
    const GATE: i64 = 991_382;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION test_reaper_poison() RETURNS trigger LANGUAGE plpgsql AS $body$
         BEGIN IF NEW.job_id = '{first}'::uuid AND NEW.status = 'ENQUEUED' THEN
           RAISE EXCEPTION 'fixture first-row failure'; END IF; RETURN NEW; END $body$;
         CREATE TRIGGER test_reaper_poison AFTER UPDATE ON workflow_steps
         FOR EACH ROW EXECUTE FUNCTION test_reaper_poison();
         CREATE FUNCTION test_reaper_second_gate() RETURNS trigger LANGUAGE plpgsql AS $body$
         BEGIN IF NEW.id = '{second}'::uuid AND NEW.status = 'PENDING' THEN
           PERFORM pg_advisory_xact_lock({GATE}); END IF; RETURN NEW; END $body$;
         CREATE TRIGGER test_reaper_second_gate AFTER UPDATE ON job_queue
         FOR EACH ROW EXECUTE FUNCTION test_reaper_second_gate();"
    )))
    .execute(&pool)
    .await
    .expect("install rollback probe");
    let mut gate = pool.begin().await.expect("gate");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("pid");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(GATE)
        .execute(&mut *gate)
        .await
        .expect("hold gate");
    let worker = PgPoolOptions::new()
        .max_connections(1)
        .connect(database.url())
        .await
        .expect("worker");
    let worker_pid = reaper::backend(&worker).await;
    let task = tokio::spawn({
        let worker = worker.clone();
        async move { reap_expired_leases_with_diagnostics(&worker, 2, 60_000).await }
    });
    let parked = reaper::wait_blocked(&pool, worker_pid, gate_pid).await;
    let mut probe = pool.begin().await.expect("probe");
    let lock = sqlx::query("SELECT id FROM workflow_runs WHERE id = $1 FOR UPDATE NOWAIT")
        .bind(run)
        .execute(&mut *probe)
        .await;
    probe.rollback().await.expect("release probe");
    gate.rollback().await.expect("release gate");
    let result = reaper::joined(task).await;
    worker.close().await;
    let result = result.expect("worker joins").expect("batch");
    assert!(parked, "second row reached gate after first row rollback");
    assert_eq!(
        lock.expect_err("run lock must survive row rollback")
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref(),
        Some("55P03")
    );
    assert_eq!(result.summary.processed, 1);
    assert_eq!(result.deferred_row_error_count, 1);
    assert_eq!(result.deferred_row_errors[0].job_id, first);
    assert_eq!(result.reaped_leases[0].job_id, second);
    assert_eq!(
        step_status(&pool, run, "first").await,
        WorkflowStepStatus::Running
    );
    assert_eq!(
        step_status(&pool, run, "second").await,
        WorkflowStepStatus::Enqueued
    );
    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn competing_reapers_prepare_runs_independently_of_expiry_order() {
    let (pool, database) = setup_ephemeral_pool("workflow_reaper_two_runs", 5).await;
    let kind = JobType::new("jobs.test.reaper_two_runs");
    register_job_definition(&pool, kind).await;
    let (a, aj) = pair(&pool, kind, 1).await;
    let (b, bj) = pair(&pool, kind, 1).await;
    claim_jobs_for_types(&pool, "two-run-worker", 30, 4, &[kind])
        .await
        .expect("claim");
    expire(&pool, &[aj[0], bj[0], bj[1], aj[1]]).await;
    let mut gate = pool.begin().await.expect("gate");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("gate pid");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(workflow_run_release_lock_key(a.min(b)))
        .execute(&mut *gate)
        .await
        .expect("run gate");
    let one = PgPoolOptions::new()
        .max_connections(1)
        .connect(database.url())
        .await
        .expect("one");
    let two = PgPoolOptions::new()
        .max_connections(1)
        .connect(database.url())
        .await
        .expect("two");
    let first_pid = reaper::backend(&one).await;
    let second_pid = reaper::backend(&two).await;
    let first = tokio::spawn({
        let pool = one.clone();
        async move { reap_expired_leases_with_diagnostics(&pool, 2, 60_000).await }
    });
    let first_parked = reaper::wait_blocked(&pool, first_pid, gate_pid).await;
    let second = tokio::spawn({
        let pool = two.clone();
        async move { reap_expired_leases_with_diagnostics(&pool, 2, 60_000).await }
    });
    let second_parked = reaper::wait_blocked(&pool, second_pid, gate_pid).await;
    gate.rollback().await.expect("release runs");
    let first = reaper::joined(first).await;
    let second = reaper::joined(second).await;
    one.close().await;
    two.close().await;
    let first = first.expect("first joins").expect("first batch");
    let second = second.expect("second joins").expect("second batch");
    assert!(
        first_parked && second_parked,
        "both preparations reached gate"
    );
    let processed = first.summary.processed + second.summary.processed;
    assert!(processed > 0);
    for result in [&first, &second] {
        assert_eq!(
            result.summary.processed + result.deferred_row_error_count as i64,
            2
        );
        for error in &result.deferred_row_errors {
            assert_eq!(error.sqlstate.as_deref(), Some("55P03"));
        }
    }
    expire(&pool, &[aj[0], aj[1], bj[0], bj[1]]).await;
    let final_pass = reap_expired_leases_with_diagnostics(&pool, 4, 60_000)
        .await
        .expect("finish");
    assert_eq!(processed + final_pass.summary.processed, 4);
    assert_eq!(final_pass.deferred_row_error_count, 0);
    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn direct_only_reaping_needs_no_workflow_run_grants() {
    use batter_sqlx::verification::{
        DiscoveryScope, ExactRoleManifest, FragmentObjectPolicy, Identifier,
    };
    use runledger_postgres::grants::{RunledgerOperation, grant_fragment};
    let (pool, database) = setup_ephemeral_pool("reaper_direct_grants", 1).await;
    let kind = JobType::new("jobs.test.reaper_direct_grants");
    register_job_definition(&pool, kind).await;
    let job = direct(&pool, kind).await;
    claim_jobs_for_types(&pool, "restricted-worker", 30, 1, &[kind])
        .await
        .expect("claim");
    expire(&pool, &[job]).await;
    let role_name = format!("reaper_{}", Uuid::new_v4().simple());
    let role = Identifier::new(&role_name).expect("role");
    let policy = FragmentObjectPolicy::new(Identifier::new("public").expect("schema"));
    let grants = ExactRoleManifest::new(policy.schema().clone(), DiscoveryScope::Declared)
        .expect("manifest")
        .with_fragment(
            grant_fragment(&policy, [RunledgerOperation::DirectJobExecution]).expect("fragment"),
        )
        .expect("compose")
        .compile()
        .expect("compile")
        .grant_plan()
        .render(&role, None)
        .expect("grants");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {role_name} NOLOGIN; {grants}; SET ROLE {role_name}"
    )))
    .execute(&pool)
    .await
    .expect("restricted role");
    let result = reap_expired_leases_with_diagnostics(&pool, 2, 60_000).await;
    let forbidden = sqlx::query("SELECT id FROM workflow_runs")
        .fetch_all(&pool)
        .await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "RESET ROLE; DROP OWNED BY {role_name}; DROP ROLE {role_name}"
    )))
    .execute(&pool)
    .await
    .expect("remove role");
    let result = result.expect("direct grants cover preparation and reaping");
    assert_eq!(result.summary.processed, 1);
    assert_eq!(result.deferred_row_error_count, 0);
    assert!(result.cleanup_errors.is_empty());
    assert_eq!(
        forbidden
            .expect_err("role cannot read runs")
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref(),
        Some("42501")
    );
    teardown_ephemeral_pool(pool, database).await;
}
