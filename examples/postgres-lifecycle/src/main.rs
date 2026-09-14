//! Native SQLx pool lifecycle and error-preserving partial-startup cleanup.
//! No Runledger schema is created here. No business transaction is retried.
mod support;
#[cfg(test)]
mod tests;

use batter::{
    BoxError,
    lifecycle::{Supervisor, check_shutdown},
    operation::OperationContext,
    startup::{ProtectedStartupScope, ScopedStartup, Startup, StartupFuture},
};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
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

fn process_result(result: Result<(), BoxError>) -> Result<(), ProcessFailure> {
    result.map_err(|cause| ProcessFailure { cause })
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
    let supervisor = Supervisor::new(support::shutdown_budget());
    // The root owns the complete startup allowance; the probe derives a child.
    let context = OperationContext::new(Duration::from_secs(15))?;
    let probe_parent = context.clone();
    let startup = Startup::scoped(
        supervisor,
        context,
        support::cleanup_budget(),
        move |scope| {
            Box::pin(async move {
                let result: Result<(), BoxError> = async {
                    scope.stage("postgres.acquire")?;
                    // Parse the application-owned endpoint before native construction.
                    let connection: PgConnectOptions = database_url.parse()?;
                    let pool = database_pool(scope, connection)?;
                    scope.stage("postgres.probe")?;
                    // The pool is lazy: this bounded probe establishes connectivity.
                    let probe = probe_parent.child(Duration::from_secs(5))?;
                    batter_sqlx::probe(&pool, &probe).await?;
                    scope
                        .registration()
                        .register("application", |shutdown| async move {
                            shutdown.mark_started();
                            shutdown.draining().await;
                            Ok(())
                        })?;
                    Ok(())
                }
                .await;
                process_result(result)
            })
        },
    )
    .with_unix_signals("signals");
    serve(startup).await
}

/// Reserve before construction, so a rejected name cannot strand a pool; the
/// adapter publishes native close before returning the lazy pool.
fn database_pool(
    scope: &mut ProtectedStartupScope,
    connection: PgConnectOptions,
) -> Result<PgPool, BoxError> {
    let slot = scope.reserve_cleanup("postgres.pool")?;
    let options = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(3));
    Ok(batter_sqlx::pool_in(slot, options, connection))
}

async fn serve<F>(startup: ScopedStartup<F>) -> Result<(), BoxError>
where
    F: for<'a> FnOnce(&'a mut ProtectedStartupScope) -> StartupFuture<'a, ProcessFailure>
        + Send
        + 'static,
{
    let mut starting = startup.start();
    let running = starting.wait().await?;
    if running.handle().wait_ready().await.is_ok() {
        tracing::info!("PostgreSQL lifecycle ready");
    }
    check_shutdown(running.wait().await)?;
    Ok(())
}
