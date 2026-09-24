//! Owned startup and native runtime registration for the staged reference service.

use crate::{
    config::{PreparedServing, ServingSettings},
    delivery::DeliveryWorker,
    http::register_in,
    schema::initialize_schema,
};
use batter::{
    BoxError,
    cleanup::CleanupBudget,
    health::{HealthMonitor, HealthPolicy, HealthReader},
    lifecycle::{ShutdownBudget, ShutdownSuccess},
    operation::OperationError,
    registration::Registration,
    startup::{InitializationError, ProtectedStartupScope, Startup, StartupError},
};
use runledger_core::jobs::JobExecutionHandler;
use std::{fmt, time::Duration};

mod announcement;

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

/// Prepare the fixed serving runtime without starting application work.
///
/// This is the only public constructor for [`PreparedServing`], so callers cannot
/// substitute an application-specific shutdown budget before entering [`run`].
/// Native PostgreSQL option construction occurs here so an ambient PG* conflict
/// is rejected before startup acquires resources or starts its driver.
///
/// Maintenance settings cannot be promoted into serving preparation:
///
/// ```compile_fail,E0308
/// use batter_example_reference_service::{config::MaintenanceSettings, runtime};
///
/// fn cannot_prepare(settings: MaintenanceSettings) {
///     let prepared = runtime::prepare(settings);
/// }
/// ```
pub fn prepare(
    settings: ServingSettings,
) -> Result<PreparedServing, batter::settings::SettingsError> {
    settings.prepare(shutdown_budget())
}

/// Failed protected startup returned, boxed, by [`run`].
///
/// The retained report distinguishes an application failure
/// ([`InitializationError::Application`]), a Unix signal policy failure and a
/// received signal ([`batter::startup::StartupCause::Draining`]), together with
/// every awaited cleanup outcome. Default formatting excludes native causes;
/// the error source exposes the protected startup error for explicit inspection.
///
/// # Examples
///
/// ```
/// use batter::BoxError;
/// use batter_example_reference_service::runtime::ProtectedRuntimeStartupFailure;
///
/// # #[allow(dead_code)]
/// fn inspect_startup(error: &BoxError) {
///     if let Some(failure) = error.downcast_ref::<ProtectedRuntimeStartupFailure>() {
///         let report = failure.startup();
///         // Inspect the typed cause and cleanup report without rendering secrets.
///         let _ = report;
///     }
/// }
/// ```
pub struct ProtectedRuntimeStartupFailure(StartupError<InitializationError<InitializationFailure>>);
impl ProtectedRuntimeStartupFailure {
    /// Inspect the actual initialization cause and cleanup outcomes.
    pub fn startup(&self) -> &StartupError<InitializationError<InitializationFailure>> {
        &self.0
    }
}
impl fmt::Display for ProtectedRuntimeStartupFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("reference service startup failed")
    }
}
impl fmt::Debug for ProtectedRuntimeStartupFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for ProtectedRuntimeStartupFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

fn startup_failure(error: StartupError<InitializationError<InitializationFailure>>) -> BoxError {
    Box::new(ProtectedRuntimeStartupFailure(error))
}

/// An otherwise successful runtime shutdown that did not retain its required
/// `postgres.pool` cleanup record.
///
/// Checked completion has already classified the generic shutdown report, so every
/// recorded task and cleanup outcome succeeded. This application-root failure
/// separately enforces that its required pool finalizer actually participated.
/// Formatting is fixed and does not inspect report contents; [`Self::report`]
/// provides typed access for an explicitly selected diagnostic sink.
///
/// # Examples
///
/// ```
/// use batter::BoxError;
/// use batter_example_reference_service::runtime::RuntimePoolCleanupFailure;
///
/// # #[allow(dead_code)]
/// fn inspect_shutdown(error: &BoxError) {
///     if let Some(failure) = error.downcast_ref::<RuntimePoolCleanupFailure>() {
///         let report = failure.report();
///         // Inspect the retained shutdown report without rendering native errors.
///         let _ = report;
///     }
/// }
/// ```
pub struct RuntimePoolCleanupFailure(batter::lifecycle::SharedShutdownReport);

impl RuntimePoolCleanupFailure {
    /// Inspect the otherwise successful shutdown report and its cleanup records.
    pub fn report(&self) -> &batter::lifecycle::SharedShutdownReport {
        &self.0
    }
}

impl fmt::Display for RuntimePoolCleanupFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .write_str("reference service shutdown did not retain its postgres.pool cleanup record")
    }
}

impl fmt::Debug for RuntimePoolCleanupFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for RuntimePoolCleanupFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// Run a purpose-qualified preparation until shutdown.
/// Native loop acknowledgement, fresh PostgreSQL health and explicit application
/// approval are separate. No production startup control job is created.
/// The provider handler is registered before native preparation, and ordinary
/// startup approval remains contingent on every critical acknowledgement.
///
/// Startup-owned SIGTERM/SIGINT listeners are installed before initialization.
/// A startup failure downcasts to [`ProtectedRuntimeStartupFailure`]. Generic
/// running failures downcast to [`batter::lifecycle::ShutdownFailure`]; an
/// otherwise successful report without its required pool-cleanup record
/// downcasts to [`RuntimePoolCleanupFailure`].
///
/// Maintenance preparation cannot cross this boundary:
///
/// ```compile_fail,E0308
/// use batter_example_reference_service::{config::PreparedMaintenance, runtime};
///
/// fn cannot_serve(prepared: PreparedMaintenance) {
///     let future = runtime::run(prepared);
/// }
/// ```
pub async fn run(prepared: PreparedServing) -> Result<(), BoxError> {
    let parts = prepared.into_parts();
    let supervisor = parts.supervisor;
    let lifecycle = supervisor.status();
    let admission = supervisor.operation_admission();
    let context = batter::operation::OperationOwner::new(STARTUP_ALLOWANCE)?.into_context();
    let native_startup = context.clone();
    let mut starting = Startup::scoped(supervisor, context, cleanup_budget(), move |scope| {
        Box::pin(async move {
            let result: Result<(), BoxError> = async {
                scope.stage("postgres.acquire")?;
                let database = register_pool(scope, parts.pool_options, parts.connect_options)?;
                let pool = database.pool().clone();
                drop(pool.acquire().await?);
                scope.stage("postgres.schema")?;
                initialize_schema(&database).await?;
                let health = register_health(scope.registration(), pool.clone())?;

                scope.stage("http.bind")?;
                let listener = tokio::net::TcpListener::bind(parts.bind).await?;
                announcement::publish(
                    parts.listener_announcement.as_deref(),
                    listener.local_addr()?,
                )
                .await?;
                register_in(
                    scope,
                    listener,
                    parts.http,
                    lifecycle.clone(),
                    admission.clone(),
                    database,
                    health,
                )?;

                scope.stage("worker.register")?;
                let mut registry = runledger_runtime::registry::JobRegistry::new();
                registry.try_register(
                    DeliveryWorker::new(pool.clone(), parts.provider, parts.provider_bulkhead)
                        .into_job_handler(),
                )?;
                batter::runledger::register_in(scope, "worker", native_startup, {
                    runledger_runtime::Supervisor::builder(&pool, parts.jobs)?
                        .with_registry(registry)
                        .prepare()?
                })?;
                Ok(())
            }
            .await;
            result.map_err(initialization)
        })
    })
    .with_unix_signals("signals")
    .start();
    let pending = starting.wait().await.map_err(startup_failure)?;
    check_application_shutdown(pending.wait_checked().await?)?;
    Ok(())
}

fn check_application_shutdown(success: ShutdownSuccess) -> Result<(), BoxError> {
    let report = success.into_report();
    let pool_recorded = report
        .cleanup
        .records
        .iter()
        .any(|record| record.name == "postgres.pool");
    if !pool_recorded {
        return Err(Box::new(RuntimePoolCleanupFailure(report)));
    }
    Ok(())
}

/// Publish native close before returning the lazy pool. The explicit acquisition
/// and schema stages that follow establish connectivity.
fn register_pool(
    scope: &mut ProtectedStartupScope,
    pool_options: sqlx::postgres::PgPoolOptions,
    connect_options: sqlx::postgres::PgConnectOptions,
) -> Result<runledger_postgres::RunledgerDatabase, BoxError> {
    let slot = scope.reserve_cleanup("postgres.pool")?;
    let database = crate::database::configured(connect_options, pool_options)?;
    let closing = database.pool().clone();
    slot.register(move || async move {
        closing.close().await;
        Ok(())
    });
    Ok(database)
}

fn register_health(
    mut registration: Registration<'_>,
    pool: sqlx::PgPool,
) -> Result<HealthReader<OperationError<batter::sqlx::SqlxFailure>>, BoxError> {
    let second = Duration::from_secs(1);
    let policy = HealthPolicy::new(second, second, Duration::from_secs(3), second)?;
    let reader = HealthMonitor::new(policy, move || {
        let pool = pool.clone();
        async move {
            let context = batter::operation::OperationOwner::new(second)
                .map(|owner| owner.into_context())
                .expect("static probe budget is valid");
            batter::sqlx::probe(&pool, &context).await
        }
    })
    .register_in(&mut registration, "postgres.health")?;
    Ok(reader)
}

#[cfg(test)]
mod tests {
    use super::*;
    use batter::{
        cleanup::CleanupOutcome,
        lifecycle::{Readiness, Supervisor},
        startup::{StartupCause, StartupFailure},
    };
    use std::{
        error::Error,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    const PRIVATE: &str = "private-marker";
    type ProtectedReport = StartupFailure<InitializationError<InitializationFailure>>;

    fn context() -> batter::operation::OperationContext {
        batter::operation::OperationOwner::new(Duration::from_secs(5))
            .unwrap()
            .into_context()
    }

    fn protected(error: &BoxError) -> &ProtectedReport {
        let failure = error
            .downcast_ref::<ProtectedRuntimeStartupFailure>()
            .expect("run maps protected startup errors to the protected wrapper");
        assert_eq!(failure.to_string(), "reference service startup failed");
        assert_eq!(format!("{failure:?}"), "reference service startup failed");
        assert!(!format!("{error:?}").contains(PRIVATE));
        assert!(std::ptr::addr_eq(
            failure.source().unwrap(),
            failure.startup()
        ));
        match failure.startup() {
            StartupError::Failed(report) => report,
            StartupError::Coordinator(_) => panic!("expected a retained startup report"),
        }
    }

    fn assert_one_pool_cleanup(report: &ProtectedReport) {
        assert!(report.destruction_panic.is_none());
        assert!(report.cleanup.skipped.is_empty());
        assert_eq!(report.cleanup.records.len(), 1);
        assert_eq!(report.cleanup.records[0].name, "postgres.pool");
        assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    }

    #[tokio::test]
    async fn protected_application_failure_retains_cause_and_cleanup() {
        let supervisor = Supervisor::new(shutdown_budget());
        let mut starting = Startup::scoped(supervisor, context(), cleanup_budget(), |scope| {
            Box::pin(async move {
                scope.stage("postgres.acquire").map_err(initialization)?;
                scope
                    .reserve_cleanup("postgres.pool")
                    .map_err(initialization)?
                    .register(|| async { Ok(()) });
                Err(initialization(std::io::Error::other(PRIVATE)))
            })
        })
        .start();
        let Err(error) = starting.wait().await else {
            panic!("startup must fail")
        };
        let error = startup_failure(error);
        let report = protected(&error);
        assert_eq!(report.stage, "postgres.acquire");
        let StartupCause::Failed(InitializationError::Application(cause)) = &report.cause else {
            panic!("expected the application initialization failure")
        };
        assert_eq!(cause.source().unwrap().to_string(), PRIVATE);
        assert_one_pool_cleanup(report);
    }

    #[tokio::test]
    async fn protected_signal_policy_failure_is_not_an_application_error() {
        let mut supervisor = Supervisor::new(shutdown_budget());
        supervisor
            .on_cleanup("postgres.pool", || async { Ok(()) })
            .unwrap();
        let invoked = Arc::new(AtomicBool::new(false));
        let flag = invoked.clone();
        // Repeated selection fails before any listener is installed.
        let mut starting = Startup::scoped(supervisor, context(), cleanup_budget(), move |_| {
            Box::pin(async move {
                flag.store(true, Ordering::SeqCst);
                Ok::<_, InitializationFailure>(())
            })
        })
        .with_unix_signals("signals")
        .with_unix_signals("signals")
        .start();
        let Err(error) = starting.wait().await else {
            panic!("repeated signal policy must fail")
        };
        let error = startup_failure(error);
        let report = protected(&error);
        assert_eq!(report.stage, "startup");
        assert!(matches!(
            report.cause,
            StartupCause::Failed(InitializationError::SignalPolicyAlreadySelected)
        ));
        assert!(!invoked.load(Ordering::SeqCst));
        assert_one_pool_cleanup(report);
    }

    #[tokio::test]
    async fn protected_drain_has_no_fabricated_application_cause() {
        let supervisor = Supervisor::new(shutdown_budget());
        let handle = supervisor.handle();
        let (held, holding) = tokio::sync::oneshot::channel();
        let mut starting = Startup::scoped(supervisor, context(), cleanup_budget(), |scope| {
            Box::pin(async move {
                scope.stage("postgres.acquire").map_err(initialization)?;
                scope
                    .reserve_cleanup("postgres.pool")
                    .map_err(initialization)?
                    .register(|| async { Ok(()) });
                let _ = held.send(());
                std::future::pending::<()>().await;
                Ok(())
            })
        })
        .start();
        holding.await.unwrap();
        handle.request();
        let Err(error) = starting.wait().await else {
            panic!("requested drain must fail startup")
        };
        let error = startup_failure(error);
        let report = protected(&error);
        assert_eq!(report.stage, "postgres.acquire");
        assert!(matches!(report.cause, StartupCause::Draining));
        assert!(report.source().is_none());
        assert_one_pool_cleanup(report);
        assert_ne!(handle.status().readiness(), Readiness::Ready);
    }

    async fn checked_success(cleanup: Option<&'static str>) -> ShutdownSuccess {
        let mut supervisor = Supervisor::new(shutdown_budget());
        supervisor
            .register("component", |startup| async move {
                let shutdown = startup.shutdown().clone();
                shutdown.draining().await;
                Ok(startup.abandon())
            })
            .unwrap();
        if let Some(name) = cleanup {
            supervisor.on_cleanup(name, || async { Ok(()) }).unwrap();
        }
        supervisor.start().shutdown_checked().await.unwrap()
    }

    #[tokio::test]
    async fn running_success_requires_the_application_pool_cleanup_record() {
        check_application_shutdown(checked_success(Some("postgres.pool")).await).unwrap();
        for cleanup in [None, Some("different.resource")] {
            let error = check_application_shutdown(checked_success(cleanup).await).unwrap_err();
            let failure = error
                .downcast_ref::<RuntimePoolCleanupFailure>()
                .expect("the application invariant has a public typed failure");
            assert_eq!(
                failure.to_string(),
                "reference service shutdown did not retain its postgres.pool cleanup record"
            );
            assert!(std::ptr::addr_eq(
                failure.source().unwrap(),
                failure.report()
            ));
        }
    }

    #[tokio::test]
    async fn failed_required_cleanup_remains_a_generic_shutdown_failure() {
        use batter::{cleanup::CleanupOutcome, lifecycle::ShutdownFailure};

        let mut supervisor = Supervisor::new(shutdown_budget());
        supervisor
            .register("component", |startup| async move {
                let shutdown = startup.shutdown().clone();
                shutdown.draining().await;
                Ok(startup.abandon())
            })
            .unwrap();
        supervisor
            .on_cleanup("postgres.pool", || async {
                Err(std::io::Error::other("pool-close-marker").into())
            })
            .unwrap();
        let running = supervisor.start();
        let result: Result<(), BoxError> =
            async { check_application_shutdown(running.shutdown_checked().await?) }.await;
        let error = result.unwrap_err();
        let ShutdownFailure::Report(report) = error.downcast_ref::<ShutdownFailure>().unwrap()
        else {
            panic!("failed cleanup must retain the shutdown report")
        };
        assert_eq!(report.cleanup.records[0].name, "postgres.pool");
        assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Failed);
        assert_eq!(
            report.cleanup.records[0]
                .error
                .as_ref()
                .unwrap()
                .to_string(),
            "pool-close-marker"
        );
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
