//! Owned startup and native runtime registration for the staged reference service.

use crate::{config::RootSettings, http::router, schema::initialize_schema};
use batter::{
    BoxError,
    cleanup::CleanupBudget,
    health::{HealthMonitor, HealthPolicy, HealthReader},
    lifecycle::{ShutdownBudget, Supervisor, check_shutdown, install_signals},
    operation::{OperationContext, OperationError},
    startup::{Startup, StartupError},
};
use std::{fmt, time::Duration};

const STARTUP_ALLOWANCE: Duration = Duration::from_secs(20);

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
    CleanupBudget::new(
        Duration::from_secs(5),
        Duration::from_secs(5),
        Duration::from_secs(1),
    )
    .expect("static cleanup budget is valid")
}

fn shutdown_budget() -> ShutdownBudget {
    ShutdownBudget::new(
        Duration::from_secs(10),
        Duration::from_secs(1),
        Duration::from_secs(1),
        cleanup_budget(),
    )
    .expect("static shutdown budget is valid")
}

/// Failed owned startup, including its independently driven cleanup report.
/// Default formatting excludes native causes; inspection remains explicit.
pub struct RuntimeStartupFailure(StartupError<InitializationFailure>);
impl RuntimeStartupFailure {
    /// Inspect the actual initialization cause and cleanup outcomes.
    pub fn startup(&self) -> &StartupError<InitializationFailure> {
        &self.0
    }
}
impl fmt::Display for RuntimeStartupFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("reference service startup failed")
    }
}
impl fmt::Debug for RuntimeStartupFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for RuntimeStartupFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// Run until a registered Unix signal or component failure initiates shutdown.
/// Native loop acknowledgement, fresh PostgreSQL health and explicit application
/// approval are separate. No production startup control job is created.
/// The delivery handler is still absent, so the root withholds approval and
/// keeps /ready unavailable; its empty native registry cannot claim delivery jobs.
pub async fn run(settings: RootSettings) -> Result<(), BoxError> {
    let supervisor = settings.supervisor(shutdown_budget())?;
    let readiness = supervisor.handle();
    let context = OperationContext::new(STARTUP_ALLOWANCE)?;
    let native_startup = context.clone();
    let mut starting = Startup::new(supervisor, context, cleanup_budget(), move |scope| {
        Box::pin(async move {
            scope.stage("signals.install").map_err(initialization)?;
            let mut signals =
                install_signals(scope.supervisor(), "signals").map_err(initialization)?;
            let initialize = async {
                let result: Result<(), BoxError> = async {
                    scope.stage("postgres.acquire")?;
                    let pool = register_pool(scope, &settings)?;
                    drop(pool.acquire().await?);
                    scope.stage("postgres.schema")?;
                    initialize_schema(&pool).await?;
                    let health = register_health(scope.supervisor(), pool.clone())?;

                    scope.stage("http.bind")?;
                    let application = router(&settings, readiness.clone(), pool.clone(), health)?;
                    let listener = tokio::net::TcpListener::bind(settings.bind()).await?;
                    batter_axum::register_http(scope.supervisor(), "http", listener, application)?;

                    scope.stage("worker.register")?;
                    let config = settings.worker().jobs_config()?;
                    batter_runledger::register(scope.supervisor(), "worker", native_startup, {
                        runledger_runtime::Supervisor::builder(&pool, config)?
                            .with_registry(runledger_runtime::registry::JobRegistry::new())
                            .prepare()?
                    })?;
                    Ok(())
                }
                .await;
                result.map_err(initialization)
            };
            tokio::select! {
                biased;
                _ = signals.received() => { readiness.request(); return Ok(()); }
                result = initialize => result?,
            }
            signals
                .register(scope.supervisor())
                .map_err(initialization)?;
            Ok(())
        })
    })
    .without_readiness_approval()
    .start();
    let running = starting
        .wait()
        .await
        .map_err(|error| Box::new(RuntimeStartupFailure(error)) as BoxError)?;
    check_shutdown(running.wait().await)?;
    Ok(())
}

fn register_pool(
    scope: &mut batter::startup::StartupScope,
    settings: &RootSettings,
) -> Result<sqlx::PgPool, BoxError> {
    let slot = scope.supervisor().reserve_cleanup("postgres.pool")?;
    let options = settings.connect_options_from_process()?;
    let pool = settings.pool_options().connect_lazy_with(options);
    let closing = pool.clone();
    slot.register(move || async move {
        closing.close().await;
        Ok(())
    });
    Ok(pool)
}

fn register_health(
    process: &mut Supervisor,
    pool: sqlx::PgPool,
) -> Result<HealthReader<OperationError<batter_sqlx::SqlxFailure>>, BoxError> {
    let second = Duration::from_secs(1);
    let policy = HealthPolicy::new(second, second, Duration::from_secs(3), second)?;
    let monitor = HealthMonitor::new(policy, move || {
        let pool = pool.clone();
        async move {
            let context = OperationContext::new(second).expect("static probe budget is valid");
            batter_sqlx::probe(&pool, &context).await
        }
    });
    let reader = monitor.reader();
    process.register("postgres.health", move |shutdown| async move {
        monitor.run(shutdown).await;
        Ok(())
    })?;
    Ok(reader)
}

#[cfg(test)]
mod tests {
    use super::*;

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
