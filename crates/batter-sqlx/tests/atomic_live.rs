//! PostgreSQL 18 contracts for owned scopes; included in the explicit live runner.
#[path = "atomic_live/cancellation.rs"]
mod cancellation;
#[path = "atomic_live/commit.rs"]
mod commit;
#[path = "atomic_live/fail_fast.rs"]
mod fail_fast;
#[path = "atomic_live/fail_fast_context.rs"]
mod fail_fast_context;
#[path = "atomic_live/fail_fast_loss.rs"]
mod fail_fast_loss;
#[path = "atomic_live/fail_fast_profile.rs"]
mod fail_fast_profile;
#[path = "atomic_live/poison.rs"]
mod poison;
#[path = "atomic_live/policy.rs"]
mod policy;
#[path = "atomic_live/policy_cancellation.rs"]
mod policy_cancellation;
#[path = "atomic_live/policy_context.rs"]
mod policy_context;
#[path = "atomic_live/profile.rs"]
mod profile;
#[path = "atomic_live/profile_timeout_drift.rs"]
mod profile_timeout_drift;
#[path = "atomic_live/profile_timeout_expiry.rs"]
mod profile_timeout_expiry;
#[path = "atomic_live/profile_timeouts.rs"]
mod profile_timeouts;
#[path = "atomic_live/profiled_pool.rs"]
mod profiled_pool;
#[path = "atomic_live/runner.rs"]
mod runner;
#[path = "atomic_live/session_state.rs"]
mod session_state;
#[path = "atomic_live/snapshots.rs"]
mod snapshots;
mod support;
#[path = "atomic_live/transactions.rs"]
mod transactions;
#[path = "atomic_live/validation.rs"]
mod validation;
#[path = "atomic_live/validation_drift.rs"]
mod validation_drift;

use batter_sqlx::{PgScopedSql, low_level::PgAtomicTransaction};
use support::{Fixture, Result};

async fn fixture() -> Result<Fixture> {
    let mut fixture = Fixture::new().await?;
    let version: String = sqlx::query_scalar("SHOW server_version")
        .fetch_one(&mut fixture.observer)
        .await?;
    let number: String = sqlx::query_scalar("SHOW server_version_num")
        .fetch_one(&mut fixture.observer)
        .await?;
    assert_eq!(number.parse::<u32>()? / 10_000, 18);
    eprintln!("owned-scope PostgreSQL server_version={version}");
    Ok(fixture)
}

async fn pid(sql: &mut PgScopedSql<'_>) -> std::result::Result<i32, sqlx::Error> {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(sql.executor())
        .await
}

async fn transaction(pool: &sqlx::PgPool) -> Result<(PgAtomicTransaction, i32)> {
    Ok(PgAtomicTransaction::begin(pool)
        .await?
        .application(async |sql| pid(sql).await)
        .await?)
}

async fn replacement(pool: &sqlx::PgPool, previous: i32) -> Result {
    let next: i32 =
        support::bounded(sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(pool)).await??;
    assert_ne!(
        next, previous,
        "terminal owner returned its connection to the pool"
    );
    Ok(())
}
