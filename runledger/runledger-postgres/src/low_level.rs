//! Native database construction without registered lifecycle ownership.
use crate::{PgSessionProfile, RunledgerDatabase};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

/// Connect a standalone native database, directly probing profile configuration.
/// Migration: `RunledgerDatabase::connect` moved here. Protected applications use
/// `batter_sqlx::pool_in` then [`RunledgerDatabase::from_profiled`]. This constructor
/// registers no finalizer; the caller owns failure cleanup and awaited closure
/// after every dependent stops. No runtime-death guarantee is implied.
pub async fn connect_database(
    options: PgConnectOptions,
    profile: PgSessionProfile,
    max_connections: u32,
) -> Result<RunledgerDatabase, sqlx::Error> {
    RunledgerDatabase::connect(options, profile, max_connections).await
}

/// Construct a lazy standalone database with caller-owned cleanup.
/// Migration: `RunledgerDatabase::connect_lazy` moved here. Prefer protected
/// `batter_sqlx::pool_in` then [`RunledgerDatabase::from_profiled`]. Reserve cleanup
/// before this call and register close without yielding: SQLx may start minimum
/// connection maintenance immediately. Session hooks remain profile-owned.
pub fn connect_database_lazy(
    options: PgConnectOptions,
    profile: PgSessionProfile,
    pool_options: PgPoolOptions,
) -> Result<RunledgerDatabase, sqlx::Error> {
    RunledgerDatabase::connect_lazy(options, profile, pool_options)
}
