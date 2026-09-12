//! Owned startup and native runtime registration for the staged reference service.

use crate::{config::RootSettings, http::router, schema::initialize_schema};
use batter::{
    BoxError,
    cleanup::CleanupBudget,
    health::{HealthMonitor, HealthPolicy, HealthReader},
    lifecycle::{ShutdownBudget, check_shutdown},
    operation::{OperationContext, OperationError},
    registration::RegistrationTarget,
    startup::{InitializationError, ProtectedStartupScope, Startup, StartupError},
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
pub struct RuntimeStartupFailure(StartupError<InitializationError<InitializationFailure>>);
impl RuntimeStartupFailure {
    /// Inspect the actual initialization cause and cleanup outcomes.
    pub fn startup(&self) -> &StartupError<InitializationError<InitializationFailure>> {
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
    let mut starting = Startup::scoped(supervisor, context, cleanup_budget(), move |scope| {
        Box::pin(async move {
            let result: Result<(), BoxError> = async {
                scope.stage("postgres.acquire")?;
                let pool = register_pool(scope, &settings)?;
                drop(pool.acquire().await?);
                scope.stage("postgres.schema")?;
                initialize_schema(&pool).await?;
                let health = register_health(scope, pool.clone())?;

                scope.stage("http.bind")?;
                let application = router(&settings, readiness.clone(), pool.clone(), health)?;
                let listener = tokio::net::TcpListener::bind(settings.bind()).await?;
                batter_axum::register_http_in(scope, "http", listener, application)?;

                scope.stage("worker.register")?;
                let config = settings.worker().jobs_config()?;
                batter_runledger::register_in(scope, "worker", native_startup, {
                    runledger_runtime::Supervisor::builder(&pool, config)?
                        .with_registry(runledger_runtime::registry::JobRegistry::new())
                        .prepare()?
                })?;
                Ok(())
            }
            .await;
            result.map_err(initialization)
        })
    })
    .without_readiness_approval()
    .with_unix_signals("signals")
    .start();
    let running = starting
        .wait()
        .await
        .map_err(|error| Box::new(RuntimeStartupFailure(error)) as BoxError)?;
    check_shutdown(running.wait().await)?;
    Ok(())
}

fn register_pool(
    scope: &mut ProtectedStartupScope,
    settings: &RootSettings,
) -> Result<sqlx::PgPool, BoxError> {
    let slot = scope.reserve_cleanup("postgres.pool")?;
    let options = settings.connect_options_from_process()?;
    let pool = settings.pool_options().connect_lazy_with(options);
    let closing = pool.clone();
    slot.register(move || async move {
        closing.close().await;
        Ok(())
    });
    Ok(pool)
}

fn register_health<T: RegistrationTarget + ?Sized>(
    target: &mut T,
    pool: sqlx::PgPool,
) -> Result<HealthReader<OperationError<batter_sqlx::SqlxFailure>>, BoxError> {
    let second = Duration::from_secs(1);
    let policy = HealthPolicy::new(second, second, Duration::from_secs(3), second)?;
    let reader = HealthMonitor::new(policy, move || {
        let pool = pool.clone();
        async move {
            let context = OperationContext::new(second).expect("static probe budget is valid");
            batter_sqlx::probe(&pool, &context).await
        }
    })
    .register_in(target, "postgres.health")?;
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
