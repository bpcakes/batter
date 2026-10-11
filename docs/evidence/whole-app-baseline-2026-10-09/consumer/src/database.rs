//! PostgreSQL ownership: a lazily connected, profiled pool whose close is a
//! registered finalizer, plus the schema work done during owned startup.

use crate::settings::DatabaseSettings;
use batter::BoxError;
use batter::health::{HealthMonitor, HealthPolicy, HealthReader};
use batter::operation::OperationContext;
use batter::runledger::native::postgres::migrate_after_idempotency_cutover;
use batter::runledger::{PgSessionProfile, RunledgerDatabase};
use batter::startup::ProtectedStartupScope;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

/// The single authoritative schema for both the Runledger tables and `records`.
const SCHEMA: &str = "public";

/// Application table. Runledger's own schema is applied by its migrator.
const RECORDS_TABLE: &str = "CREATE TABLE IF NOT EXISTS records (\
    id uuid primary key, \
    name text not null, \
    created_at timestamptz not null default now(), \
    notified_at timestamptz)";

/// Reserve the pool's finalizer, construct the lazy profiled pool, and register
/// its close before returning. Nothing here connects.
pub fn open(
    scope: &mut ProtectedStartupScope,
    settings: &DatabaseSettings,
) -> Result<RunledgerDatabase, BoxError> {
    let profile = PgSessionProfile::new(
        settings.login(),
        settings.login(),
        vec![SCHEMA.to_owned()],
        Duration::from_secs(30),
        Duration::from_secs(5),
    )?;
    let slot = scope.reserve_cleanup("postgres.pool")?;
    let database = RunledgerDatabase::connect_lazy(
        settings.connect_options(),
        profile,
        PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(5)),
    )?;
    let closing = database.pool().clone();
    slot.register(move || async move {
        closing.close().await;
        Ok(())
    });
    Ok(database)
}

/// Create the application table and apply Runledger's bundled migrations,
/// both bounded by the startup budget.
pub async fn prepare_schema(
    context: &OperationContext,
    database: &RunledgerDatabase,
) -> Result<(), BoxError> {
    context
        .run("schema.records", |_| async {
            sqlx::query(RECORDS_TABLE)
                .execute(database.pool())
                .await
                .map(|_| ())
        })
        .await?;
    context
        .run("schema.runledger", |_| async {
            migrate_after_idempotency_cutover(database).await
        })
        .await?;
    Ok(())
}

/// Register a sequential `SELECT 1` sampler and return its read-only view for
/// readiness. The monitor bounds each probe, including acquisition.
pub fn register_health(
    scope: &mut ProtectedStartupScope,
    database: &RunledgerDatabase,
) -> Result<HealthReader<sqlx::Error>, BoxError> {
    let policy = HealthPolicy::new(
        Duration::from_secs(2),
        Duration::from_secs(3),
        Duration::from_secs(10),
        Duration::from_secs(1),
    )?;
    let pool = database.pool().clone();
    let reader = HealthMonitor::new(policy, move || {
        let pool = pool.clone();
        async move { sqlx::query("SELECT 1").execute(&pool).await.map(|_| ()) }
    })
    .register_in(scope, "postgres.health")?;
    Ok(reader)
}
