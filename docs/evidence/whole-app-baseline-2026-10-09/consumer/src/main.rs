//! records-service: a records API whose inserts enqueue a durable notification
//! job in the same transaction, served and supervised by Batter.

mod database;
mod http;
mod jobs;
mod records;
mod settings;

use batter::BoxError;
use batter::axum::ReadinessPolicy;
use batter::cleanup::CleanupBudget;
use batter::lifecycle::{ShutdownBudget, ShutdownFailure, Supervisor};
use batter::operation::OperationOwner;
use batter::runledger::native::runtime::Supervisor as NativeSupervisor;
use batter::settings::SettingsError;
use batter::startup::{InitializationError, Startup, StartupError};
use records::RecordsStore;
use settings::Settings;
use std::{process::ExitCode, time::Duration};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Print only fixed text: retained causes may contain connection
            // details and belong to a trusted sink, not stderr.
            if let Some(error) = error.downcast_ref::<SettingsError>() {
                eprintln!("Error: configuration failed: {error}");
            } else if error.downcast_ref::<ShutdownFailure>().is_some() {
                eprintln!("Error: shutdown did not complete cleanly");
            } else if error
                .downcast_ref::<StartupError<InitializationError<InitializationFailure>>>()
                .is_some()
            {
                eprintln!("Error: startup failed; registered cleanup was driven");
            } else {
                eprintln!("Error: process failed");
            }
            ExitCode::FAILURE
        }
    }
}

/// Application-side initialization failure with fixed formatting and a
/// retained source chain.
struct InitializationFailure(BoxError);

impl std::fmt::Display for InitializationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("initialization failed")
    }
}

impl std::fmt::Debug for InitializationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for InitializationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

fn cleanup_budget() -> Result<CleanupBudget, BoxError> {
    Ok(CleanupBudget::new(
        Duration::from_secs(5),
        Duration::from_secs(3),
        Duration::from_secs(1),
    )?)
}

fn shutdown_budget() -> Result<ShutdownBudget, BoxError> {
    Ok(ShutdownBudget::new(
        Duration::from_secs(10),
        Duration::from_secs(2),
        Duration::from_secs(1),
        cleanup_budget()?,
    )?)
}

async fn run() -> Result<(), BoxError> {
    let settings = Settings::load()?;
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_new(
            settings.log_filter.as_deref().unwrap_or("info"),
        )?)
        .try_init()?;

    let supervisor = Supervisor::new(shutdown_budget()?);
    let lifecycle = supervisor.status();
    let admission = supervisor.operation_admission();

    // One budget bounds initialization; the initializer borrows a clone of
    // the same context for its own bounded startup queries.
    let startup = OperationOwner::new(Duration::from_secs(60))?;
    let initialization = startup.context().clone();

    let mut starting = Startup::scoped(
        supervisor,
        startup.into_context(),
        cleanup_budget()?,
        move |scope| {
            Box::pin(async move {
                let result: Result<(), BoxError> = async {
                    scope.stage("postgres.pool")?;
                    let database = database::open(scope, &settings.database)?;

                    scope.stage("postgres.schema")?;
                    database::prepare_schema(&initialization, &database).await?;

                    let store = RecordsStore::new(database.clone(), Duration::from_secs(2));

                    scope.stage("jobs.catalog")?;
                    let catalog = jobs::catalog(store.clone());
                    initialization
                        .run("jobs.sync_definitions", |_| async {
                            catalog.sync_definitions(database.pool()).await
                        })
                        .await?;

                    scope.stage("postgres.health")?;
                    let health = database::register_health(scope, &database)?;

                    scope.stage("jobs.worker")?;
                    let prepared =
                        NativeSupervisor::builder(database.pool(), settings.jobs.clone())?
                            .with_catalog(&catalog)
                            .prepare()?;
                    batter::runledger::register_in(
                        scope,
                        "runledger.worker",
                        OperationOwner::new(Duration::from_secs(10))?.into_context(),
                        prepared,
                    )?;

                    scope.stage("http.bind")?;
                    let application = http::assemble(
                        admission.clone(),
                        settings.request_budget,
                        ReadinessPolicy::new(lifecycle.clone(), health),
                        http::AppState { records: store },
                    )
                    .await?;
                    let listener = tokio::net::TcpListener::bind(settings.bind).await?;
                    tracing::info!(address = %listener.local_addr()?, "HTTP listener bound");
                    application.register_in(scope, "http", listener)?;
                    Ok(())
                }
                .await;
                result.map_err(InitializationFailure)
            })
        },
    )
    .with_unix_signals("signals")
    .start();

    let running = starting.wait().await?;
    running.wait_checked().await?;
    Ok(())
}
