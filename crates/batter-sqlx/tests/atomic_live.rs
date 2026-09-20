//! PostgreSQL 18 contracts for owned scopes; included in the explicit live runner.
#[path = "atomic_live/cancellation.rs"]
mod cancellation;
#[path = "atomic_live/commit.rs"]
mod commit;
#[path = "atomic_live/snapshots.rs"]
mod snapshots;
mod support;
#[path = "atomic_live/transactions.rs"]
mod transactions;

use batter_sqlx::{PgAtomicTransaction, PgScopedSql};
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
