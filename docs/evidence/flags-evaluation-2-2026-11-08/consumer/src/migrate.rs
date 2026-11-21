//! `checkout-service migrate`: the finite migration command.
//!
//! Run it under the schema-owner login (`CHECKOUT_MIGRATION_DATABASE_URL`)
//! before the build that needs the schema serves. Serving never migrates. The
//! order is fixed: Runledger's history first (it filters shared SQLx history
//! through its own fence), then this application's bundle, which tolerates the
//! Runledger versions already recorded in the shared `_sqlx_migrations`, then
//! the flags store, whose ledger lives in its own `flags` schema.

use std::{error::Error, fmt, time::Duration};

use batter::{
    BoxError,
    cleanup::CleanupBudget,
    command::{Command, CommandCause, CommandFailure},
    flags::postgres::{PooledStore, StoreTiming},
    operation::OperationOwner,
    runledger::{
        PgSessionProfile, RunledgerDatabase, native::postgres::migrate_after_idempotency_cutover,
    },
    sqlx::PgLease,
};
use sqlx::postgres::PgPoolOptions;

use crate::config::MigrateConfig;

/// Which migration step failed, retaining its cause without formatting it.
pub struct MigrationFailure {
    step: &'static str,
    cause: BoxError,
}

fn step<T, E: Into<BoxError>>(
    step: &'static str,
    result: Result<T, E>,
) -> Result<T, MigrationFailure> {
    result.map_err(|cause| MigrationFailure {
        step,
        cause: cause.into(),
    })
}

impl fmt::Display for MigrationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "migration step `{}` failed", self.step)
    }
}

impl fmt::Debug for MigrationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Error for MigrationFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause.as_ref())
    }
}

fn application_migrator() -> sqlx::migrate::Migrator {
    let mut migrator = sqlx::migrate!();
    // Runledger's versions share `_sqlx_migrations`; they are not ours to know.
    migrator.set_ignore_missing(true);
    migrator
}

/// Runs every migration as one finite command whose pool closure is a
/// registered finalizer. SIGINT/SIGTERM cancels the work and still closes it.
pub async fn run(config: MigrateConfig) -> Result<(), BoxError> {
    let second = Duration::from_secs(1);
    let context = OperationOwner::new(second * 120)?.into_context();
    let cleanup = CleanupBudget::new(second * 5, second * 5, second)?;
    let command = Command::new(context, cleanup, move |scope| {
        Box::pin(async move {
            step("connect", scope.stage("migrate.connect"))?;
            let login = config.database;
            let profile = step(
                "connect",
                PgSessionProfile::new(
                    &login.role,
                    &login.role,
                    vec!["public".into()],
                    Duration::ZERO,
                    Duration::ZERO,
                ),
            )?;
            let slot = step("connect", scope.reserve_cleanup("database"))?;
            let pool = step(
                "connect",
                batter::sqlx::pool_in(
                    slot,
                    PgPoolOptions::new().max_connections(2),
                    login.options,
                    profile,
                ),
            )?;
            let database = step("connect", RunledgerDatabase::from_profiled(pool.clone()))?;

            step("runledger", scope.stage("migrate.runledger"))?;
            step(
                "runledger",
                migrate_after_idempotency_cutover(&database).await,
            )?;

            step("application", scope.stage("migrate.application"))?;
            let lease = step(
                "application",
                PgLease::acquire(pool.pool(), scope.context()).await,
            )?;
            step("application", lease.migrate(&application_migrator()).await)?;

            step("flags", scope.stage("migrate.flags"))?;
            let timing = step(
                "flags",
                StoreTiming::new(second * 5, second * 30, second * 5, second * 30),
            )?;
            let store = PooledStore::new(pool.pool().clone(), timing);
            step(
                "flags",
                store
                    .migrate(tokio::time::Instant::now() + second * 60)
                    .await,
            )?;
            Ok::<_, MigrationFailure>(())
        })
    });
    match command.start_with_unix_signals()?.finish().await {
        Ok(success) => {
            success.into_value();
            println!("checkout-service: migrations applied");
            Ok(())
        }
        Err(CommandFailure::Report(report)) => {
            // Both formats are redacted: they name the stage and step, never a cause.
            eprintln!("checkout-service: migrate failed: {}", *report);
            if let Err(CommandCause::Failed(failure)) = &report.work {
                eprintln!("checkout-service: {failure}");
            }
            Err("migration failed".into())
        }
        Err(CommandFailure::Coordinator(_)) => Err("migration coordinator failed".into()),
    }
}
