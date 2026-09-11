//! Owned production startup for the staged command and worker reference service.

use crate::{
    config::RootSettings,
    http::router,
    schema::initialize_schema,
    worker::{
        DependencyCleanup, DependencyCleanupError, TerminationGate, WORKER_SHUTDOWN_ALLOWANCE,
        WorkerSettlement, prepare_probe_worker,
    },
};
use batter::{
    BoxError,
    cleanup::{CleanupBudget, CleanupReport, CleanupStack},
    lifecycle::{DriverOutcome, RunningSupervisor, ShutdownBudget, install_signals},
    operation::OperationContext,
    startup::{Startup, StartupError},
};
use std::{
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};

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

fn dependency_cleanup_budget() -> CleanupBudget {
    let second = Duration::from_secs(1);
    CleanupBudget::new(Duration::from_secs(5), second, second)
        .expect("static cleanup budget is valid")
}

fn cleanup_budget() -> CleanupBudget {
    let second = Duration::from_secs(1);
    let total = WORKER_SHUTDOWN_ALLOWANCE + dependency_cleanup_budget().total_allowance() + second;
    CleanupBudget::new(total, total, second).expect("static cleanup budget is valid")
}

fn shutdown_budget() -> ShutdownBudget {
    let second = Duration::from_secs(1);
    ShutdownBudget::new(
        WORKER_SHUTDOWN_ALLOWANCE,
        Duration::from_secs(3),
        second,
        cleanup_budget(),
    )
    .expect("static shutdown budget is valid")
}

/// Unsuccessful process shutdown with the separately retained dependency result.
///
/// The process report retains every direct component error, including the
/// original Runledger error. `dependent_cleanup` remains available even when
/// Batter skipped the grouped outer hook after aborting a wrapper.
pub struct RuntimeShutdownFailure {
    process: DriverOutcome,
    dependent_cleanup: Arc<CleanupReport>,
    worker: WorkerSettlement,
}

impl RuntimeShutdownFailure {
    /// Borrow the complete Batter driver outcome.
    pub fn process(&self) -> &DriverOutcome {
        &self.process
    }

    /// Inspect independently retained worker failures and release observations.
    pub fn worker(&self) -> &WorkerSettlement {
        &self.worker
    }

    /// Inspect the attempted or explicitly skipped worker dependency finalizers.
    pub fn dependent_cleanup(&self) -> &CleanupReport {
        &self.dependent_cleanup
    }
}

impl fmt::Display for RuntimeShutdownFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("reference service shutdown failed")
    }
}

impl fmt::Debug for RuntimeShutdownFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for RuntimeShutdownFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.process {
            Ok(report) if report.is_success() => Some(&*self.dependent_cleanup),
            Ok(report) => Some(&**report),
            Err(error) => Some(&**error),
        }
    }
}

/// Failed startup together with late dependency cleanup and worker settlement.
///
/// These outcomes survive an outer cleanup-hook timeout or cancelled preparation
/// waiter. Automatic formatting does not expose native error contents.
pub struct RuntimeStartupFailure {
    startup: StartupError<InitializationFailure>,
    dependent_cleanup: Option<Arc<CleanupReport>>,
    worker: WorkerSettlement,
}

impl RuntimeStartupFailure {
    /// Inspect the original startup cause and its outer cleanup report.
    pub fn startup(&self) -> &StartupError<InitializationFailure> {
        &self.startup
    }
    /// Inspect eventual nested cleanup, including after an outer hook timeout.
    pub fn dependent_cleanup(&self) -> Option<&CleanupReport> {
        self.dependent_cleanup.as_deref()
    }
    /// Inspect preparation, native-driver and lease-release outcomes.
    pub fn worker(&self) -> &WorkerSettlement {
        &self.worker
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
        Some(&self.startup)
    }
}

/// Run the staged production root until a registered Unix signal drains it.
///
/// Startup uses the configured supervisor/pool/listener/authentication/request
/// constructors, applies both migration histories, and starts a probe-only
/// Runledger registry. The actual startup control handler must execute and reach
/// durable success before the worker component can acknowledge startup.
///
/// This root deliberately withholds application readiness: the ordinary
/// delivery handler is not registered until `batter-8q8.2`. Delivery jobs remain
/// pending and `/ready` remains unavailable even though the control witness ran.
pub async fn run(settings: RootSettings) -> Result<(), BoxError> {
    let supervisor = settings.supervisor(shutdown_budget())?;
    let handle = supervisor.handle();
    let readiness = handle.clone();
    let cleanup = cleanup_budget();
    let startup_context = OperationContext::new(Duration::from_secs(35))?;
    let initializer_context = startup_context.clone();
    let termination = TerminationGate::new();
    let retained_cleanup = Arc::new(Mutex::new(None::<Arc<DependencyCleanup>>));
    let initialized_cleanup = retained_cleanup.clone();
    let worker_termination = termination.clone();
    let mut starting = Startup::new(
        supervisor,
        // Work is clamped before the shared end-to-end worker stop reserve.
        startup_context,
        cleanup,
        move |scope| {
            Box::pin(async move {
                scope.stage("signals.install").map_err(initialization)?;
                let mut signals =
                    install_signals(scope.supervisor(), "signals").map_err(initialization)?;
                let initialize = async {
                    let result: Result<(), BoxError> = async {
                        scope.stage("postgres.acquire")?;
                        let pool = register_dependencies(
                            scope,
                            &settings,
                            worker_termination.clone(),
                            &initialized_cleanup,
                        )?;
                        drop(pool.acquire().await?);
                        scope.stage("postgres.schema")?;
                        initialize_schema(&pool).await?;

                        scope.stage("http.bind")?;
                        let application = router(&settings, readiness.clone(), pool.clone())?;
                        let listener = tokio::net::TcpListener::bind(settings.bind()).await?;
                        batter_axum::register_http(
                            scope.supervisor(),
                            "http",
                            listener,
                            application,
                        )?;

                        scope.stage("worker.witness")?;
                        let worker = prepare_probe_worker(
                            &pool,
                            settings.worker(),
                            worker_termination.clone(),
                            &initializer_context,
                            Duration::from_secs(20),
                        )
                        .await?;
                        worker.register(scope.supervisor())?;
                        Ok(())
                    }
                    .await;
                    result.map_err(initialization)
                };
                tokio::select! {
                    biased;
                    _ = signals.received() => {
                        readiness.request();
                        return Ok(());
                    }
                    result = initialize => result?,
                }
                signals
                    .register(scope.supervisor())
                    .map_err(initialization)?;
                Ok(())
            })
        },
    )
    .without_readiness_approval()
    .start();
    let running = match starting.wait().await {
        Ok(running) => running,
        Err(error) => {
            return Err(finish_startup_failure(error, &retained_cleanup, &termination).await);
        }
    };
    finish_running(running, &retained_cleanup, &termination).await
}

async fn finish_startup_failure(
    startup: StartupError<InitializationFailure>,
    retained: &Mutex<Option<Arc<DependencyCleanup>>>,
    termination: &TerminationGate,
) -> BoxError {
    let dependent_cleanup = match retained_dependency_cleanup(retained) {
        Some(cleanup) => Some(cleanup.finish().await),
        None => None,
    };
    Box::new(RuntimeStartupFailure {
        startup,
        dependent_cleanup,
        worker: termination.settlement(),
    })
}

fn register_dependencies(
    scope: &mut batter::startup::StartupScope,
    settings: &RootSettings,
    termination: TerminationGate,
    retained: &Mutex<Option<Arc<DependencyCleanup>>>,
) -> Result<sqlx::PgPool, BoxError> {
    let grouped_cleanup = scope.supervisor().reserve_cleanup("worker.dependencies")?;
    let mut dependency_stack = CleanupStack::new();
    let pool_cleanup = dependency_stack.reserve("postgres.pool")?;
    let options = settings.connect_options_from_process()?;
    let pool = settings.pool_options().connect_lazy_with(options);
    let closing = pool.clone();
    pool_cleanup.register(move || async move {
        closing.close().await;
        Ok::<_, BoxError>(())
    });
    let dependencies = Arc::new(DependencyCleanup::new(
        dependency_stack,
        dependency_cleanup_budget(),
        termination,
    ));
    *retained.lock().unwrap_or_else(|error| error.into_inner()) = Some(dependencies.clone());
    grouped_cleanup.register(move || async move {
        let report = dependencies.finish().await;
        if report.is_success() {
            Ok(())
        } else {
            Err(Box::new(DependencyCleanupError(report)) as BoxError)
        }
    });

    Ok(pool)
}

async fn finish_running(
    running: RunningSupervisor,
    retained_cleanup: &Mutex<Option<Arc<DependencyCleanup>>>,
    termination: &TerminationGate,
) -> Result<(), BoxError> {
    let process = running.wait().await;
    let dependent_cleanup = retained_dependency_cleanup(retained_cleanup)
        .expect("successful startup installs the dependency cleanup owner")
        .finish_after_process(&process)
        .await;
    let process_succeeded = matches!(&process, Ok(report) if report.is_success());
    let worker = termination.settlement();
    if process_succeeded && dependent_cleanup.is_success() && worker.is_success() {
        Ok(())
    } else {
        Err(Box::new(RuntimeShutdownFailure {
            process,
            dependent_cleanup,
            worker,
        }))
    }
}

fn retained_dependency_cleanup(
    retained: &Mutex<Option<Arc<DependencyCleanup>>>,
) -> Option<Arc<DependencyCleanup>> {
    retained
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn startup_retains_cleanup_failure_after_outer_hook_timeout() {
        let mut stack = CleanupStack::new();
        stack
            .push("late-dependency", || async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Err(Box::new(std::io::Error::other("late-cleanup-marker")) as BoxError)
            })
            .unwrap();
        let gate = TerminationGate::new();
        let cleanup = Arc::new(DependencyCleanup::new(
            stack,
            dependency_cleanup_budget(),
            gate.clone(),
        ));
        let retained = Mutex::new(Some(cleanup.clone()));
        let supervisor = batter::lifecycle::Supervisor::new(shutdown_budget());
        let short = Duration::from_millis(50);
        let mut starting = Startup::new(
            supervisor,
            OperationContext::new(Duration::from_secs(1)).unwrap(),
            CleanupBudget::new(short, short, short).unwrap(),
            move |scope| {
                Box::pin(async move {
                    cleanup.register(scope.supervisor()).unwrap();
                    Err::<(), _>(initialization(std::io::Error::other("startup-marker")))
                })
            },
        )
        .start();
        let error = starting.wait().await.err().expect("initializer failed");
        let error = finish_startup_failure(error, &retained, &gate).await;
        let failure = error.downcast_ref::<RuntimeStartupFailure>().unwrap();
        let StartupError::Failed(startup) = failure.startup() else {
            panic!("missing startup report")
        };
        assert_eq!(
            startup.cleanup.records[0].outcome,
            batter::cleanup::CleanupOutcome::TimedOut
        );
        assert!(
            matches!(&startup.cause, batter::startup::StartupCause::Failed(error) if error.0.to_string() == "startup-marker")
        );
        let late = failure.dependent_cleanup().unwrap();
        assert_eq!(
            late.records[0].outcome,
            batter::cleanup::CleanupOutcome::Failed
        );
        assert_eq!(
            late.records[0].error.as_ref().unwrap().to_string(),
            "late-cleanup-marker"
        );
        assert!(!format!("{failure:?}").contains("marker"));
    }

    #[test]
    fn static_budgets_preserve_cleanup_inside_total_shutdown() {
        let _dependency_cleanup = dependency_cleanup_budget();
        let _cleanup = cleanup_budget();
        let _shutdown = shutdown_budget();
        assert!(shutdown_budget().total_allowance() >= Duration::from_secs(30));
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
