//! Hold the fourth one-row pass until a test has requested shutdown.
use sqlx::{PgPool, Postgres, Transaction};
use tokio::time::{Duration, sleep, timeout};

pub async fn install(pool: &PgPool, trigger: &str) -> (Transaction<'static, Postgres>, i32) {
    sqlx::raw_sql(
        "CREATE SEQUENCE test_pass_count;
         CREATE FUNCTION test_gate_pass() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
             IF nextval('test_pass_count') = 4 THEN
                 PERFORM pg_advisory_xact_lock(871239);
             END IF;
             RETURN NEW;
         END $$;",
    )
    .execute(pool)
    .await
    .expect("install fourth-pass gate");
    sqlx::raw_sql(sqlx::AssertSqlSafe(trigger.to_owned()))
        .execute(pool)
        .await
        .expect("install pass trigger");
    let mut gate = pool.begin().await.expect("gate transaction");
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("gate backend");
    sqlx::query("SELECT pg_advisory_xact_lock(871239)")
        .execute(&mut *gate)
        .await
        .expect("hold pass gate");
    (gate, pid)
}

pub async fn wait(pool: &PgPool, blocker: i32) {
    timeout(Duration::from_secs(10), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                 WHERE datname = current_database() AND wait_event = 'advisory'
                   AND $1 = ANY(pg_blocking_pids(pid)))",
            )
            .bind(blocker)
            .fetch_one(pool)
            .await
            .expect("observe the fourth pass at its gate");
            if blocked {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fourth pass reaches its gate");
}
