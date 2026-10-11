//! A runnable Axum composition root with probes and bounded operations.
#[path = "http_service/config.rs"]
mod config;
#[path = "http_service/logging.rs"]
mod logging;
use config::Config;
#[cfg(test)]
#[path = "http_service/configuration_tests.rs"]
mod configuration_tests;
#[cfg(test)]
#[path = "http_service/diagnostic_process.rs"]
mod diagnostic_process;
mod support;
#[cfg(test)]
#[path = "http_service/tests.rs"]
mod tests;

use axum::{
    extract::State,
    response::{IntoResponse, Response},
    routing::get,
};
use batter::axum::{
    AdmittedRequest, AssembledHttp, GuardedRouter, HttpBoundary, HttpFailure, ProbePath,
    ReadinessPolicy, RequestPolicy, ResponseConstructionBudget, render_infrastructure_failure,
};
use batter::{
    BoxError,
    admission::{Admission, AdmissionError, Bulkhead, BulkheadCapacity},
    health::{HealthMonitor, HealthPolicy, HealthReader},
    lifecycle::Supervisor,
    operation::OperationError,
    registration::Registration,
    startup::{ApplicationError, Startup},
};
use std::{convert::Infallible, time::Duration};

async fn fail(admitted: AdmittedRequest) -> Response {
    render_infrastructure_failure(HttpFailure::Internal, Some(admitted.correlation_id()))
}

fn register_dependency_health(
    mut registration: Registration<'_>,
) -> Result<HealthReader<std::io::Error>, BoxError> {
    let policy = HealthPolicy::new(
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_secs(4),
        Duration::from_secs(1),
    )?;
    let health = HealthMonitor::new(policy, || async {
        // Demonstration only, like the read in /work. Replace the whole future
        // with a native dependency probe, including connection acquisition.
        tokio::time::sleep(Duration::from_millis(10)).await;
        Ok::<_, std::io::Error>(())
    })
    .register_in(&mut registration, "dependency.health")?;
    Ok(health)
}

#[derive(Clone)]
struct AppState {
    outbound: Bulkhead,
}

/// The admitted request carries the boundary's operation context, the generated
/// request ID and the admission policy's interruption envelope.
async fn work(State(state): State<AppState>, admitted: AdmittedRequest) -> Response {
    let id = admitted.correlation_id();
    let _permit = match state
        .outbound
        .enter(admitted.context(), Admission::Reject)
        .await
    {
        Ok(permit) => permit,
        Err(AdmissionError::Overloaded) => {
            return render_infrastructure_failure(HttpFailure::Overloaded, Some(id));
        }
        Err(AdmissionError::Closed) => {
            return render_infrastructure_failure(HttpFailure::Unavailable, Some(id));
        }
        Err(AdmissionError::Interrupted(reason)) => {
            return admitted.interruption_responder().render(reason);
        }
    };
    // Demonstration only: replace this read with a real dependency call.
    match admitted
        .context()
        .run("demo.read", |_scope| async {
            tokio::time::sleep(Duration::from_millis(25)).await;
            Ok::<_, Infallible>("ok\n")
        })
        .await
    {
        Ok(body) => body.into_response(),
        Err(OperationError::Interrupted(reason)) => {
            admitted.interruption_responder().render(reason)
        }
        Err(OperationError::Failed(never)) => match never {},
    }
}

async fn router(
    lifecycle: batter::lifecycle::LifecycleStatus,
    admission: batter::lifecycle::OperationAdmission,
    request_budget: ResponseConstructionBudget,
    dependency: HealthReader<std::io::Error>,
    bulkhead_capacity: BulkheadCapacity,
) -> Result<AssembledHttp, batter::axum::BoundaryAssemblyError> {
    let policy = RequestPolicy::new(admission, request_budget).with_infrastructure_json();
    let application = GuardedRouter::new()
        .route("/work", get(work))
        .route("/fail", get(fail))
        .with_state(AppState {
            outbound: Bulkhead::new(bulkhead_capacity),
        });
    // The boundary owns the layer order: probes outside admission, one
    // observer with correlation outermost, admission around every guarded route.
    // Guarded handlers extract the request that admission recorded.
    HttpBoundary::new(policy)
        .with_liveness(ProbePath::new("/live").expect("static liveness path is valid"))
        .expect("liveness path is unique")
        .with_readiness(
            ProbePath::new("/ready").expect("static readiness path is valid"),
            ReadinessPolicy::new(lifecycle, dependency),
        )
        .expect("readiness path is unique")
        .assemble(application)
        .await
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    report_exit(run().await)
}

fn report_exit(result: Result<(), BoxError>) -> std::process::ExitCode {
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if let Some(error) = error.downcast_ref::<batter::settings::SettingsError>() {
                eprintln!("Error: configuration failed: {error}");
            } else if let Some(error) = error.downcast_ref::<logging::LogConfigurationError>() {
                eprintln!("Error: configuration failed: {error}");
            } else {
                eprintln!("Error: process failed");
            }
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), BoxError> {
    let config = Config::load()?;
    tracing_subscriber::fmt()
        .with_env_filter(logging::filter(
            config
                .log_filter
                .clone()
                .ok_or(std::env::VarError::NotPresent),
        )?)
        .try_init()?;
    let supervisor = Supervisor::new(support::shutdown_budget());
    let lifecycle = supervisor.status();
    let admission = supervisor.operation_admission();
    // The initializer borrows the protected scope across awaits; `?` combines
    // unrelated errors into `ApplicationError` and the bound address is returned.
    let mut starting = Startup::protected(
        supervisor,
        batter::operation::OperationOwner::new(Duration::from_secs(15))?.into_context(),
        support::cleanup_budget(),
        async move |scope| {
            scope.stage("dependency.health")?;
            let health = register_dependency_health(scope.registration())?;
            scope.stage("http.bind")?;
            let application = router(
                lifecycle,
                admission,
                config.request_budget,
                health,
                config.bulkhead_capacity,
            )
            .await?;
            let listener = tokio::net::TcpListener::bind(config.bind).await?;
            let address = listener.local_addr()?;
            application.register_in(scope, "http", listener)?;
            Ok::<_, ApplicationError>(address)
        },
    )
    .with_unix_signals("signals")
    .start();
    let (running, address) = starting.wait_with_output().await?;
    tracing::info!(%address, "HTTP listener bound");
    running.wait_checked().await?;
    Ok(())
}

#[test]
fn child_fixture() {
    if diagnostic_process::launch::scenario().is_some() {
        assert_eq!(main(), std::process::ExitCode::FAILURE);
        std::process::exit(42);
    }
}
