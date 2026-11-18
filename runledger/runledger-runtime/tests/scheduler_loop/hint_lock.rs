use super::*;
use sqlx::postgres::PgPoolOptions;

async fn backend(pool: &PgPool) -> i32 {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(pool)
        .await
        .expect("backend")
}

async fn wait_blocked(pool: &PgPool, waiter: i32, blocker: i32) -> bool {
    timeout(Duration::from_secs(3), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                 WHERE datname = current_database() AND pid = $1
                   AND wait_event_type = 'Lock' AND $2 = ANY(pg_blocking_pids(pid)))",
            )
            .bind(waiter)
            .bind(blocker)
            .fetch_one(pool)
            .await
            .expect("observe identified wait");
            if blocked {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

async fn materialization_gate(pool: &PgPool) -> (sqlx::Transaction<'_, sqlx::Postgres>, i32) {
    const GATE: i64 = 991_383;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION test_park_schedule_commit() RETURNS trigger LANGUAGE plpgsql AS $body$
         BEGIN PERFORM pg_advisory_xact_lock({GATE}); RETURN NEW; END $body$;
         CREATE TRIGGER test_park_schedule_commit AFTER UPDATE OF last_fired_at ON job_schedules
         FOR EACH ROW EXECUTE FUNCTION test_park_schedule_commit();"
    )))
    .execute(pool)
    .await
    .expect("install commit gate");
    let mut gate = pool.begin().await.expect("gate");
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("gate pid");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(GATE)
        .execute(&mut *gate)
        .await
        .expect("hold gate");
    (gate, pid)
}

async fn queue_table_blocker(
    pool: &PgPool,
) -> (i32, JoinHandle<sqlx::Transaction<'static, sqlx::Postgres>>) {
    let mut tx = pool.begin().await.expect("table blocker");
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .expect("blocker pid");
    let task = tokio::spawn(async move {
        sqlx::query("LOCK TABLE job_schedules IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await
            .expect("table lock");
        tx
    });
    (pid, task)
}

async fn session_settings(pool: &PgPool) -> (i32, String, String) {
    sqlx::query_as(
        "SELECT pg_backend_pid(), current_setting('statement_timeout'), current_setting('lock_timeout')",
    )
    .fetch_one(pool)
    .await
    .expect("session settings and reusable connection")
}

#[tokio::test]
async fn scheduler_shutdown_finishes_while_post_commit_hint_is_table_blocked() {
    let (pool, database) = setup_ephemeral_pool("scheduler_hint_lock_shutdown", 5).await;
    register_job_definition(&pool, JobType::new(JOB_TYPE)).await;
    insert_due_schedules(&pool, "hint-lock-", 1, YEARLY_CRON).await;
    let (gate, gate_pid) = materialization_gate(&pool).await;
    let scheduler_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(database.url())
        .await
        .expect("scheduler pool");
    sqlx::raw_sql("SET statement_timeout = '0'; SET lock_timeout = '0'")
        .execute(&scheduler_pool)
        .await
        .expect("unbounded caller settings");
    let scheduler_pid = backend(&scheduler_pool).await;
    let (shutdown, mut scheduler) = start(&scheduler_pool, 8);
    let materializing = wait_blocked(&pool, scheduler_pid, gate_pid).await;
    let (blocker_pid, mut blocker) = queue_table_blocker(&pool).await;
    let queued = wait_blocked(&pool, blocker_pid, scheduler_pid).await;
    gate.rollback().await.expect("allow materialization commit");
    let table_guard = match timeout(Duration::from_secs(3), &mut blocker).await {
        Ok(result) => result.expect("blocker joined"),
        Err(_) => {
            blocker.abort();
            let _ = blocker.await;
            scheduler.abort();
            let _ = scheduler.await;
            panic!("table blocker did not acquire after commit");
        }
    };
    let hint_waiting = wait_blocked(&pool, scheduler_pid, blocker_pid).await;
    shutdown.send(true).expect("shutdown request");
    let stopped = timeout(Duration::from_secs(3), &mut scheduler).await;
    // Probe while the table lock remains held: timeout recovery must return a
    // usable connection with the original settings, not abandon a pending read.
    let settings = if stopped.is_ok() {
        timeout(Duration::from_secs(2), session_settings(&scheduler_pool))
            .await
            .ok()
    } else {
        None
    };
    table_guard.rollback().await.expect("release table lock");
    if stopped.is_err() {
        scheduler.abort();
        let _ = scheduler.await;
    }
    scheduler_pool.close().await;
    assert!(
        materializing && queued && hint_waiting,
        "all lock phases observed"
    );
    assert_eq!(
        stopped
            .expect("shutdown must not require releasing table lock")
            .expect("scheduler joins"),
        RuntimeLoopExit::Shutdown
    );
    assert_eq!(settings, Some((scheduler_pid, "0".into(), "0".into())));
    assert_eq!(scheduled_jobs(&pool).await, 1, "durable pass committed");
    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn earliest_fire_preserves_stricter_timeout_and_restores_pooled_settings() {
    let (pool, database) = setup_ephemeral_pool("scheduler_hint_stricter_timeout", 3).await;
    let reader = PgPoolOptions::new()
        .max_connections(1)
        .connect(database.url())
        .await
        .expect("reader pool");
    sqlx::raw_sql("SET statement_timeout = '50ms'; SET lock_timeout = '0'")
        .execute(&reader)
        .await
        .expect("stricter setting");
    let before = session_settings(&reader).await;
    let mut blocker = pool.begin().await.expect("blocker");
    sqlx::query("LOCK TABLE job_schedules IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .expect("hold table");
    let started = Instant::now();
    let result = runledger_postgres::jobs::earliest_active_schedule_fire_at(&reader).await;
    let elapsed = started.elapsed();
    blocker.rollback().await.expect("unblock");
    assert!(
        elapsed < Duration::from_millis(700),
        "stricter 50ms timeout must not become 1s"
    );
    assert!(
        matches!(result, Err(runledger_postgres::Error::QueryError(ref error)) if error.sqlstate() == Some("57014"))
    );
    assert_eq!(session_settings(&reader).await, before);
    assert_eq!(
        runledger_postgres::jobs::earliest_active_schedule_fire_at(&reader)
            .await
            .expect("successful hint"),
        None
    );
    assert_eq!(session_settings(&reader).await, before);
    reader.close().await;
    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn failed_hint_waits_the_poll_interval_before_another_materialization() {
    const POLL: Duration = Duration::from_secs(37);
    let (pool, database) = setup_ephemeral_pool("scheduler_hint_error_cadence", 5).await;
    register_job_definition(&pool, JobType::new(JOB_TYPE)).await;
    insert_due_schedules(&pool, "first-pass-", 1, YEARLY_CRON).await;
    let (gate, gate_pid) = materialization_gate(&pool).await;
    let scheduler_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(database.url())
        .await
        .expect("scheduler pool");
    let scheduler_pid = backend(&scheduler_pool).await;
    let mut configured = config(8);
    configured.schedule_poll_interval = POLL;
    let (shutdown, receiver) = watch::channel(false);
    let scheduler = tokio::spawn(run_scheduler_loop(
        scheduler_pool.clone(),
        configured,
        receiver,
    ));
    assert!(wait_blocked(&pool, scheduler_pid, gate_pid).await);
    let (blocker_pid, blocker) = queue_table_blocker(&pool).await;
    assert!(wait_blocked(&pool, blocker_pid, scheduler_pid).await);
    gate.rollback().await.expect("commit partial pass");
    let table_guard = timeout(Duration::from_secs(3), blocker)
        .await
        .expect("table lock acquired")
        .expect("blocker joins");
    assert!(wait_blocked(&pool, scheduler_pid, blocker_pid).await);
    let hint_waiting_at = Instant::now();

    // Acquiring this single pooled connection while the table remains locked
    // proves the hint failed and completed rollback. Retain it until new work
    // is claimable, so even an incorrectly early next pass cannot miss that work.
    let connection = timeout(Duration::from_secs(3), scheduler_pool.acquire())
        .await
        .expect("hint returns its connection despite table lock")
        .expect("connection reusable");
    let hint_returned_at = Instant::now();
    table_guard
        .rollback()
        .await
        .expect("remove the hint failure");
    insert_due_schedules(&pool, "next-pass-", 1, YEARLY_CRON).await;
    assert_eq!(scheduled_jobs(&pool).await, 1, "partial pass committed");
    // The sleep starts between observing the blocked read and acquiring its
    // returned connection. Cross most of the configured interval, retaining
    // room for the negative probe below. Keep real time running during SQL I/O.
    advance_clock_to(hint_waiting_at + POLL - Duration::from_secs(5)).await;
    drop(connection);
    sleep(QUIET_WINDOW).await;
    assert_eq!(
        scheduled_jobs(&pool).await,
        1,
        "failed hint must wait the full poll interval"
    );
    // Cross the latest possible expiry and require the SAME owner to resume;
    // a hard-coded delay, stuck loop or lingering SQL failure cannot pass.
    advance_clock_to(hint_returned_at + POLL + Duration::from_millis(1)).await;
    wait_until(
        "new schedule materializes",
        Duration::from_secs(5),
        || async { scheduled_jobs(&pool).await == 2 },
    )
    .await;
    stop(shutdown, scheduler).await;
    scheduler_pool.close().await;
    teardown_ephemeral_pool(pool, database).await;
}

async fn advance_clock_to(deadline: Instant) {
    tokio::time::pause();
    tokio::time::advance(deadline.saturating_duration_since(Instant::now())).await;
    tokio::time::resume();
}
