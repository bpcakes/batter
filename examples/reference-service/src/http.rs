//! Authenticated Axum routes for delivery submission and reconciliation.

use crate::{
    auth::{AuthenticationError, BearerAuthenticator},
    config::PreparedHttp,
    delivery::{
        Delivery, DeliveryService, OwnerId, QueryError, SubmitDelivery, SubmitDisposition,
        SubmitError, SubmitRejection, SubmitResult, UncertainSubmission,
    },
    request::{TrustedRequestMetadata, install_trusted_request_metadata},
};
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, Path, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use batter::axum::{
    AssembledHttp, BoundaryAssemblyError, CorrelationId, GuardedRouter, HttpBoundary, HttpFailure,
    ProbePath, ReadinessPolicy, RequestPolicy, render_infrastructure_failure,
};
use batter::{
    RegistrationError,
    admission::{Admission, AdmissionError, Bulkhead, BulkheadCapacity},
    health::HealthReader,
    lifecycle::{LifecycleStatus, OperationAdmission},
    operation::{Interruption, OperationContext},
    registration::RegistrationTarget,
};
use runledger_postgres::RunledgerDatabase;
use serde::Serialize;
use std::{net::SocketAddr, time::Duration};
use tokio::net::TcpListener;
use tower::ServiceExt;
use uuid::Uuid;

const REQUEST_BODY_MAX_BYTES: usize = crate::delivery::PAYLOAD_MAX_BYTES + 2 * 1024;
const RESPONSE_RESERVE_MAX: Duration = Duration::from_millis(25);

#[derive(Clone)]
struct AppState {
    deliveries: DeliveryService,
    database: Bulkhead,
}

/// Failure to assemble or register the reference HTTP server.
///
/// Both causes are sanitized library errors. Neither contains request,
/// credential or configuration data.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum HttpRegistrationError {
    /// The fixed business routes could not be combined with the probes.
    #[error(transparent)]
    Assembly(#[from] BoundaryAssemblyError),
    /// Protected registration rejected the assembled server.
    #[error(transparent)]
    Registration(#[from] RegistrationError),
}

/// Assemble and register the production command router with its required
/// native peer metadata.
///
/// This is the canonical serving boundary. [`HttpBoundary`] owns server
/// correlation, the single HTTP observer, the `/live` and `/ready` probes
/// outside admission, and lifecycle admission with the request deadline around
/// every business route and the unmatched-path fallback. Inside that admission
/// the application installs [`TrustedRequestMetadata`] and then authenticates
/// the bearer credential, so a draining or not-yet-ready process rejects a
/// request before any authentication work.
///
/// Registration consumes the direct-peer policy in [`PreparedHttp`] and
/// inseparably selects Axum's native [`axum::extract::ConnectInfo`] registration
/// through [`AssembledHttp::register_with_connect_info_in`]. An arbitrary router
/// cannot tell a generic server adapter which request extensions it requires, so
/// this application-owned function keeps those two choices in one place.
///
/// ```no_run
/// use batter::{
///     health::HealthReader,
///     lifecycle::ShutdownHandle,
///     registration::RegistrationTarget,
/// };
/// use batter_example_reference_service::{
///     config::PreparedHttp,
///     http::{HttpRegistrationError, register_in},
/// };
/// use runledger_postgres::RunledgerDatabase;
/// use tokio::net::TcpListener;
///
/// async fn register<E: Send + Sync + 'static, T: RegistrationTarget + ?Sized>(
///     target: &mut T,
///     prepared: PreparedHttp,
///     control: ShutdownHandle,
///     pool: RunledgerDatabase,
///     health: HealthReader<E>,
///     listener: TcpListener,
/// ) -> Result<(), HttpRegistrationError> {
///     register_in(
///         target,
///         listener,
///         prepared,
///         control.status(),
///         control.operation_admission(),
///         pool,
///         health,
///     )
///     .await
/// }
/// ```
pub async fn register_in<E, T>(
    target: &mut T,
    listener: TcpListener,
    prepared: PreparedHttp,
    lifecycle: LifecycleStatus,
    admission: OperationAdmission,
    pool: RunledgerDatabase,
    health: HealthReader<E>,
) -> Result<(), HttpRegistrationError>
where
    E: Send + Sync + 'static,
    T: RegistrationTarget + ?Sized,
{
    let assembled = assemble(prepared, lifecycle, admission, pool, health).await?;
    assembled.register_with_connect_info_in(target, "http", listener)?;
    Ok(())
}

/// In-process request client that always supplies an explicitly selected peer.
///
/// This application-local test seam deliberately does not implement Tower's
/// `Service` traits and does not expose its inner [`Router`], so it cannot be
/// passed to [`axum::serve()`] or Batter's HTTP registration operations. Use
/// [`register_in`] for production serving.
///
/// ```compile_fail,E0308
/// use batter_example_reference_service::http::InProcessRequestClient;
///
/// fn cannot_become_a_production_router(client: InProcessRequestClient) {
///     let _: axum::Router = client;
/// }
/// ```
#[derive(Clone)]
pub struct InProcessRequestClient {
    application: Router,
}

impl InProcessRequestClient {
    /// Execute one request with the caller-owned synthetic direct socket peer.
    ///
    /// Any existing `ConnectInfo<SocketAddr>` value is replaced. The caller is
    /// responsible for choosing a peer that represents the intended scenario.
    pub async fn request(&self, mut request: Request<Body>, peer: SocketAddr) -> Response {
        request
            .extensions_mut()
            .insert(ConnectInfo::<SocketAddr>(peer));
        match self.application.clone().oneshot(request).await {
            Ok(response) => response,
            Err(error) => match error {},
        }
    }
}

/// Build a non-servable request client for an in-process caller.
///
/// The client serves the same [`HttpBoundary`] assembly as [`register_in`].
/// Business routes require lifecycle admission and the request deadline, then
/// trusted peer metadata and configured bearer authentication, then
/// process-local database admission. Unmatched paths reach a 404 fallback
/// inside the same lifecycle admission. Health endpoints stay outside those
/// gates. The outer operational middleware supplies only server-generated
/// diagnostics; it cannot select `OwnerId`.
///
/// This is a lower-level test seam, not the production serving path. Each call
/// to [`InProcessRequestClient::request`] requires an exact synthetic peer.
/// Production must use [`register_in`], which installs native peer metadata at
/// the same boundary that consumes the direct-peer policy.
///
/// ```
/// use batter::{axum::BoundaryAssemblyError, health::HealthReader, lifecycle::ShutdownHandle};
/// use batter_example_reference_service::{
///     config::PreparedHttp,
///     http::{InProcessRequestClient, in_process_client},
/// };
/// use runledger_postgres::RunledgerDatabase;
///
/// async fn can_issue_test_requests(
///     prepared: PreparedHttp,
///     control: ShutdownHandle,
///     pool: RunledgerDatabase,
///     health: HealthReader<()>,
/// ) -> Result<InProcessRequestClient, BoundaryAssemblyError> {
///     in_process_client(
///         prepared,
///         control.status(),
///         control.operation_admission(),
///         pool,
///         health,
///     )
///     .await
/// }
/// ```
///
/// Maintenance preparation cannot cross this boundary:
///
/// ```compile_fail,E0308
/// use batter::{health::HealthReader, lifecycle::ShutdownHandle};
/// use batter_example_reference_service::{config::PreparedMaintenance, http::in_process_client};
/// use runledger_postgres::RunledgerDatabase;
///
/// async fn cannot_route(
///     prepared: PreparedMaintenance,
///     control: ShutdownHandle,
///     pool: RunledgerDatabase,
///     health: HealthReader<()>,
/// ) {
///     let application = in_process_client(
///         prepared,
///         control.status(),
///         control.operation_admission(),
///         pool,
///         health,
///     )
///     .await;
/// }
/// ```
pub async fn in_process_client<E: Send + Sync + 'static>(
    prepared: PreparedHttp,
    lifecycle: LifecycleStatus,
    admission: OperationAdmission,
    pool: RunledgerDatabase,
    health: HealthReader<E>,
) -> Result<InProcessRequestClient, BoundaryAssemblyError> {
    let assembled = assemble(prepared, lifecycle, admission, pool, health).await?;
    // The adapter has no request-only client yet; this type keeps the
    // assembled router private and unservable.
    Ok(InProcessRequestClient {
        application: assembled.into_router(),
    })
}

async fn assemble<E: Send + Sync + 'static>(
    prepared: PreparedHttp,
    lifecycle: LifecycleStatus,
    admission: OperationAdmission,
    pool: RunledgerDatabase,
    health: HealthReader<E>,
) -> Result<AssembledHttp, BoundaryAssemblyError> {
    let routes = delivery_routes(pool, prepared.bulkhead_capacity);
    assemble_routes(
        routes,
        prepared,
        admission,
        ReadinessPolicy::new(lifecycle, health),
    )
    .await
}

fn delivery_routes(pool: RunledgerDatabase, bulkhead_capacity: BulkheadCapacity) -> GuardedRouter {
    GuardedRouter::new()
        .route("/records/{record_id}/deliveries", post(submit_delivery))
        .route("/deliveries/{delivery_id}", get(get_delivery))
        .route(
            "/delivery-commands/{idempotency_key}",
            get(get_delivery_by_key),
        )
        .with_state(AppState {
            deliveries: DeliveryService::new(pool),
            database: Bulkhead::new(bulkhead_capacity),
        })
}

/// Install the application's layers around `routes` and assemble the boundary.
///
/// The boundary adds correlation and the observer outermost, keeps the probes
/// outside admission and admits every guarded route and the fallback with the
/// request deadline. Route layers added later run outside earlier ones, so an
/// admitted request installs trusted metadata, then authenticates, then reaches
/// the handler under the body limit.
async fn assemble_routes<E: Send + Sync + 'static>(
    routes: GuardedRouter,
    prepared: PreparedHttp,
    admission: OperationAdmission,
    readiness: ReadinessPolicy<E>,
) -> Result<AssembledHttp, BoundaryAssemblyError> {
    let policy = RequestPolicy::new(admission, prepared.request_budget).with_infrastructure_json();
    let guarded = routes
        .layer(DefaultBodyLimit::max(REQUEST_BODY_MAX_BYTES))
        .route_layer(middleware::from_fn_with_state(
            prepared.authenticator,
            authenticate,
        ))
        .route_layer(middleware::from_fn_with_state(
            prepared.trusted_peer_policy,
            install_trusted_request_metadata,
        ))
        .fallback(|| async { StatusCode::NOT_FOUND });
    HttpBoundary::new(policy)
        .with_liveness(ProbePath::new("/live").expect("static liveness path is valid"))
        .expect("liveness path is unique")
        .with_readiness(
            ProbePath::new("/ready").expect("static readiness path is valid"),
            readiness,
        )
        .expect("readiness path is unique")
        .assemble(guarded)
        .await
}

async fn authenticate(
    State(authenticator): State<BearerAuthenticator>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    request.extensions_mut().remove::<OwnerId>();
    let Some(metadata) = request
        .extensions()
        .get::<TrustedRequestMetadata>()
        .cloned()
    else {
        let correlation_id = request.extensions().get::<CorrelationId>();
        return render_infrastructure_failure(HttpFailure::Internal, correlation_id);
    };
    match authenticator.authenticate(request.headers().get(header::AUTHORIZATION)) {
        Ok(owner) => {
            request.extensions_mut().insert(owner);
            next.run(request).await
        }
        Err(error) => authentication_failure(error, &metadata),
    }
}

#[derive(Serialize)]
struct Problem<'a> {
    code: &'static str,
    message: &'static str,
    request_id: &'a str,
}

fn problem(
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    metadata: &TrustedRequestMetadata,
) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "application/problem+json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Json(Problem {
            code,
            message,
            request_id: metadata.correlation_id().as_str(),
        }),
    )
        .into_response()
}

fn authentication_failure(
    _error: AuthenticationError,
    metadata: &TrustedRequestMetadata,
) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [
            (header::WWW_AUTHENTICATE, "Bearer"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Json(Problem {
            code: "authentication_required",
            message: "valid authentication is required",
            request_id: metadata.correlation_id().as_str(),
        }),
    )
        .into_response()
}

async fn submit_delivery(
    State(state): State<AppState>,
    Extension(owner): Extension<OwnerId>,
    Extension(metadata): Extension<TrustedRequestMetadata>,
    Extension(context): Extension<OperationContext>,
    Path(record_id): Path<Uuid>,
    Json(request): Json<SubmitDelivery>,
) -> Response {
    let _permit = match state.database.enter(&context, Admission::Reject).await {
        Ok(permit) => permit,
        Err(error) => return admission_failure(error, &metadata),
    };
    let command_context = match command_context(&context) {
        Ok(context) => context,
        Err(interruption) => {
            return render_infrastructure_failure(
                interruption_failure(interruption),
                Some(metadata.correlation_id()),
            );
        }
    };
    let key = request.idempotency_key.clone();
    match state
        .deliveries
        .submit(&command_context, owner, record_id, request)
        .await
    {
        Ok(result) => accepted(result, &metadata),
        Err(error) => submit_failure(error, &key, &metadata),
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
        .map(|owner| owner.into_context())
        .map_err(|_| Interruption::DeadlineExceeded)
}

#[derive(Serialize)]
struct SubmitResponse<'a> {
    outcome: SubmitDisposition,
    delivery: Delivery,
    request_id: &'a str,
}

fn accepted(result: SubmitResult, metadata: &TrustedRequestMetadata) -> Response {
    (
        StatusCode::ACCEPTED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(SubmitResponse {
            outcome: result.disposition,
            delivery: result.delivery,
            request_id: metadata.correlation_id().as_str(),
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
    request_id: &'a str,
}

fn submit_failure(error: SubmitError, key: &str, metadata: &TrustedRequestMetadata) -> Response {
    match error {
        SubmitError::Invalid(_) => problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_delivery_command",
            "the delivery command is invalid",
            metadata,
        ),
        SubmitError::Rejected(SubmitRejection::RecordNotObserved) => problem(
            StatusCode::NOT_FOUND,
            "record_not_observed",
            "the target record was not observed",
            metadata,
        ),
        SubmitError::Rejected(SubmitRejection::StaleGeneration { .. }) => problem(
            StatusCode::CONFLICT,
            "stale_record_generation",
            "the target record generation changed",
            metadata,
        ),
        SubmitError::Rejected(SubmitRejection::IdempotencyConflict) => problem(
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "the idempotency key is retained with different command input",
            metadata,
        ),
        SubmitError::Uncertain(uncertain) => uncertain_response(&uncertain, key, metadata),
        SubmitError::Interrupted(interruption) => render_infrastructure_failure(
            interruption_failure(interruption),
            Some(metadata.correlation_id()),
        ),
        SubmitError::Storage(error) if error.is_pool_unavailable() => {
            render_infrastructure_failure(HttpFailure::Unavailable, Some(metadata.correlation_id()))
        }
        SubmitError::Storage(_) => {
            render_infrastructure_failure(HttpFailure::Internal, Some(metadata.correlation_id()))
        }
    }
}

fn uncertain_response(
    uncertain: &UncertainSubmission,
    key: &str,
    metadata: &TrustedRequestMetadata,
) -> Response {
    let code = match uncertain {
        UncertainSubmission::Interrupted(_) => "submission_interrupted",
        UncertainSubmission::Atomic(error) => match &**error {
            batter::sqlx::PgAtomicUncertainty::CommitUnconfirmed { .. } => {
                "commit_acknowledgement_lost"
            }
            batter::sqlx::PgAtomicUncertainty::RollbackUnconfirmed { .. } => {
                "rollback_acknowledgement_lost"
            }
            batter::sqlx::PgAtomicUncertainty::ScopeLost { .. } => "submission_scope_uncertain",
        },
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
            request_id: metadata.correlation_id().as_str(),
        }),
    )
        .into_response()
}

async fn get_delivery(
    State(state): State<AppState>,
    Extension(owner): Extension<OwnerId>,
    Extension(metadata): Extension<TrustedRequestMetadata>,
    Extension(context): Extension<OperationContext>,
    Path(delivery_id): Path<Uuid>,
) -> Response {
    let _permit = match state.database.enter(&context, Admission::Reject).await {
        Ok(permit) => permit,
        Err(error) => return admission_failure(error, &metadata),
    };
    query_response(
        state
            .deliveries
            .get_by_id(&context, owner, delivery_id)
            .await,
        false,
        &metadata,
    )
}

async fn get_delivery_by_key(
    State(state): State<AppState>,
    Extension(owner): Extension<OwnerId>,
    Extension(metadata): Extension<TrustedRequestMetadata>,
    Extension(context): Extension<OperationContext>,
    Path(idempotency_key): Path<String>,
) -> Response {
    let _permit = match state.database.enter(&context, Admission::Reject).await {
        Ok(permit) => permit,
        Err(error) => return admission_failure(error, &metadata),
    };
    query_response(
        state
            .deliveries
            .get_by_key(&context, owner, &idempotency_key)
            .await,
        true,
        &metadata,
    )
}

#[derive(Serialize)]
struct DeliveryResponse<'a> {
    outcome: &'static str,
    delivery: Delivery,
    request_id: &'a str,
}

#[derive(Serialize)]
struct NotObservedResponse<'a> {
    outcome: &'static str,
    message: &'static str,
    request_id: &'a str,
}

fn query_response(
    result: Result<Option<Delivery>, QueryError>,
    reconciliation: bool,
    metadata: &TrustedRequestMetadata,
) -> Response {
    match result {
        Ok(Some(delivery)) => (
            StatusCode::OK,
            [(header::CACHE_CONTROL, "no-store")],
            Json(DeliveryResponse {
                outcome: "observed",
                delivery,
                request_id: metadata.correlation_id().as_str(),
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
                request_id: metadata.correlation_id().as_str(),
            }),
        )
            .into_response(),
        Err(QueryError::Invalid(_)) => problem(
            StatusCode::BAD_REQUEST,
            "invalid_delivery_query",
            "the delivery query is invalid",
            metadata,
        ),
        Err(QueryError::Interrupted(interruption)) => render_infrastructure_failure(
            interruption_failure(interruption),
            Some(metadata.correlation_id()),
        ),
        Err(QueryError::Storage(error)) if error.is_pool_unavailable() => {
            render_infrastructure_failure(HttpFailure::Unavailable, Some(metadata.correlation_id()))
        }
        Err(QueryError::Storage(_)) => {
            render_infrastructure_failure(HttpFailure::Internal, Some(metadata.correlation_id()))
        }
    }
}

fn admission_failure(error: AdmissionError, metadata: &TrustedRequestMetadata) -> Response {
    let failure = match error {
        AdmissionError::Overloaded => HttpFailure::Overloaded,
        AdmissionError::Closed => HttpFailure::Unavailable,
        AdmissionError::Interrupted(interruption) => interruption_failure(interruption),
    };
    render_infrastructure_failure(failure, Some(metadata.correlation_id()))
}

const fn interruption_failure(interruption: Interruption) -> HttpFailure {
    match interruption {
        Interruption::Cancelled => HttpFailure::Cancelled,
        Interruption::DeadlineExceeded => HttpFailure::DeadlineExceeded,
    }
}

#[cfg(test)]
mod tests;
