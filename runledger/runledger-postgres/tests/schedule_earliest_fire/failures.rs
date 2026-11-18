use runledger_postgres::{Error, jobs::earliest_active_schedule_fire_at};
use runledger_test_support::{setup_ephemeral_pool, teardown_ephemeral_pool};
use sqlx::postgres::PgPoolOptions;
use std::error::Error as _;
use tokio::time::{Duration, sleep, timeout};

#[tokio::test]
async fn terminated_earliest_fire_read_retains_read_and_rollback_errors() {
    let (pool, database) = setup_ephemeral_pool("earliest_fire_dual_failure", 3).await;
    let reader = PgPoolOptions::new()
        .max_connections(1)
        .connect(database.url())
        .await
        .expect("reader pool");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&reader)
        .await
        .expect("reader backend");
    let mut blocker = pool.begin().await.expect("blocker");
    sqlx::query("LOCK TABLE job_schedules IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .expect("block read");
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .expect("blocker backend");
    let read_pool = reader.clone();
    let read = tokio::spawn(async move { earliest_active_schedule_fire_at(&read_pool).await });
    timeout(Duration::from_secs(3), async {
        loop {
            // Identify the lock wait and terminate that same backend in one
            // statement, before the helper's one-second statement cap expires.
            let terminated: Option<bool> = sqlx::query_scalar(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
                 WHERE pid = $1 AND wait_event_type = 'Lock'
                   AND $2 = ANY(pg_blocking_pids(pid))",
            )
            .bind(pid)
            .bind(blocker_pid)
            .fetch_optional(&pool)
            .await
            .expect("terminate the blocked hint backend");
            if terminated == Some(true) {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("hint backend reaches its lock wait");
    let result = timeout(Duration::from_secs(3), read)
        .await
        .expect("read settles")
        .expect("join");
    blocker.rollback().await.expect("release table");
    let Error::RollbackFailure(pair) = result.expect_err("read and rollback fail") else {
        panic!("both failures must be retained");
    };
    let source = pair
        .operation
        .source()
        .expect("query source")
        .source()
        .expect("SQLx source")
        .downcast_ref::<sqlx::Error>()
        .expect("concrete SQLx error");
    assert!(matches!(
        source,
        sqlx::Error::Database(_) | sqlx::Error::Io(_)
    ));
    assert!(matches!(
        pair.rollback,
        sqlx::Error::Database(_) | sqlx::Error::Io(_) | sqlx::Error::Protocol(_)
    ));
    // Recovery uses another backend, not a claim that rollback was acknowledged.
    assert_eq!(
        earliest_active_schedule_fire_at(&reader)
            .await
            .expect("replacement connection"),
        None
    );
    reader.close().await;
    teardown_ephemeral_pool(pool, database).await;
}
