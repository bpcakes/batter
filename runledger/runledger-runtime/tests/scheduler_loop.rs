//! Scheduler cadence: full batches drain at once, failures and idle passes wait.
use std::future::Future;
use std::time::Duration;

use runledger_core::jobs::JobType;
use runledger_runtime::RuntimeLoopExit;
use runledger_runtime::config::JobsConfig;
use runledger_runtime::scheduler::run_scheduler_loop;
use runledger_test_support::{
    setup_ephemeral_pool_with_untracked_migrations as setup_ephemeral_pool, teardown_ephemeral_pool,
};
use sqlx::PgPool;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};

use job_definition::register_job_definition;

#[path = "support/job_definition.rs"]
mod job_definition;

#[path = "support/pass_gate.rs"]
mod pass_gate;

#[path = "scheduler_loop/hint_lock.rs"]
mod hint_lock;

const JOB_TYPE: &str = "jobs.test.scheduler.cadence";
/// Fires once a year, so a materialized schedule is not due again during a test.
const YEARLY_CRON: &str = "0 0 0 1 1 * *";
const LONG_POLL: Duration = Duration::from_secs(3_600);
/// Longer than two idle-wait floors, shorter than any deferral or poll interval.
const QUIET_WINDOW: Duration = Duration::from_millis(1_500);

fn config(claim_batch_size: i64) -> JobsConfig {
    JobsConfig {
        worker_id: "scheduler-cadence-test-worker".to_string(),
        poll_interval: Duration::from_secs(30),
        claim_batch_size,
        lease_ttl_seconds: 30,
        max_global_concurrency: 1,
        reaper_interval: Duration::from_secs(30),
        schedule_poll_interval: LONG_POLL,
        reaper_retry_delay_ms: 1_000,
    }
}

async fn insert_due_schedules(pool: &PgPool, prefix: &str, count: i64, cron_expr: &str) {
    sqlx::query(
        "INSERT INTO job_schedules (name, job_type, cron_expr, next_fire_at)
         SELECT $1 || n, $2, $3, now() - interval '1 minute'
         FROM generate_series(1, $4) n",
    )
    .bind(prefix)
    .bind(JOB_TYPE)
    .bind(cron_expr)
    .bind(count)
    .execute(pool)
    .await
    .expect("insert due schedules");
}

/// Counts updates of `job_schedules` rows, including rolled-back ones, because
/// sequence increments are not transactional. `poisoned` makes each update fail.
async fn count_schedule_updates(pool: &PgPool, poisoned: bool) {
    sqlx::query("CREATE SEQUENCE test_schedule_updates")
        .execute(pool)
        .await
        .expect("create update counter");
    let body = if poisoned {
        "BEGIN PERFORM nextval('test_schedule_updates'); \
         RAISE EXCEPTION 'poisoned schedule update'; END"
    } else {
        "BEGIN PERFORM nextval('test_schedule_updates'); RETURN NEW; END"
    };
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION test_count_schedule_update() RETURNS trigger
         LANGUAGE plpgsql AS $body$ {body} $body$"
    )))
    .execute(pool)
    .await
    .expect("create update counter function");
    sqlx::query(
        "CREATE TRIGGER test_count_schedule_update BEFORE UPDATE ON job_schedules
         FOR EACH ROW EXECUTE FUNCTION test_count_schedule_update()",
    )
    .execute(pool)
    .await
    .expect("create update counter trigger");
}

async fn schedule_updates(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM test_schedule_updates",
    )
    .fetch_one(pool)
    .await
    .expect("read update counter")
}

async fn scheduled_jobs(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*)::bigint FROM job_queue WHERE job_type = $1")
        .bind(JOB_TYPE)
        .fetch_one(pool)
        .await
        .expect("count scheduled jobs")
}

async fn wait_until<F, Fut>(what: &str, within: Duration, mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + within;
    while !condition().await {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        sleep(Duration::from_millis(10)).await;
    }
}

fn start(
    pool: &PgPool,
    claim_batch_size: i64,
) -> (watch::Sender<bool>, JoinHandle<RuntimeLoopExit>) {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(run_scheduler_loop(
        pool.clone(),
        config(claim_batch_size),
        shutdown_rx,
    ));
    (shutdown_tx, task)
}

async fn stop(shutdown_tx: watch::Sender<bool>, task: JoinHandle<RuntimeLoopExit>) {
    shutdown_tx.send(true).expect("scheduler receives shutdown");
    let exit = timeout(Duration::from_secs(5), task)
        .await
        .expect("scheduler returns within one pass of shutdown")
        .expect("scheduler task joins");
    assert_eq!(exit, RuntimeLoopExit::Shutdown);
}

#[tokio::test]
async fn scheduler_loop_drains_full_batches_without_waiting_for_the_poll_interval() {
    const BATCH: i64 = 2;
    const TOTAL: i64 = 3 * BATCH + 1;
    let (pool, database) = setup_ephemeral_pool("runtime_scheduler_backlog_drain", 4).await;
    register_job_definition(&pool, JobType::new(JOB_TYPE)).await;
    insert_due_schedules(&pool, "drain-", TOTAL, YEARLY_CRON).await;

    let (shutdown_tx, task) = start(&pool, BATCH);
    wait_until(
        "every due schedule materialized inside one hour-long poll interval",
        Duration::from_secs(10),
        || async { scheduled_jobs(&pool).await >= TOTAL },
    )
    .await;
    sleep(QUIET_WINDOW).await;
    stop(shutdown_tx, task).await;

    let jobs_per_schedule: Vec<i64> = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM job_queue WHERE job_type = $1
         GROUP BY payload #>> '{_schedule,schedule_id}'",
    )
    .bind(JOB_TYPE)
    .fetch_all(&pool)
    .await
    .expect("count jobs per schedule");
    assert_eq!(jobs_per_schedule.len(), TOTAL as usize);
    assert!(
        jobs_per_schedule.iter().all(|jobs| *jobs == 1),
        "draining must materialize each due schedule exactly once: {jobs_per_schedule:?}"
    );

    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn scheduler_loop_wakes_for_a_fire_after_a_partial_pass() {
    let (pool, database) = setup_ephemeral_pool("runtime_scheduler_earliest_wake", 4).await;
    register_job_definition(&pool, JobType::new(JOB_TYPE)).await;
    insert_due_schedules(&pool, "wake-sentinel-", 1, YEARLY_CRON).await;
    sqlx::query(
        "INSERT INTO job_schedules (name, job_type, cron_expr, next_fire_at)
         VALUES ('wake-target', $1, $2, now() + interval '1 year')",
    )
    .bind(JOB_TYPE)
    .bind(YEARLY_CRON)
    .execute(&pool)
    .await
    .expect("distant target");
    // The first SELECT has already fixed its claim set before this trigger can
    // move the target. Slow runtime startup therefore cannot turn this into a
    // test that succeeds by materializing both schedules in the initial pass.
    sqlx::raw_sql(
        "CREATE TABLE test_wake_expected (fire_at timestamptz NOT NULL);
         CREATE FUNCTION test_move_target_after_claim() RETURNS trigger LANGUAGE plpgsql AS $body$
         DECLARE due timestamptz;
         BEGIN
           UPDATE job_schedules SET next_fire_at = clock_timestamp() + interval '1 second'
             WHERE name = 'wake-target' RETURNING next_fire_at INTO due;
           INSERT INTO test_wake_expected VALUES (due);
           RETURN NEW;
         END $body$;
         CREATE TRIGGER test_move_target_after_claim AFTER UPDATE OF last_fired_at ON job_schedules
         FOR EACH ROW WHEN (NEW.name = 'wake-sentinel-1' AND OLD.last_fired_at IS NULL)
         EXECUTE FUNCTION test_move_target_after_claim();",
    )
    .execute(&pool)
    .await
    .expect("install one-shot fixture trigger");

    // One initial due schedule is a partial batch, so immediate full-batch
    // repoll cannot substitute for the earliest-fire wait under test.
    let (shutdown_tx, task) = start(&pool, 8);
    let observed = timeout(Duration::from_secs(12), async {
        while scheduled_jobs(&pool).await < 2 {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    stop(shutdown_tx, task).await;
    assert!(
        observed.is_ok(),
        "target must fire before the hour-long poll interval"
    );
    let target_jobs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_queue jq CROSS JOIN test_wake_expected expected
         JOIN job_schedules schedule ON schedule.name = 'wake-target'
         WHERE jq.payload #>> '{_schedule,schedule_id}' = schedule.id::text
           AND (jq.payload #>> '{_schedule,scheduled_for}')::timestamptz = expected.fire_at
           AND schedule.next_fire_at > expected.fire_at
           AND schedule.last_fired_at >= expected.fire_at",
    )
    .fetch_one(&pool)
    .await
    .expect("target metadata and cursor");
    assert_eq!(target_jobs, 1);
    assert_eq!(scheduled_jobs(&pool).await, 2);
    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn scheduler_loop_does_not_repoll_a_full_batch_of_deferred_failures() {
    const BATCH: i64 = 3;
    let (pool, database) = setup_ephemeral_pool("runtime_scheduler_deferred_batch", 4).await;
    register_job_definition(&pool, JobType::new(JOB_TYPE)).await;
    insert_due_schedules(&pool, "invalid-cron-", BATCH, "not a cron expression").await;
    count_schedule_updates(&pool, false).await;

    let (shutdown_tx, task) = start(&pool, BATCH);
    wait_until(
        "each failing schedule is deferred",
        Duration::from_secs(5),
        || async { schedule_updates(&pool).await >= BATCH },
    )
    .await;
    sleep(QUIET_WINDOW).await;

    assert_eq!(
        schedule_updates(&pool).await,
        BATCH,
        "a full batch of failures is deferred once and then waits; it must not re-poll"
    );
    let still_due: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM job_schedules WHERE next_fire_at <= now()",
    )
    .fetch_one(&pool)
    .await
    .expect("count due schedules");
    assert_eq!(still_due, 0, "failed schedules leave the due window");
    stop(shutdown_tx, task).await;

    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn scheduler_loop_waits_the_poll_interval_after_a_failed_pass() {
    const BATCH: i64 = 3;
    let (pool, database) = setup_ephemeral_pool("runtime_scheduler_failed_pass", 4).await;
    register_job_definition(&pool, JobType::new(JOB_TYPE)).await;
    insert_due_schedules(&pool, "poisoned-", BATCH, YEARLY_CRON).await;
    // Marking the fire fails, then deferring fails, so the pass rolls back and
    // leaves every claimed schedule due.
    count_schedule_updates(&pool, true).await;

    let (shutdown_tx, task) = start(&pool, BATCH);
    wait_until("the first pass fails", Duration::from_secs(5), || async {
        schedule_updates(&pool).await >= 2
    })
    .await;
    sleep(QUIET_WINDOW).await;

    assert_eq!(
        schedule_updates(&pool).await,
        2,
        "a failed pass claims a full batch but commits nothing; it must wait the poll interval"
    );
    assert_eq!(scheduled_jobs(&pool).await, 0);
    stop(shutdown_tx, task).await;

    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn scheduler_loop_observes_shutdown_between_back_to_back_full_batches() {
    const BATCH: i64 = 1;
    const TOTAL: i64 = 8;
    let (pool, database) = setup_ephemeral_pool("runtime_scheduler_drain_shutdown", 4).await;
    register_job_definition(&pool, JobType::new(JOB_TYPE)).await;
    insert_due_schedules(&pool, "backlog-", TOTAL, YEARLY_CRON).await;
    let (gate, gate_pid) = pass_gate::install(
        &pool,
        "CREATE TRIGGER test_park_schedule_pass AFTER UPDATE OF last_fired_at ON job_schedules
         FOR EACH ROW EXECUTE FUNCTION test_gate_pass()",
    )
    .await;

    let (shutdown_tx, task) = start(&pool, BATCH);
    pass_gate::wait(&pool, gate_pid).await;
    assert_eq!(scheduled_jobs(&pool).await, 3);
    shutdown_tx
        .send(true)
        .expect("request stop while pass four is held");
    gate.rollback().await.expect("release pass four");
    assert_eq!(
        timeout(Duration::from_secs(5), task)
            .await
            .expect("scheduler stops")
            .expect("join"),
        RuntimeLoopExit::Shutdown
    );
    let after_shutdown = scheduled_jobs(&pool).await;
    assert_eq!(
        after_shutdown, 4,
        "only the held pass may finish after stop"
    );

    teardown_ephemeral_pool(pool, database).await;
}
