//! Native SQLx pool lifecycle and error-preserving partial-startup cleanup.
//! No Runledger schema is created here. No business transaction is retried.
mod support;

use batter::{
    BoxError,
    cleanup::CleanupReport,
    lifecycle::{ShutdownReport, Supervisor},
    operation::OperationContext,
};
use sqlx::postgres::PgPoolOptions;
use std::{fmt, sync::Arc, time::Duration};

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
    report: Arc<ShutdownReport>,
}

impl fmt::Debug for ShutdownFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for ShutdownFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "shutdown failed; {}", self.report)
    }
}

impl std::error::Error for ShutdownFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.report.as_ref())
    }
}

fn process_result(result: Result<(), BoxError>) -> Result<(), ProcessFailure> {
    result.map_err(|cause| ProcessFailure { cause })
}

async fn complete_startup(
    mut supervisor: Supervisor,
    startup: Result<(), BoxError>,
) -> Result<Supervisor, BoxError> {
    match startup {
        Ok(()) => Ok(supervisor),
        Err(cause) => {
            let cleanup = supervisor
                .take_cleanup()
                .close(support::cleanup_budget())
                .await;
            Err(Box::new(StartupFailure { cause, cleanup }))
        }
    }
}

fn complete_shutdown(report: Arc<ShutdownReport>) -> Result<(), BoxError> {
    if report.is_success() {
        Ok(())
    } else {
        Err(Box::new(ShutdownFailure { report }))
    }
}

#[tokio::main]
async fn main() -> Result<(), ProcessFailure> {
    process_result(run().await)
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
    let supervisor = complete_startup(supervisor, startup).await?;
    supervisor.handle().mark_ready();
    let running = supervisor.start();
    let report = running.wait().await?;
    complete_shutdown(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use batter::{
        cleanup::CleanupOutcome,
        lifecycle::{ShutdownCause, TaskOutcome},
    };

    const STARTUP_DETAIL: &str = "sensitive startup detail";
    const CLEANUP_DETAIL: &str = "sensitive cleanup detail";
    const SHUTDOWN_DETAIL: &str = "sensitive shutdown detail";
    const SHUTDOWN_CLEANUP_DETAIL: &str = "sensitive shutdown cleanup detail";

    fn assert_process_error_is_redacted(error: &ProcessFailure, details: &[&str]) {
        assert_eq!(error.to_string(), "process failed");
        let debug = format!("{error:?}");
        assert_eq!(debug, "process failed");
        for detail in details {
            assert!(!debug.contains(detail));
        }
    }

    #[test]
    fn process_boundary_retains_and_redacts_an_early_failure() {
        let result: Result<(), BoxError> = Err(std::io::Error::other(STARTUP_DETAIL).into());
        let error = process_result(result).expect_err("early failure must reach the boundary");
        let source = std::error::Error::source(&error)
            .expect("the concrete early failure must remain the source");

        assert!(std::ptr::addr_eq(source, error.cause.as_ref()));
        assert_eq!(source.to_string(), STARTUP_DETAIL);
        assert_process_error_is_redacted(&error, &[STARTUP_DETAIL]);
    }

    #[tokio::test]
    async fn startup_failure_path_retains_real_cleanup_diagnostics() {
        let mut supervisor = Supervisor::new(support::shutdown_budget());
        supervisor
            .on_cleanup("dependency", || async { Ok(()) })
            .unwrap();
        supervisor
            .on_cleanup("resource", || async {
                Err(std::io::Error::other(CLEANUP_DETAIL).into())
            })
            .unwrap();

        let result = complete_startup(
            supervisor,
            Err(std::io::Error::other(STARTUP_DETAIL).into()),
        )
        .await
        .map(|_| ());
        let error = process_result(result).expect_err("startup failure must reach the boundary");
        let failure = std::error::Error::source(&error)
            .expect("typed startup failure must remain the process source")
            .downcast_ref::<StartupFailure>()
            .expect("typed startup failure must survive BoxError conversion");

        let source = std::error::Error::source(failure)
            .expect("startup cause must remain in the error source chain");
        assert!(std::ptr::addr_eq(source, failure.cause.as_ref()));
        assert_eq!(source.to_string(), STARTUP_DETAIL);
        assert_eq!(failure.cleanup.records.len(), 2);
        assert_eq!(failure.cleanup.records[0].name, "resource");
        assert_eq!(failure.cleanup.records[0].outcome, CleanupOutcome::Failed);
        assert_eq!(
            failure.cleanup.records[0]
                .error
                .as_deref()
                .expect("cleanup error must be retained")
                .to_string(),
            CLEANUP_DETAIL
        );
        assert_eq!(failure.cleanup.records[1].name, "dependency");
        assert_eq!(
            failure.cleanup.records[1].outcome,
            CleanupOutcome::Succeeded
        );
        assert!(failure.cleanup.skipped.is_empty());

        let diagnostic = format!("{failure:?}");
        assert_eq!(
            diagnostic,
            "startup failed; cleanup: 1 unsuccessful, 0 skipped"
        );
        assert!(!diagnostic.contains(STARTUP_DETAIL));
        assert!(!diagnostic.contains(CLEANUP_DETAIL));
        assert_process_error_is_redacted(&error, &[STARTUP_DETAIL, CLEANUP_DETAIL]);
    }

    #[tokio::test]
    async fn shutdown_failure_path_retains_the_complete_report() {
        let mut supervisor = Supervisor::new(support::shutdown_budget());
        supervisor
            .on_cleanup("resource", || async {
                Err(std::io::Error::other(SHUTDOWN_CLEANUP_DETAIL).into())
            })
            .unwrap();
        supervisor
            .register("component", |shutdown| async move {
                shutdown.mark_started();
                Err(std::io::Error::other(SHUTDOWN_DETAIL).into())
            })
            .unwrap();

        let running = supervisor.start();
        let report = running
            .wait()
            .await
            .expect("component failure must still publish a shutdown report");
        let error = process_result(complete_shutdown(report))
            .expect_err("unsuccessful shutdown must reach the process boundary");
        let failure = std::error::Error::source(&error)
            .expect("typed shutdown failure must remain the process source")
            .downcast_ref::<ShutdownFailure>()
            .expect("typed shutdown failure must survive BoxError conversion");
        let report_source = std::error::Error::source(failure)
            .expect("shutdown report must remain in the error source chain");

        assert!(std::ptr::addr_eq(report_source, failure.report.as_ref()));
        assert_eq!(
            failure.report.cause,
            ShutdownCause::ComponentExit("component")
        );
        assert_eq!(failure.report.tasks.len(), 1);
        assert_eq!(failure.report.tasks[0].outcome, TaskOutcome::Failed);
        assert_eq!(
            failure.report.tasks[0]
                .error
                .as_deref()
                .expect("task error must be retained")
                .to_string(),
            SHUTDOWN_DETAIL
        );
        assert_eq!(failure.report.cleanup.records.len(), 1);
        assert_eq!(
            failure.report.cleanup.records[0]
                .error
                .as_deref()
                .expect("shutdown cleanup error must be retained")
                .to_string(),
            SHUTDOWN_CLEANUP_DETAIL
        );

        let diagnostic = format!("{failure:?}");
        assert!(!diagnostic.contains(SHUTDOWN_DETAIL));
        assert!(!diagnostic.contains(SHUTDOWN_CLEANUP_DETAIL));
        assert_process_error_is_redacted(&error, &[SHUTDOWN_DETAIL, SHUTDOWN_CLEANUP_DETAIL]);
    }
}
