//! The scheduler's earliest-active-fire hint and the index serving it.
use batter_sqlx::verification::{
    DiscoveryScope, ExactRoleManifest, FragmentObjectPolicy, Identifier,
};
use chrono::{DateTime, Duration, Utc};
use runledger_postgres::DbPool;
use runledger_postgres::grants::{RunledgerOperation, grant_fragment};
use runledger_postgres::jobs::earliest_active_schedule_fire_at;
use runledger_test_support::{setup_ephemeral_pool, teardown_ephemeral_pool};
use serde_json::Value;

mod support;

#[path = "schedule_earliest_fire/failures.rs"]
mod failures;

const JOB_TYPE: &str = "jobs.test.schedule_earliest_fire";

fn fixed_utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("fixed timestamp")
        .with_timezone(&Utc)
}

async fn insert_schedule(pool: &DbPool, name: &str, next_fire_at: DateTime<Utc>, is_active: bool) {
    sqlx::query(
        "INSERT INTO job_schedules (name, job_type, cron_expr, is_active, next_fire_at)
         VALUES ($1, $2, '0 0 * * * * *', $3, $4)",
    )
    .bind(name)
    .bind(JOB_TYPE)
    .bind(is_active)
    .bind(next_fire_at)
    .execute(pool)
    .await
    .expect("insert schedule");
}

async fn earliest(pool: &DbPool) -> Option<DateTime<Utc>> {
    earliest_active_schedule_fire_at(pool)
        .await
        .expect("read earliest active schedule fire")
}

#[tokio::test]
async fn earliest_active_schedule_fire_ignores_inactive_schedules() {
    let (pool, database) = setup_ephemeral_pool("postgres_schedule_earliest_fire", 2).await;
    support::register_test_job_definition(&pool, JOB_TYPE).await;
    let base = fixed_utc("2026-05-26T12:00:00Z");

    assert_eq!(earliest(&pool).await, None, "no schedule exists");

    insert_schedule(&pool, "earliest-paused", base - Duration::hours(1), false).await;
    assert_eq!(
        earliest(&pool).await,
        None,
        "an inactive schedule is never claimed, so it must not shorten the wait"
    );

    insert_schedule(&pool, "earliest-later", base + Duration::minutes(10), true).await;
    insert_schedule(&pool, "earliest-sooner", base + Duration::minutes(5), true).await;
    assert_eq!(
        earliest(&pool).await,
        Some(base + Duration::minutes(5)),
        "the earliest active cursor wins over later active and earlier inactive rows"
    );

    teardown_ephemeral_pool(pool, database).await;
}

fn scheduled_dispatch_grants(role: &Identifier) -> String {
    let policy = FragmentObjectPolicy::new(Identifier::new("public").expect("schema identifier"));
    ExactRoleManifest::new(policy.schema().clone(), DiscoveryScope::Declared)
        .expect("valid manifest")
        .with_fragment(
            grant_fragment(&policy, [RunledgerOperation::ScheduledDispatch])
                .expect("valid fragment"),
        )
        .expect("composable fragment")
        .compile()
        .expect("compilable manifest")
        .grant_plan()
        .render(role, None)
        .expect("renderable plan")
}

#[tokio::test]
async fn earliest_active_schedule_fire_runs_under_the_scheduled_dispatch_grants() {
    // The scheduler falls back to its poll interval when this read fails, so a
    // missing privilege would only show as a warning; prove the grant instead.
    let (pool, database) = setup_ephemeral_pool("postgres_schedule_earliest_grants", 1).await;
    support::register_test_job_definition(&pool, JOB_TYPE).await;
    let base = fixed_utc("2026-05-26T12:00:00Z");
    insert_schedule(&pool, "earliest-granted", base, true).await;
    let suffix = Utc::now().timestamp_nanos_opt().expect("nanosecond clock");
    let role_name = format!("earliest_fire_dispatch_{}_{suffix}", std::process::id());
    let role = Identifier::new(&role_name).expect("role identifier");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {role_name} NOLOGIN; {}",
        scheduled_dispatch_grants(&role)
    )))
    .execute(&pool)
    .await
    .expect("grant only the scheduled-dispatch fragment");

    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("SET ROLE {role_name}")))
        .execute(&pool)
        .await
        .expect("assume the dispatch role");
    let granted = earliest_active_schedule_fire_at(&pool).await;
    let ungranted_column = sqlx::query("SELECT created_at FROM job_schedules")
        .fetch_all(&pool)
        .await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "RESET ROLE; DROP OWNED BY {role_name}; DROP ROLE {role_name}"
    )))
    .execute(&pool)
    .await
    .expect("drop the dispatch role");

    assert_eq!(granted.expect("dispatch grants cover the read"), Some(base));
    assert!(
        ungranted_column.is_err(),
        "the role must be column-restricted for this check to mean anything"
    );

    teardown_ephemeral_pool(pool, database).await;
}

fn uses_index(value: &Value, index: &str) -> bool {
    match value {
        Value::Object(object) => {
            object
                .get("Index Name")
                .and_then(Value::as_str)
                .is_some_and(|name| name == index)
                || object.values().any(|value| uses_index(value, index))
        }
        Value::Array(values) => values.iter().any(|value| uses_index(value, index)),
        _ => false,
    }
}

#[tokio::test]
async fn earliest_active_schedule_fire_reads_the_partial_next_fire_index() {
    // One connection keeps the public call and EXPLAIN on the same backend, so
    // this explains the executed statement rather than a copied query.
    let (pool, database) = setup_ephemeral_pool("postgres_schedule_earliest_plan", 1).await;
    support::register_test_job_definition(&pool, JOB_TYPE).await;
    sqlx::raw_sql(
        "INSERT INTO job_schedules (name, job_type, cron_expr, is_active, next_fire_at)
         SELECT 'earliest-plan-' || n, 'jobs.test.schedule_earliest_fire', '0 0 * * * * *',
                n % 10 = 0, now() + n * interval '1 second'
         FROM generate_series(1, 20000) n;
         ANALYZE job_schedules;",
    )
    .execute(&pool)
    .await
    .expect("populate schedules");

    assert!(earliest(&pool).await.is_some());
    // Bind the pattern so this lookup's own prepared text cannot match it.
    let name: String = sqlx::query_scalar(
        "SELECT name FROM pg_prepared_statements
         WHERE statement LIKE $1",
    )
    .bind("%min(next_fire_at)%FROM job_schedules%")
    .fetch_one(&pool)
    .await
    .expect("earliest-fire prepared statement");
    let statement = format!(
        "EXPLAIN (FORMAT JSON) EXECUTE \"{}\"",
        name.replace('"', "\"\"")
    );
    let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(statement))
        .fetch_one(&pool)
        .await
        .expect("explain earliest-fire statement");

    assert!(
        uses_index(&plan, "idx_job_schedules_next_fire"),
        "the earliest-fire read must use the active next-fire index: {plan}"
    );

    teardown_ephemeral_pool(pool, database).await;
}
