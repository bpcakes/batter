//! Native SQLx pool lifecycle and error-preserving partial-startup cleanup.
//! No Runledger schema is created here. No business transaction is retried.
mod support;
#[cfg(test)]
mod tests;

use batter::{
    BoxError,
    cleanup::CleanupReport,
    lifecycle::{SharedShutdownReport, Supervisor},
    operation::OperationContext,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{fmt, io::Write, process::ExitCode, time::Duration};

#[derive(thiserror::Error)]
#[error("process failed")]
struct ProcessFailure {
    #[source]
    cause: BoxError,
}

impl fmt::Debug for ProcessFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[derive(thiserror::Error)]
#[error("startup failed; {cleanup}")]
struct StartupFailure {
    #[source]
    cause: BoxError,
    cleanup: CleanupReport,
}

impl fmt::Debug for StartupFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

struct ShutdownFailure {
    report: SharedShutdownReport,
}

impl fmt::Debug for ShutdownFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for ShutdownFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "shutdown failed; {}", *self.report)
    }
}

impl std::error::Error for ShutdownFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.report)
    }
}

fn process_result(result: Result<(), BoxError>) -> Result<(), ProcessFailure> {
    result.map_err(|cause| ProcessFailure { cause })
}

fn complete_shutdown(report: SharedShutdownReport) -> Result<(), BoxError> {
    if report.is_success() {
        Ok(())
    } else {
        Err(Box::new(ShutdownFailure { report }))
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    report_exit(run().await, &mut std::io::stderr().lock())
}

fn report_exit(result: Result<(), BoxError>, diagnostics: &mut impl Write) -> ExitCode {
    match process_result(result) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            // Retain the complete failure for an application-selected trusted
            // sink. Only this known wrapper's fixed Display reaches stderr.
            // A failed diagnostic write must not turn a failure into success.
            let _ = writeln!(diagnostics, "Error: {failure}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), BoxError> {
    tracing_subscriber::fmt().with_target(false).try_init()?;
    let database_url = std::env::var("DATABASE_URL")?;
    let mut supervisor = Supervisor::new(support::shutdown_budget());
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(3))
        .connect(&database_url)
        .await?;
    register_pool_close(&mut supervisor, &pool)?;

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
    complete_startup(supervisor, startup).await
}

fn register_pool_close(supervisor: &mut Supervisor, pool: &PgPool) -> Result<(), BoxError> {
    let closing_pool = pool.clone();
    supervisor.on_cleanup("postgres.pool", move || async move {
        closing_pool.close().await;
        Ok(())
    })?;
    Ok(())
}

async fn complete_startup(
    mut supervisor: Supervisor,
    startup: Result<(), BoxError>,
) -> Result<(), BoxError> {
    if let Err(cause) = startup {
        let cleanup = supervisor
            .take_cleanup()
            .close(support::cleanup_budget())
            .await;
        return Err(Box::new(StartupFailure { cause, cleanup }));
    }
    supervisor.handle().mark_ready();
    let running = supervisor.start();
    if running.handle().wait_ready().await.is_ok() {
        tracing::info!("PostgreSQL lifecycle ready");
    }
    let report = running.wait().await?;
    complete_shutdown(report)
}
