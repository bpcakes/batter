//! Owned production startup for the command-only reference service.

use crate::{config::RootSettings, http::router, schema::initialize_schema};
use batter::{
    BoxError,
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, check_shutdown, register_signals},
    operation::OperationContext,
    startup::Startup,
};
use std::{fmt, time::Duration};

/// Fixed application startup failure wrapper used by sanitized process output.
pub struct InitializationFailure(BoxError);

impl fmt::Display for InitializationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("reference service initialization failed")
    }
}

impl fmt::Debug for InitializationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for InitializationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

fn initialization(error: impl Into<BoxError>) -> InitializationFailure {
    InitializationFailure(error.into())
}

fn cleanup_budget() -> CleanupBudget {
    let second = Duration::from_secs(1);
    CleanupBudget::new(Duration::from_secs(5), second, second)
        .expect("static cleanup budget is valid")
}

fn shutdown_budget() -> ShutdownBudget {
    let second = Duration::from_secs(1);
    ShutdownBudget::new(
        Duration::from_secs(10),
        Duration::from_secs(3),
        second,
        cleanup_budget(),
    )
    .expect("static shutdown budget is valid")
}

/// Run the command-only production root until a registered Unix signal drains it.
///
/// Startup uses the configured supervisor/pool/listener/authentication/request
/// constructors, applies both migration histories, synchronizes the producer job
/// definition, and registers explicit pool cleanup before approving readiness.
/// No Runledger worker or provider handler is started by this task.
pub async fn run(settings: RootSettings) -> Result<(), BoxError> {
    let supervisor = settings.supervisor(shutdown_budget())?;
    let handle = supervisor.handle();
    let readiness = handle.clone();
    let cleanup = cleanup_budget();
    let mut starting = Startup::new(
        supervisor,
        OperationContext::new(Duration::from_secs(20))?,
        cleanup,
        move |scope| {
            Box::pin(async move {
                let result: Result<(), BoxError> = async {
                    scope.stage("postgres.acquire")?;
                    let pool_cleanup = scope.supervisor().reserve_cleanup("postgres.pool")?;
                    let options = settings.connect_options_from_process()?;
                    let pool = settings.pool_options().connect_with(options).await?;
                    let closing = pool.clone();
                    pool_cleanup.register(move || async move {
                        closing.close().await;
                        Ok::<_, BoxError>(())
                    });

                    scope.stage("postgres.schema")?;
                    initialize_schema(&pool).await?;

                    scope.stage("http.bind")?;
                    let application = router(&settings, readiness.clone(), pool)?;
                    let listener = tokio::net::TcpListener::bind(settings.bind()).await?;
                    batter_axum::register_http(scope.supervisor(), "http", listener, application)?;

                    scope.stage("signals")?;
                    register_signals(scope.supervisor(), "signals")?;
                    readiness.mark_ready();
                    Ok(())
                }
                .await;
                result.map_err(initialization)
            })
        },
    )
    .start();
    let running = starting.wait().await?;
    check_shutdown(running.wait().await)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_budgets_preserve_cleanup_inside_total_shutdown() {
        let _cleanup = cleanup_budget();
        let _shutdown = shutdown_budget();
    }

    #[test]
    fn initialization_diagnostic_is_fixed_while_source_remains_available() {
        use std::error::Error;
        let failure = initialization(std::io::Error::other("private-marker"));
        assert_eq!(
            failure.to_string(),
            "reference service initialization failed"
        );
        assert!(!format!("{failure:?}").contains("private-marker"));
        assert!(
            failure
                .source()
                .unwrap()
                .to_string()
                .contains("private-marker")
        );
    }
}
