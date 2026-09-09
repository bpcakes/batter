//! A runnable Axum composition root with probes and bounded operations.
#[path = "http_service/logging.rs"]
mod logging;
mod support;
#[cfg(test)]
#[path = "http_service/tests.rs"]
mod tests;

use axum::{
    Extension, Json, Router,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use batter::{
    BoxError,
    admission::{Admission, AdmissionError, Bulkhead},
    lifecycle::{Readiness, ShutdownHandle, Supervisor},
    operation::{Interruption, OperationContext, OperationError},
};
use batter_axum::{
    HttpFailure, HttpObservationLevel, RequestPolicy, liveness, observe_http, request_admission,
};
use serde::Serialize;
use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tracing::Instrument;

// Application-owned, process-local correlation. Incoming IDs carry no trust.
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct RequestId(String);

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'static str,
    message: &'static str,
    request_id: &'a str,
}

fn render_failure(failure: HttpFailure, id: &RequestId) -> Response {
    let message = match failure {
        HttpFailure::Unavailable => "The service is unavailable",
        HttpFailure::Cancelled => "The operation was cancelled",
        HttpFailure::DeadlineExceeded => "The operation exceeded its time budget",
        HttpFailure::Overloaded => "The service is at capacity",
        HttpFailure::Internal => "An internal error occurred",
    };
    (
        failure.status(),
        [(header::CACHE_CONTROL, "no-store")],
        Json(ErrorBody {
            code: failure.code(),
            message,
            request_id: &id.0,
        }),
    )
        .into_response()
}

async fn request_identity(mut request: axum::extract::Request, next: Next) -> Response {
    batter::telemetry::with_current_dispatch(async move {
        let id = RequestId(format!(
            "example-{}",
            NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let span = tracing::info_span!("request", request_id = %id.0);
        request.extensions_mut().insert(id.clone());
        let mut response = next.run(request).instrument(span).await;
        response.headers_mut().insert(
            "x-request-id",
            HeaderValue::from_str(&id.0).expect("generated ASCII request ID"),
        );
        response
    })
    .await
}

async fn fail(Extension(id): Extension<RequestId>) -> Response {
    render_failure(HttpFailure::Internal, &id)
}

async fn readiness_response(State(handle): State<ShutdownHandle>) -> Response {
    // This application's probe reports lifecycle state. Task failures still have
    // their own diagnostics; routine starting/draining probes need no warning.
    match handle.readiness() {
        Readiness::Ready => StatusCode::OK.into_response(),
        Readiness::Starting | Readiness::Draining => (
            Extension(HttpObservationLevel(tracing::Level::INFO)),
            StatusCode::SERVICE_UNAVAILABLE,
        )
            .into_response(),
        Readiness::Stopped => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

struct Config {
    bind: SocketAddr,
    request_budget: Duration,
}

impl Config {
    fn load() -> Result<Self, BoxError> {
        let bind = match std::env::var("BATTER_BIND") {
            Ok(value) => value.parse()?,
            Err(std::env::VarError::NotPresent) => "127.0.0.1:3000".parse()?,
            Err(error) => return Err(error.into()),
        };
        let milliseconds = match std::env::var("BATTER_REQUEST_TIMEOUT_MS") {
            Ok(value) => value.parse::<u64>()?,
            Err(std::env::VarError::NotPresent) => 2_000,
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            bind,
            request_budget: Duration::from_millis(milliseconds),
        })
    }
}

#[derive(Clone)]
struct AppState {
    outbound: Bulkhead,
}

async fn work(
    State(state): State<AppState>,
    Extension(context): Extension<OperationContext>,
    Extension(id): Extension<RequestId>,
) -> Response {
    let _permit = match state.outbound.enter(&context, Admission::Reject).await {
        Ok(permit) => permit,
        Err(AdmissionError::Overloaded) => return render_failure(HttpFailure::Overloaded, &id),
        Err(AdmissionError::Closed) => return render_failure(HttpFailure::Unavailable, &id),
        Err(AdmissionError::Interrupted(Interruption::Cancelled)) => {
            return render_failure(HttpFailure::Cancelled, &id);
        }
        Err(AdmissionError::Interrupted(Interruption::DeadlineExceeded)) => {
            return render_failure(HttpFailure::DeadlineExceeded, &id);
        }
    };
    // Demonstration only: replace this read with a real dependency call.
    match context
        .run("demo.read", |_scope| async {
            tokio::time::sleep(Duration::from_millis(25)).await;
            Ok::<_, Infallible>("ok\n")
        })
        .await
    {
        Ok(body) => body.into_response(),
        Err(OperationError::Interrupted(Interruption::Cancelled)) => {
            render_failure(HttpFailure::Cancelled, &id)
        }
        Err(OperationError::Interrupted(Interruption::DeadlineExceeded)) => {
            render_failure(HttpFailure::DeadlineExceeded, &id)
        }
        Err(OperationError::Failed(never)) => match never {},
    }
}

fn router(
    handle: batter::lifecycle::ShutdownHandle,
    request_budget: Duration,
) -> Result<Router, batter::ConfigurationError> {
    let policy = RequestPolicy::new(handle.clone(), request_budget)?.with_failure_renderer(
        |failure, parts| {
            // request_identity is outside this layer and supplies trusted data.
            let id = parts
                .extensions
                .get::<RequestId>()
                .expect("request identity middleware is installed");
            render_failure(failure, id)
        },
    );
    let application = Router::new()
        .route("/work", get(work))
        .route("/fail", get(fail))
        .with_state(AppState {
            outbound: Bulkhead::new(32)?,
        })
        .route_layer(middleware::from_fn_with_state(policy, request_admission));
    // Health endpoints must remain outside the admission gate.
    let probes = Router::new()
        .route("/live", get(liveness))
        .route("/ready", get(readiness_response))
        .with_state(handle.clone());
    Ok(application
        .merge(probes)
        .layer(middleware::from_fn(observe_http))
        .layer(middleware::from_fn(request_identity)))
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    tracing_subscriber::fmt()
        .with_env_filter(logging::filter(std::env::var("RUST_LOG"))?)
        .try_init()?;
    let config = Config::load()?;
    let mut supervisor = Supervisor::new(support::shutdown_budget());
    let handle = supervisor.handle();
    let application = router(handle.clone(), config.request_budget)?;
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(address = %listener.local_addr()?, "HTTP listener bound");
    supervisor.register("http", move |shutdown| async move {
        shutdown.mark_started();
        axum::serve(listener, application)
            .with_graceful_shutdown(async move { shutdown.draining().await })
            .await?;
        Ok(())
    })?;
    support::register_signals(&mut supervisor)?;
    handle.mark_ready();
    let running = supervisor.start();
    let report = running.wait().await?;
    if !report.is_success() {
        return Err(std::io::Error::other((*report).to_string()).into());
    }
    Ok(())
}
