//! HTTP boundary: guarded record routes plus liveness/readiness probes.

use crate::records::RecordsStore;
use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection, rejection::PathRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use batter::axum::{
    AdmittedRequest, AssembledHttp, BoundaryAssemblyError, CorrelationId, GuardedRouter,
    HttpBoundary, HttpFailure, ProbePath, ReadinessPolicy, RequestPolicy,
    ResponseConstructionBudget, render_infrastructure_failure,
};
use batter::lifecycle::OperationAdmission;
use batter::operation::OperationError;
use batter::runledger::PgAtomicError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Longest accepted record name, in bytes.
const MAX_NAME_BYTES: usize = 200;

#[derive(Clone)]
pub struct AppState {
    pub records: RecordsStore,
}

#[derive(Deserialize)]
pub struct CreateRecord {
    pub name: String,
}

#[derive(Serialize)]
struct Accepted {
    id: Uuid,
}

/// The application's own error envelope for domain and client failures.
#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
    request_id: String,
}

fn client_error(
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    id: &CorrelationId,
) -> Response {
    (
        status,
        Json(ErrorBody {
            code,
            message,
            request_id: id.to_string(),
        }),
    )
        .into_response()
}

/// `POST /records`: insert the row and enqueue its notification atomically.
async fn create_record(
    State(state): State<AppState>,
    admitted: AdmittedRequest,
    body: Result<Json<CreateRecord>, JsonRejection>,
) -> Response {
    let id = admitted.correlation_id();
    let Json(body) = match body {
        Ok(body) => body,
        Err(_) => {
            return client_error(
                StatusCode::BAD_REQUEST,
                "invalid_body",
                "Expected a JSON object with a name.",
                id,
            );
        }
    };
    let name = body.name.trim();
    if name.is_empty() || name.len() > MAX_NAME_BYTES {
        return client_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_name",
            "name must be 1 to 200 bytes of non-blank text.",
            id,
        );
    }
    match state.records.create(admitted.context(), name).await {
        Ok(record_id) => (StatusCode::ACCEPTED, Json(Accepted { id: record_id })).into_response(),
        Err(OperationError::Interrupted(reason)) => {
            admitted.interruption_responder().render(reason)
        }
        Err(OperationError::Failed(error)) => {
            tracing::warn!(outcome = failure_outcome(&error), "record creation failed");
            render_infrastructure_failure(map_create_failure(&error), Some(id))
        }
    }
}

/// Acknowledged rejections and begin failures are retryable by the client;
/// an unconfirmed disposition is reported as an internal error because the
/// row may or may not exist.
fn map_create_failure(error: &PgAtomicError<Uuid, crate::records::CreateRejection>) -> HttpFailure {
    match error {
        PgAtomicError::Begin(_) | PgAtomicError::Rejected(_) => HttpFailure::Unavailable,
        PgAtomicError::Uncertain(_) => HttpFailure::Internal,
    }
}

fn failure_outcome(error: &PgAtomicError<Uuid, crate::records::CreateRejection>) -> &'static str {
    match error {
        PgAtomicError::Begin(_) => "begin_failed",
        PgAtomicError::Rejected(_) => "rolled_back",
        PgAtomicError::Uncertain(_) => "uncertain",
    }
}

/// `GET /records/{id}`: the row, or 404.
async fn get_record(
    State(state): State<AppState>,
    admitted: AdmittedRequest,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let id = admitted.correlation_id();
    let Path(record_id) = match path {
        Ok(path) => path,
        Err(_) => return not_found(id),
    };
    match state.records.get(admitted.context(), record_id).await {
        Ok(Some(record)) => Json(record).into_response(),
        Ok(None) => not_found(id),
        Err(OperationError::Interrupted(reason)) => {
            admitted.interruption_responder().render(reason)
        }
        Err(OperationError::Failed(_)) => {
            tracing::warn!("record read failed");
            render_infrastructure_failure(HttpFailure::Unavailable, Some(id))
        }
    }
}

fn not_found(id: &CorrelationId) -> Response {
    client_error(
        StatusCode::NOT_FOUND,
        "record_not_found",
        "No record has that id.",
        id,
    )
}

/// Assemble the complete boundary: probes outside admission, every record
/// route admitted under the request budget, one observer outermost.
pub async fn assemble(
    admission: OperationAdmission,
    request_budget: ResponseConstructionBudget,
    readiness: ReadinessPolicy<sqlx::Error>,
    state: AppState,
) -> Result<AssembledHttp, BoundaryAssemblyError> {
    let policy = RequestPolicy::new(admission, request_budget).with_infrastructure_json();
    let application = GuardedRouter::new()
        .route("/records", post(create_record))
        .route("/records/{id}", get(get_record))
        .with_state(state);
    HttpBoundary::new(policy)
        .with_liveness(ProbePath::new("/live").expect("static liveness path is valid"))
        .expect("liveness path is unique")
        .with_readiness(
            ProbePath::new("/ready").expect("static readiness path is valid"),
            readiness,
        )
        .expect("readiness path is unique")
        .assemble(application)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::extract::Request;
    use axum::http::{Method, header};
    use batter::axum::InProcessClient;
    use batter::health::{HealthMonitor, HealthPolicy};
    use batter::lifecycle::ShutdownHandle;
    use batter::runledger::{PgSessionProfile, RunledgerDatabase};
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use std::time::Duration;

    /// A lazy pool aimed at a closed loopback port: the real handlers run, and
    /// every acquisition fails fast without a server.
    fn unreachable_store() -> RecordsStore {
        let profile = PgSessionProfile::new(
            "records",
            "records",
            vec!["public".into()],
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .expect("valid profile");
        let options = PgConnectOptions::new()
            .host("127.0.0.1")
            .port(1)
            .username("records")
            .database("records");
        let database = RunledgerDatabase::connect_lazy(
            options,
            profile,
            PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_millis(300)),
        )
        .expect("lazy pool construction does not connect");
        RecordsStore::new(database, Duration::from_millis(300))
    }

    /// A probe that never runs: the monitor is kept alive but unregistered, so
    /// readers stay in their initial unsampled state.
    type IdleProbe = fn() -> std::future::Ready<Result<(), sqlx::Error>>;

    struct Harness {
        client: InProcessClient,
        _monitor: HealthMonitor<IdleProbe, sqlx::Error>,
    }

    async fn harness(approve: bool) -> Harness {
        let (control, approval) = ShutdownHandle::new_with_readiness_approval();
        if approve {
            approval.approve();
        }
        let second = Duration::from_secs(1);
        let policy = HealthPolicy::new(second, second, second * 3, second).expect("valid policy");
        fn probe() -> std::future::Ready<Result<(), sqlx::Error>> {
            std::future::ready(Ok(()))
        }
        let monitor: HealthMonitor<IdleProbe, sqlx::Error> = HealthMonitor::new(policy, probe);
        let readiness = ReadinessPolicy::new(control.status(), monitor.reader());
        let budget = ResponseConstructionBudget::new(Duration::from_secs(2)).expect("valid budget");
        let assembled = assemble(
            control.operation_admission(),
            budget,
            readiness,
            AppState {
                records: unreachable_store(),
            },
        )
        .await
        .expect("boundary assembles");
        Harness {
            client: assembled.in_process(),
            _monitor: monitor,
        }
    }

    async fn json(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json body")
    }

    fn post(body: &str) -> Request<Body> {
        Request::builder()
            .method(Method::POST)
            .uri("/records")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .expect("request")
    }

    #[tokio::test]
    async fn liveness_answers_200() {
        let harness = harness(true).await;
        let response = harness
            .client
            .request(Request::builder().uri("/live").body(Body::empty()).unwrap())
            .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn readiness_is_503_before_the_dependency_was_sampled() {
        let harness = harness(true).await;
        let response = harness
            .client
            .request(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn guarded_routes_are_rejected_while_starting() {
        let harness = harness(false).await;
        let response = harness.client.request(post(r#"{"name":"a"}"#)).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = json(response).await;
        assert_eq!(body["code"], "service_unavailable");
        assert!(body["request_id"].is_string());
    }

    #[tokio::test]
    async fn malformed_json_is_400_with_the_application_envelope() {
        let harness = harness(true).await;
        let response = harness.client.request(post("{not json")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = json(response).await;
        assert_eq!(body["code"], "invalid_body");
        assert!(body["request_id"].is_string());
    }

    #[tokio::test]
    async fn blank_name_is_422() {
        let harness = harness(true).await;
        let response = harness.client.request(post(r#"{"name":"   "}"#)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(json(response).await["code"], "invalid_name");
    }

    #[tokio::test]
    async fn unreachable_database_is_503_for_create() {
        let harness = harness(true).await;
        let response = harness.client.request(post(r#"{"name":"alpha"}"#)).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json(response).await["code"], "service_unavailable");
    }

    #[tokio::test]
    async fn malformed_id_is_404() {
        let harness = harness(true).await;
        let response = harness
            .client
            .request(
                Request::builder()
                    .uri("/records/not-a-uuid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(json(response).await["code"], "record_not_found");
    }

    #[tokio::test]
    async fn unreachable_database_is_503_for_read() {
        let harness = harness(true).await;
        let response = harness
            .client
            .request(
                Request::builder()
                    .uri(format!("/records/{}", Uuid::now_v7()))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn unknown_paths_are_404() {
        let harness = harness(true).await;
        let response = harness
            .client
            .request(Request::builder().uri("/nope").body(Body::empty()).unwrap())
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
