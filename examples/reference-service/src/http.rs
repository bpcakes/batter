//! Authenticated Axum routes for delivery submission and reconciliation.

use crate::{
    auth::{AuthenticationError, BearerAuthenticator},
    config::RootSettings,
    delivery::{
        Delivery, DeliveryService, OwnerId, QueryError, SubmitDelivery, SubmitDisposition,
        SubmitError, SubmitRejection, SubmitResult, UncertainSubmission,
    },
};
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use batter::{
    admission::{Admission, AdmissionError, Bulkhead},
    lifecycle::ShutdownHandle,
    operation::{Interruption, OperationContext},
};
use batter_axum::{
    HttpFailure, liveness, operational_http, render_infrastructure_failure, request_admission,
};
use serde::Serialize;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

const REQUEST_BODY_MAX_BYTES: usize = crate::delivery::PAYLOAD_MAX_BYTES + 2 * 1024;
const RESPONSE_RESERVE_MAX: Duration = Duration::from_millis(25);

#[derive(Clone)]
struct AppState {
    deliveries: DeliveryService,
    database: Bulkhead,
}

/// Failure while composing the production router from validated settings.
#[derive(Debug, thiserror::Error)]
pub enum RouterBuildError {
    /// A validated native constructor rejected its input.
    #[error("router policy construction failed")]
    Configuration(#[from] batter::ConfigurationError),
    /// Serving authentication was not configured.
    #[error("router authentication construction failed")]
    Authentication(#[from] batter::settings::SettingsError),
}

/// Build the production command router from the application pool and settings.
///
/// Business routes require configured bearer authentication, then lifecycle
/// admission/deadline, then process-local database admission. Health endpoints
/// stay outside those gates. The outer operational middleware supplies only
/// server-generated diagnostics; it cannot select `OwnerId`.
/// Production composition with fresh dependency health in addition to native
/// initialization acknowledgement and explicit application readiness approval.
pub fn router<E: Send + Sync + 'static>(
    settings: &RootSettings,
    handle: ShutdownHandle,
    pool: PgPool,
    health: batter::health::HealthReader<E>,
) -> Result<Router, RouterBuildError> {
    let probes = Router::new()
        .route("/live", get(liveness))
        .route("/ready", get(batter_axum::dependency_readiness::<E>))
        .with_state(batter_axum::ReadinessPolicy::new(handle.clone(), health));
    router_with_probes(settings, handle, pool, probes)
}

fn router_with_probes(
    settings: &RootSettings,
    handle: ShutdownHandle,
    pool: PgPool,
    probes: Router,
) -> Result<Router, RouterBuildError> {
    let authenticator = settings.authenticator()?;
    let policy = settings
        .request_policy(handle.clone())?
        .with_infrastructure_json();
    let state = AppState {
        deliveries: DeliveryService::new(pool),
        database: settings.bulkhead()?,
    };
    let business = Router::new()
        .route("/records/{record_id}/deliveries", post(submit_delivery))
        .route("/deliveries/{delivery_id}", get(get_delivery))
        .route(
            "/delivery-commands/{idempotency_key}",
            get(get_delivery_by_key),
        )
        .with_state(state)
        .layer(DefaultBodyLimit::max(REQUEST_BODY_MAX_BYTES))
        .route_layer(middleware::from_fn_with_state(policy, request_admission))
        .route_layer(middleware::from_fn_with_state(authenticator, authenticate));
    Ok(business
        .merge(probes)
        .fallback(|| async { StatusCode::NOT_FOUND })
        .layer(middleware::from_fn(operational_http)))
}

async fn authenticate(
    State(authenticator): State<BearerAuthenticator>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    match authenticator.authenticate(request.headers().get(header::AUTHORIZATION)) {
        Ok(owner) => {
            request.extensions_mut().insert(owner);
            next.run(request).await
        }
        Err(error) => authentication_failure(error),
    }
}

#[derive(Serialize)]
struct Problem {
    code: &'static str,
    message: &'static str,
}

fn problem(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "application/problem+json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Json(Problem { code, message }),
    )
        .into_response()
}

fn authentication_failure(_error: AuthenticationError) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [
            (header::WWW_AUTHENTICATE, "Bearer"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Json(Problem {
            code: "authentication_required",
            message: "valid authentication is required",
        }),
    )
        .into_response()
}

async fn submit_delivery(
    State(state): State<AppState>,
    Extension(owner): Extension<OwnerId>,
    Extension(context): Extension<OperationContext>,
    Path(record_id): Path<Uuid>,
    Json(request): Json<SubmitDelivery>,
) -> Response {
    let _permit = match state.database.enter(&context, Admission::Reject).await {
        Ok(permit) => permit,
        Err(error) => return admission_failure(error),
    };
    let command_context = match command_context(&context) {
        Ok(context) => context,
        Err(interruption) => {
            return render_infrastructure_failure(interruption_failure(interruption), None);
        }
    };
    let key = request.idempotency_key.clone();
    match state
        .deliveries
        .submit(&command_context, owner, record_id, request)
        .await
    {
        Ok(result) => accepted(result),
        Err(error) => submit_failure(error, &key),
    }
}

fn command_context(context: &OperationContext) -> Result<OperationContext, Interruption> {
    context.check()?;
    let remaining = context.remaining();
    let fractional = remaining / 10;
    let reserve = fractional
        .min(RESPONSE_RESERVE_MAX)
        .max(Duration::from_nanos(1));
    let work = remaining.saturating_sub(reserve);
    if work.is_zero() {
        return Err(Interruption::DeadlineExceeded);
    }
    context
        .child(work)
        .map_err(|_| Interruption::DeadlineExceeded)
}

#[derive(Serialize)]
struct SubmitResponse {
    outcome: SubmitDisposition,
    delivery: Delivery,
}

fn accepted(result: SubmitResult) -> Response {
    (
        StatusCode::ACCEPTED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(SubmitResponse {
            outcome: result.disposition,
            delivery: result.delivery,
        }),
    )
        .into_response()
}

#[derive(Serialize)]
struct UncertainResponse<'a> {
    outcome: &'static str,
    code: &'static str,
    idempotency_key: &'a str,
    reconciliation_path: String,
    message: &'static str,
}

fn submit_failure(error: SubmitError, key: &str) -> Response {
    match error {
        SubmitError::Invalid(_) => problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_delivery_command",
            "the delivery command is invalid",
        ),
        SubmitError::Rejected(SubmitRejection::RecordNotObserved) => problem(
            StatusCode::NOT_FOUND,
            "record_not_observed",
            "the target record was not observed",
        ),
        SubmitError::Rejected(SubmitRejection::StaleGeneration { .. }) => problem(
            StatusCode::CONFLICT,
            "stale_record_generation",
            "the target record generation changed",
        ),
        SubmitError::Rejected(SubmitRejection::IdempotencyConflict) => problem(
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "the idempotency key is retained with different command input",
        ),
        SubmitError::Uncertain(uncertain) => uncertain_response(&uncertain, key),
        SubmitError::Interrupted(interruption) => {
            render_infrastructure_failure(interruption_failure(interruption), None)
        }
        SubmitError::Storage(error) if error.is_pool_unavailable() => {
            render_infrastructure_failure(HttpFailure::Unavailable, None)
        }
        SubmitError::Storage(_) => render_infrastructure_failure(HttpFailure::Internal, None),
    }
}

fn uncertain_response(uncertain: &UncertainSubmission, key: &str) -> Response {
    let code = match uncertain {
        UncertainSubmission::Commit(_) => "commit_acknowledgement_lost",
        UncertainSubmission::Rollback { .. } => "rollback_acknowledgement_lost",
        UncertainSubmission::Interrupted(_) => "submission_interrupted",
    };
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CACHE_CONTROL, "no-store")],
        Json(UncertainResponse {
            outcome: "uncertain",
            code,
            idempotency_key: key,
            reconciliation_path: format!("/delivery-commands/{key}"),
            message: "query the retained key; absence while the transaction settles does not prove rollback",
        }),
    )
        .into_response()
}

async fn get_delivery(
    State(state): State<AppState>,
    Extension(owner): Extension<OwnerId>,
    Extension(context): Extension<OperationContext>,
    Path(delivery_id): Path<Uuid>,
) -> Response {
    let _permit = match state.database.enter(&context, Admission::Reject).await {
        Ok(permit) => permit,
        Err(error) => return admission_failure(error),
    };
    query_response(
        state
            .deliveries
            .get_by_id(&context, owner, delivery_id)
            .await,
        false,
    )
}

async fn get_delivery_by_key(
    State(state): State<AppState>,
    Extension(owner): Extension<OwnerId>,
    Extension(context): Extension<OperationContext>,
    Path(idempotency_key): Path<String>,
) -> Response {
    let _permit = match state.database.enter(&context, Admission::Reject).await {
        Ok(permit) => permit,
        Err(error) => return admission_failure(error),
    };
    query_response(
        state
            .deliveries
            .get_by_key(&context, owner, &idempotency_key)
            .await,
        true,
    )
}

#[derive(Serialize)]
struct DeliveryResponse {
    outcome: &'static str,
    delivery: Delivery,
}

#[derive(Serialize)]
struct NotObservedResponse {
    outcome: &'static str,
    message: &'static str,
}

fn query_response(result: Result<Option<Delivery>, QueryError>, reconciliation: bool) -> Response {
    match result {
        Ok(Some(delivery)) => (
            StatusCode::OK,
            [(header::CACHE_CONTROL, "no-store")],
            Json(DeliveryResponse {
                outcome: "observed",
                delivery,
            }),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            [(header::CACHE_CONTROL, "no-store")],
            Json(NotObservedResponse {
                outcome: "not_observed",
                message: if reconciliation {
                    "absence while an original transaction settles does not prove rollback"
                } else {
                    "the delivery was not observed for this owner"
                },
            }),
        )
            .into_response(),
        Err(QueryError::Invalid(_)) => problem(
            StatusCode::BAD_REQUEST,
            "invalid_delivery_query",
            "the delivery query is invalid",
        ),
        Err(QueryError::Interrupted(interruption)) => {
            render_infrastructure_failure(interruption_failure(interruption), None)
        }
        Err(QueryError::Storage(error)) if error.is_pool_unavailable() => {
            render_infrastructure_failure(HttpFailure::Unavailable, None)
        }
        Err(QueryError::Storage(_)) => render_infrastructure_failure(HttpFailure::Internal, None),
    }
}

fn admission_failure(error: AdmissionError) -> Response {
    let failure = match error {
        AdmissionError::Overloaded => HttpFailure::Overloaded,
        AdmissionError::Closed => HttpFailure::Unavailable,
        AdmissionError::Interrupted(interruption) => interruption_failure(interruption),
    };
    render_infrastructure_failure(failure, None)
}

const fn interruption_failure(interruption: Interruption) -> HttpFailure {
    match interruption {
        Interruption::Cancelled => HttpFailure::Cancelled,
        Interruption::DeadlineExceeded => HttpFailure::DeadlineExceeded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigMode;
    use axum::{
        body::to_bytes,
        http::{Request, header},
    };
    use batter::settings::SettingsSource;
    use serde_json::Value;
    use tower::ServiceExt;

    fn settings() -> RootSettings {
        RootSettings::from_sources(
            ConfigMode::Serve,
            None,
            SettingsSource::default(),
            SettingsSource::from_pairs([
                (
                    "DATABASE_URL".into(),
                    "postgres://user:fake@localhost/db?sslmode=disable".into(),
                ),
                ("JOBS_WORKER_ID".into(), "worker".into()),
                (
                    "BATTER_AUTH_OWNER_ID".into(),
                    "00000000-0000-0000-0000-000000000001".into(),
                ),
                ("BATTER_AUTH_TOKEN".into(), "fake-token".into()),
            ])
            .unwrap(),
        )
        .unwrap()
    }

    fn app() -> Router {
        let settings = settings();
        let handle = ShutdownHandle::new();
        handle.mark_ready();
        let pool = settings
            .pool_options()
            .connect_lazy_with(settings.connect_options_from_process().unwrap());
        // Isolated business-route tests deliberately have no published health.
        let second = Duration::from_secs(1);
        let monitor = batter::health::HealthMonitor::new(
            batter::health::HealthPolicy::new(second, second, Duration::from_secs(3), second)
                .unwrap(),
            || async { Ok::<_, std::convert::Infallible>(()) },
        );
        router(&settings, handle, pool, monitor.reader()).unwrap()
    }

    #[tokio::test]
    async fn business_routes_require_production_auth_before_database_work() {
        for authorization in [None, Some("Bearer wrong")] {
            let mut request = Request::builder()
                .method("POST")
                .uri("/records/00000000-0000-0000-0000-000000000002/deliveries")
                .header(header::CONTENT_TYPE, "application/json");
            if let Some(value) = authorization {
                request = request.header(header::AUTHORIZATION, value);
            }
            let response = app()
                .oneshot(request.body(Body::from("{}")).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            let body = to_bytes(response.into_body(), 1024).await.unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&body).unwrap()["code"],
                "authentication_required"
            );
        }
    }

    #[tokio::test]
    async fn invalid_authenticated_command_fails_before_database_acquisition() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/records/00000000-0000-0000-0000-000000000002/deliveries")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, "Bearer fake-token")
                    .body(Body::from(
                        r#"{"expected_generation":0,"idempotency_key":"key","payload":{}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
