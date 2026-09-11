//! Native compatibility seams for the unpublished reference application.
//!
//! ```no_run
//! # async fn probe(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
//! let pool = batter_example_reference_service::native_pool(pool);
//! let mut transaction = pool.begin().await?;
//! let _: &mut runledger_postgres::DbTx<'_> =
//!     batter_example_reference_service::native_transaction(&mut transaction);
//! let connection = batter_example_reference_service::native_connection(&mut transaction);
//! let _: i32 = sqlx::query_scalar("SELECT 1").fetch_one(connection).await?;
//! transaction.rollback().await?;
//! # Ok(())
//! # }
//! ```
#![forbid(unsafe_code)]

/// Application-owned bearer authentication producing trusted owner identity.
pub mod auth;

/// Proves the application pool and upstream pool are the same native type.
pub fn native_pool(pool: &sqlx::PgPool) -> &runledger_postgres::DbPool {
    pool
}

/// Preserves the caller-owned transaction without conversion or wrapping.
pub fn native_transaction<'borrow, 'connection>(
    transaction: &'borrow mut sqlx::Transaction<'connection, sqlx::Postgres>,
) -> &'borrow mut runledger_postgres::DbTx<'connection> {
    transaction
}

/// Borrows the native connection inside the caller-owned transaction.
pub fn native_connection<'borrow>(
    transaction: &'borrow mut runledger_postgres::DbTx<'_>,
) -> &'borrow mut sqlx::PgConnection {
    transaction
}

/// Application-owned validated settings and explicit native constructors.
pub mod config;

/// Atomic application command, durable delivery projection and query paths.
pub mod delivery;

/// Authenticated HTTP command and owner-scoped reconciliation routes.
pub mod http;

/// Owned staged service startup and shutdown composition.
pub mod runtime;

/// Forward-only application/Runledger startup migration and definition sync.
pub mod schema;

/// Probe-only Runledger worker hosting and termination certainty.
pub mod worker;
