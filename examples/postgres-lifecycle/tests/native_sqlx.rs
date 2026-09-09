//! Standalone SQLx type probe; Runledger and harness dependencies are unnecessary.

use sqlx::{Acquire, PgConnection, PgPool, Postgres, Transaction};

async fn transaction_boundary(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut connection = pool.acquire().await?;
    let connection: &mut PgConnection = &mut connection;
    let mut transaction: Transaction<'_, Postgres> = connection.begin().await?;
    let connection: &mut PgConnection = &mut transaction;
    let _: i32 = sqlx::query_scalar("SELECT 1").fetch_one(connection).await?;
    transaction.rollback().await
}

#[tokio::test]
async fn native_transaction_factory_is_inert() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .min_connections(0)
        .connect_lazy("postgres://localhost/unused_compile_probe")
        .expect("static URL");
    let future = transaction_boundary(&pool);
    drop(future);
    assert_eq!(
        pool.size(),
        0,
        "unpolled factory must not acquire a connection"
    );
    pool.close().await;
}
