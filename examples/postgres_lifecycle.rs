//! Native SQLx pool lifecycle and error-preserving partial-startup cleanup.
//! No Runledger schema is created here. No business transaction is retried.
mod support;

use batter::{
    BoxError, cleanup::CleanupReport, lifecycle::Supervisor, operation::OperationContext,
};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
#[error("startup failed; {cleanup}")]
struct StartupFailure {
    #[source]
    cause: BoxError,
    cleanup: CleanupReport,
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    tracing_subscriber::fmt().with_target(false).try_init()?;
    let database_url = std::env::var("DATABASE_URL")?;
    let mut supervisor = Supervisor::new(support::shutdown_budget());
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(3))
        .connect(&database_url)
        .await?;
    let closing_pool = pool.clone();
    supervisor.on_cleanup("postgres.pool", move || async move {
        closing_pool.close().await;
        Ok(())
    })?;

    let startup: Result<(), BoxError> = async {
        // Schema compatibility/migrations are application-owned and explicit.
        let probe = OperationContext::new(Duration::from_secs(5))?;
        let one: i32 = probe
            .run("postgres.readiness", |_| async {
                sqlx::query_scalar("SELECT 1::integer")
                    .fetch_one(&pool)
                    .await
            })
            .await?;
        if one != 1 {
            return Err(std::io::Error::other("unexpected database probe result").into());
        }
        support::register_signals(&mut supervisor)?;
        supervisor.register("application", |shutdown| async move {
            // A real application would register its server and/or Runledger host.
            shutdown.mark_started();
            shutdown.draining().await;
            Ok(())
        })?;
        Ok(())
    }
    .await;
    if let Err(cause) = startup {
        let cleanup = supervisor
            .take_cleanup()
            .close(support::cleanup_budget())
            .await;
        let failure = StartupFailure { cause, cleanup };
        // The complete object is available to a trusted error sink. Its Display
        // deliberately avoids printing the database error/cause by default.
        return Err(std::io::Error::other(failure.to_string()).into());
    }
    supervisor.handle().mark_ready();
    let running = supervisor.start();
    let report = running.wait().await?;
    if !report.is_success() {
        return Err(std::io::Error::other(report.to_string()).into());
    }
    Ok(())
}
