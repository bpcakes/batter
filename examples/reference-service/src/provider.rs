//! Application-owned transport for the selected reference effect protocol.
//!
//! This is intentionally private application code. It does not claim that an
//! arbitrary provider supports keyed lookup, payload retention, or the outcome
//! vocabulary implemented by the local protocol fixture.

use batter::{
    operation::{Interruption, OperationContext, OperationError},
    settings::SecretString,
};
use reqwest::{Request, Response, StatusCode, Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fmt, time::Duration};
use tokio::sync::OwnedSemaphorePermit;
use uuid::Uuid;

pub(crate) const EFFECT_PROTOCOL_VERSION: u8 = 1;
pub(crate) const IDEMPOTENCY_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
// Application retry policy, deliberately independent of key retention. Never
// shorten an accepted provider lower bound to make work run earlier.
const MAX_ACCEPTED_RETRY_DELAY: Duration = Duration::from_secs(24 * 60 * 60);
const RESPONSE_MAX_BYTES: usize = 64 * 1024;
const PROVIDER_EFFECT_ID_MAX_BYTES: usize = 256;

/// Canonical provider body retained with one application delivery identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ProviderEffectRequest {
    pub(crate) version: u8,
    pub(crate) effect_id: Uuid,
    pub(crate) owner_id: Uuid,
    pub(crate) record_id: Uuid,
    pub(crate) record_generation: i64,
    pub(crate) payload: Value,
}

impl ProviderEffectRequest {
    pub(crate) fn idempotency_key(&self) -> String {
        format!("reference-delivery:{}", self.effect_id)
    }
}

#[derive(Clone)]
pub(crate) struct ProviderClient {
    client: reqwest::Client,
    base_url: Url,
    token: SecretString,
}

impl ProviderClient {
    pub(crate) fn new(base_url: Url, token: SecretString) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .build()?;
        Ok(Self {
            client,
            base_url,
            token,
        })
    }

    /// Build and serialize a mutation request before dispatch becomes possible.
    pub(crate) fn prepare_dispatch(
        &self,
        key: &str,
        effect: &ProviderEffectRequest,
    ) -> Result<PreparedDispatch, reqwest::Error> {
        let request = self
            .client
            .post(self.endpoint("effects"))
            .bearer_auth(self.token.expose_secret())
            .header("Idempotency-Key", key)
            .json(effect)
            .build()?;
        Ok(PreparedDispatch {
            client: self.client.clone(),
            request,
            expected: effect.clone(),
        })
    }

    pub(crate) async fn reconcile(
        &self,
        context: &OperationContext,
        _permit: OwnedSemaphorePermit,
        key: &str,
        expected: &ProviderEffectRequest,
    ) -> Result<ReconcileOutcome, ProviderCallError> {
        let mut url = self.endpoint("effects");
        url.path_segments_mut()
            .expect("validated provider URL is a base")
            .push(key);
        let request = self
            .client
            .get(url)
            .bearer_auth(self.token.expose_secret())
            .build()
            .map_err(ProviderCallError::Send)?;
        let client = self.client.clone();
        let response = context
            .run("provider.reconcile", move |_| async move {
                execute_bounded(client, request).await
            })
            .await
            .map_err(ProviderCallError::from_operation)?;
        Ok(classify_reconcile(response, expected))
    }

    fn endpoint(&self, segment: &str) -> Url {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .expect("validated provider URL is a base")
            .push(segment);
        url
    }
}

impl fmt::Debug for ProviderClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderClient([REDACTED])")
    }
}

pub(crate) struct PreparedDispatch {
    client: reqwest::Client,
    request: Request,
    expected: ProviderEffectRequest,
}

impl PreparedDispatch {
    pub(crate) async fn execute(
        self,
        context: &OperationContext,
        _permit: OwnedSemaphorePermit,
    ) -> Result<DispatchOutcome, ProviderCallError> {
        let Self {
            client,
            request,
            expected,
        } = self;
        let response = match context
            .run("provider.dispatch", move |_| async move {
                execute_bounded(client, request).await
            })
            .await
        {
            Ok(response) => response,
            Err(OperationError::Failed(ProviderCallError::Send(error))) if error.is_connect() => {
                return Ok(DispatchOutcome::KnownNotDispatched {
                    retry_not_before: Duration::ZERO,
                });
            }
            Err(error) => return Err(ProviderCallError::from_operation(error)),
        };
        Ok(classify_dispatch(response, &expected))
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum DispatchOutcome {
    Accepted { provider_effect_id: String },
    KnownNotDispatched { retry_not_before: Duration },
    BusinessDenied,
    Conflict,
    Indeterminate,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ReconcileOutcome {
    Accepted { provider_effect_id: String },
    Absent,
    Expired,
    Conflict,
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ProviderCallError {
    #[error("provider operation was interrupted")]
    Interrupted(#[source] Interruption),
    #[error("provider request failed")]
    Send(#[source] reqwest::Error),
    #[error("provider response body failed")]
    Body(#[source] reqwest::Error),
    #[error("provider response exceeded the configured bound")]
    ResponseTooLarge,
}

impl ProviderCallError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Interrupted(Interruption::DeadlineExceeded) => "delivery.provider_deadline",
            Self::Interrupted(Interruption::Cancelled) => "delivery.provider_cancelled",
            Self::ResponseTooLarge => "delivery.provider_response_too_large",
            Self::Send(error) | Self::Body(error) if error.is_timeout() => {
                "delivery.provider_timeout"
            }
            Self::Send(_) => "delivery.provider_send_failed",
            Self::Body(_) => "delivery.provider_body_failed",
        }
    }

    fn from_operation(error: OperationError<Self>) -> Self {
        match error {
            OperationError::Interrupted(interruption) => Self::Interrupted(interruption),
            OperationError::Failed(error) => error,
        }
    }
}

struct BufferedResponse {
    status: StatusCode,
    body: Vec<u8>,
}

#[derive(Deserialize)]
struct AcceptedResponse {
    effect_id: Uuid,
    provider_effect_id: String,
    request: ProviderEffectRequest,
}

#[derive(Deserialize)]
struct ErrorResponse {
    code: String,
    #[serde(default)]
    retry_after_ms: Option<u64>,
    #[serde(default)]
    retention_active: Option<bool>,
}

fn classify_dispatch(
    response: BufferedResponse,
    expected: &ProviderEffectRequest,
) -> DispatchOutcome {
    classify_dispatch_body(response.status, &response.body, expected)
}

fn classify_dispatch_body(
    status: StatusCode,
    body: &[u8],
    expected: &ProviderEffectRequest,
) -> DispatchOutcome {
    if status == StatusCode::OK {
        return match accepted_provider_effect_id(body, expected) {
            Ok(provider_effect_id) => DispatchOutcome::Accepted { provider_effect_id },
            Err(AcceptedResponseError::Conflict) => DispatchOutcome::Conflict,
            Err(AcceptedResponseError::Invalid) => DispatchOutcome::Indeterminate,
        };
    }

    let Ok(error) = serde_json::from_slice::<ErrorResponse>(body) else {
        return DispatchOutcome::Indeterminate;
    };
    match (status, error.code.as_str()) {
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited") => {
            let Some(milliseconds) = error.retry_after_ms else {
                return DispatchOutcome::Indeterminate;
            };
            let delay = Duration::from_millis(milliseconds);
            if delay > MAX_ACCEPTED_RETRY_DELAY {
                DispatchOutcome::Indeterminate
            } else {
                DispatchOutcome::KnownNotDispatched {
                    retry_not_before: delay,
                }
            }
        }
        (StatusCode::UNPROCESSABLE_ENTITY, "business_denied") => DispatchOutcome::BusinessDenied,
        (StatusCode::CONFLICT, "idempotency_conflict") => DispatchOutcome::Conflict,
        (StatusCode::CONFLICT, "concurrent_request") => DispatchOutcome::Indeterminate,
        _ => DispatchOutcome::Indeterminate,
    }
}

fn classify_reconcile(
    response: BufferedResponse,
    expected: &ProviderEffectRequest,
) -> ReconcileOutcome {
    if response.status == StatusCode::OK {
        return match accepted_provider_effect_id(&response.body, expected) {
            Ok(provider_effect_id) => ReconcileOutcome::Accepted { provider_effect_id },
            Err(AcceptedResponseError::Conflict) => ReconcileOutcome::Conflict,
            Err(AcceptedResponseError::Invalid) => ReconcileOutcome::Unavailable,
        };
    }

    let Ok(error) = serde_json::from_slice::<ErrorResponse>(&response.body) else {
        return ReconcileOutcome::Unavailable;
    };
    match (response.status, error.code.as_str(), error.retention_active) {
        (StatusCode::NOT_FOUND, "not_found", Some(true)) => ReconcileOutcome::Absent,
        (StatusCode::GONE, "retention_expired", _) => ReconcileOutcome::Expired,
        (StatusCode::CONFLICT, "idempotency_conflict", _) => ReconcileOutcome::Conflict,
        _ => ReconcileOutcome::Unavailable,
    }
}

#[derive(Clone, Copy)]
enum AcceptedResponseError {
    Invalid,
    Conflict,
}

fn accepted_provider_effect_id(
    body: &[u8],
    expected: &ProviderEffectRequest,
) -> Result<String, AcceptedResponseError> {
    let accepted = serde_json::from_slice::<AcceptedResponse>(body)
        .map_err(|_| AcceptedResponseError::Invalid)?;
    if !valid_provider_effect_id(&accepted.provider_effect_id) {
        return Err(AcceptedResponseError::Invalid);
    }
    if accepted.effect_id != expected.effect_id || accepted.request != *expected {
        return Err(AcceptedResponseError::Conflict);
    }
    Ok(accepted.provider_effect_id)
}

async fn execute_bounded(
    client: reqwest::Client,
    request: Request,
) -> Result<BufferedResponse, ProviderCallError> {
    let response = client
        .execute(request)
        .await
        .map_err(ProviderCallError::Send)?;
    let status = response.status();
    let body = bounded_body(response).await?;
    Ok(BufferedResponse { status, body })
}

async fn bounded_body(mut response: Response) -> Result<Vec<u8>, ProviderCallError> {
    if response
        .content_length()
        .is_some_and(|length| length > RESPONSE_MAX_BYTES as u64)
    {
        return Err(ProviderCallError::ResponseTooLarge);
    }
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or(0)
            .min(RESPONSE_MAX_BYTES as u64) as usize,
    );
    while let Some(chunk) = response.chunk().await.map_err(ProviderCallError::Body)? {
        if body.len().saturating_add(chunk.len()) > RESPONSE_MAX_BYTES {
            return Err(ProviderCallError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn valid_provider_effect_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= PROVIDER_EFFECT_ID_MAX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
fn test_permit() -> OwnedSemaphorePermit {
    std::sync::Arc::new(tokio::sync::Semaphore::new(1))
        .try_acquire_owned()
        .unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::{Path, State},
        http::{HeaderMap, header},
        response::{IntoResponse, Response as AxumResponse},
        routing::{get, post},
    };
    use serde_json::json;
    use std::{collections::HashMap, sync::Arc};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::Mutex,
    };

    #[derive(Clone, Default)]
    struct FixtureState {
        effects: Arc<Mutex<HashMap<String, ProviderEffectRequest>>>,
    }

    async fn fixture_dispatch(
        State(state): State<FixtureState>,
        headers: HeaderMap,
        Json(request): Json<ProviderEffectRequest>,
    ) -> AxumResponse {
        if headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            != Some("Bearer fixture-secret")
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let Some(key) = headers
            .get("idempotency-key")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
        else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let mut effects = state.effects.lock().await;
        match effects.get(&key) {
            Some(retained) if retained != &request => (
                StatusCode::CONFLICT,
                Json(json!({"code": "idempotency_conflict"})),
            )
                .into_response(),
            Some(retained) => accepted_fixture(retained),
            None => {
                effects.insert(key, request.clone());
                accepted_fixture(&request)
            }
        }
    }

    async fn fixture_reconcile(
        State(state): State<FixtureState>,
        Path(key): Path<String>,
        headers: HeaderMap,
    ) -> AxumResponse {
        if headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            != Some("Bearer fixture-secret")
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        match state.effects.lock().await.get(&key).cloned() {
            Some(request) => accepted_fixture(&request),
            None => (
                StatusCode::NOT_FOUND,
                Json(json!({"code": "not_found", "retention_active": true})),
            )
                .into_response(),
        }
    }

    fn accepted_fixture(request: &ProviderEffectRequest) -> AxumResponse {
        (
            StatusCode::OK,
            Json(json!({
                "effect_id": request.effect_id,
                "provider_effect_id": format!("provider:{}", request.effect_id),
                "request": request,
            })),
        )
            .into_response()
    }

    #[test]
    fn stable_key_is_derived_from_effect_identity() {
        let effect = ProviderEffectRequest {
            version: EFFECT_PROTOCOL_VERSION,
            effect_id: Uuid::from_u128(1),
            owner_id: Uuid::from_u128(2),
            record_id: Uuid::from_u128(3),
            record_generation: 4,
            payload: serde_json::json!({"channel": "example"}),
        };
        assert_eq!(
            effect.idempotency_key(),
            "reference-delivery:00000000-0000-0000-0000-000000000001"
        );
        assert_eq!(serde_json::to_value(&effect).unwrap()["version"], 1);
    }

    #[test]
    fn provider_effect_identifiers_are_bounded_protocol_values() {
        assert!(valid_provider_effect_id("effect_123"));
        assert!(!valid_provider_effect_id(""));
        assert!(!valid_provider_effect_id("contains space"));
        assert!(!valid_provider_effect_id(
            &"x".repeat(PROVIDER_EFFECT_ID_MAX_BYTES + 1)
        ));
    }

    #[test]
    fn only_typed_provider_responses_authorize_known_replay() {
        let expected = ProviderEffectRequest {
            version: EFFECT_PROTOCOL_VERSION,
            effect_id: Uuid::from_u128(1),
            owner_id: Uuid::from_u128(2),
            record_id: Uuid::from_u128(3),
            record_generation: 4,
            payload: json!({"channel": "example"}),
        };
        assert_eq!(
            classify_dispatch_body(
                StatusCode::TOO_MANY_REQUESTS,
                br#"{"code":"rate_limited","retry_after_ms":25}"#,
                &expected,
            ),
            DispatchOutcome::KnownNotDispatched {
                retry_not_before: Duration::from_millis(25)
            }
        );
        for (status, body) in [
            (StatusCode::TOO_MANY_REQUESTS, b"retry later".as_slice()),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                br#"{"code":"rate_limited","retry_after_ms":25}"#.as_slice(),
            ),
            (
                StatusCode::TOO_MANY_REQUESTS,
                br#"{"code":"temporary","retry_after_ms":25}"#.as_slice(),
            ),
        ] {
            assert_eq!(
                classify_dispatch_body(status, body, &expected),
                DispatchOutcome::Indeterminate
            );
        }
    }

    #[test]
    fn only_exact_retained_absence_authorizes_replay_after_reconciliation() {
        let expected = ProviderEffectRequest {
            version: EFFECT_PROTOCOL_VERSION,
            effect_id: Uuid::from_u128(31),
            owner_id: Uuid::from_u128(32),
            record_id: Uuid::from_u128(33),
            record_generation: 34,
            payload: json!({"channel": "example"}),
        };
        for (status, body, outcome) in [
            (
                StatusCode::NOT_FOUND,
                br#"{"code":"not_found","retention_active":true}"#.as_slice(),
                ReconcileOutcome::Absent,
            ),
            (
                StatusCode::NOT_FOUND,
                br#"{"code":"not_found","retention_active":false}"#.as_slice(),
                ReconcileOutcome::Unavailable,
            ),
            (
                StatusCode::NOT_FOUND,
                br#"{"code":"not_found"}"#.as_slice(),
                ReconcileOutcome::Unavailable,
            ),
            (
                StatusCode::OK,
                br#"{"code":"not_found","retention_active":true}"#.as_slice(),
                ReconcileOutcome::Unavailable,
            ),
        ] {
            assert_eq!(
                classify_reconcile(
                    BufferedResponse {
                        status,
                        body: body.to_vec(),
                    },
                    &expected,
                ),
                outcome
            );
        }
    }

    #[tokio::test]
    async fn real_transport_replays_conflicts_and_reconciles_canonical_identity() {
        let state = FixtureState::default();
        let app = Router::new()
            .route("/effects", post(fixture_dispatch))
            .route("/effects/{key}", get(fixture_reconcile))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        let client = ProviderClient::new(
            Url::parse(&format!("http://{address}/")).unwrap(),
            SecretString::new("fixture-secret".to_owned()),
        )
        .unwrap();
        let request = ProviderEffectRequest {
            version: EFFECT_PROTOCOL_VERSION,
            effect_id: Uuid::from_u128(11),
            owner_id: Uuid::from_u128(12),
            record_id: Uuid::from_u128(13),
            record_generation: 14,
            payload: json!({"channel": "example"}),
        };
        let key = request.idempotency_key();
        let operation = batter::operation::OperationOwner::new(Duration::from_secs(2))
            .unwrap()
            .into_context();
        for _ in 0..2 {
            assert_eq!(
                client
                    .prepare_dispatch(&key, &request)
                    .unwrap()
                    .execute(&operation, test_permit())
                    .await
                    .unwrap(),
                DispatchOutcome::Accepted {
                    provider_effect_id: format!("provider:{}", request.effect_id)
                }
            );
        }
        assert_eq!(state.effects.lock().await.len(), 1);

        let mut changed = request.clone();
        changed.payload = json!({"channel": "changed"});
        assert_eq!(
            client
                .prepare_dispatch(&key, &changed)
                .unwrap()
                .execute(&operation, test_permit())
                .await
                .unwrap(),
            DispatchOutcome::Conflict
        );
        assert_eq!(
            client
                .reconcile(&operation, test_permit(), &key, &request)
                .await
                .unwrap(),
            ReconcileOutcome::Accepted {
                provider_effect_id: format!("provider:{}", request.effect_id)
            }
        );

        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn response_body_streaming_remains_inside_the_operation_deadline() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 128\r\nconnection: close\r\n\r\n{",
                )
                .await
                .unwrap();
            stream.flush().await.unwrap();
            std::future::pending::<()>().await;
        });

        let client = ProviderClient::new(
            Url::parse(&format!("http://{address}/")).unwrap(),
            SecretString::new("fixture-secret".to_owned()),
        )
        .unwrap();
        let request = ProviderEffectRequest {
            version: EFFECT_PROTOCOL_VERSION,
            effect_id: Uuid::from_u128(21),
            owner_id: Uuid::from_u128(22),
            record_id: Uuid::from_u128(23),
            record_generation: 24,
            payload: json!({"channel": "stalled-body"}),
        };
        let operation = batter::operation::OperationOwner::new(Duration::from_millis(100))
            .unwrap()
            .into_context();
        let result = client
            .prepare_dispatch(&request.idempotency_key(), &request)
            .unwrap()
            .execute(&operation, test_permit())
            .await;
        assert!(matches!(
            result,
            Err(ProviderCallError::Interrupted(
                Interruption::DeadlineExceeded
            ))
        ));

        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }

    async fn dispatch_against_raw_response(
        response: Vec<u8>,
    ) -> Result<DispatchOutcome, ProviderCallError> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            stream.write_all(&response).await.unwrap();
        });
        let client = ProviderClient::new(
            Url::parse(&format!("http://{address}/")).unwrap(),
            SecretString::new("fixture-secret".to_owned()),
        )
        .unwrap();
        let request = ProviderEffectRequest {
            version: EFFECT_PROTOCOL_VERSION,
            effect_id: Uuid::from_u128(41),
            owner_id: Uuid::from_u128(42),
            record_id: Uuid::from_u128(43),
            record_generation: 44,
            payload: json!({"channel": "oversized"}),
        };
        let result = client
            .prepare_dispatch(&request.idempotency_key(), &request)
            .unwrap()
            .execute(
                &batter::operation::OperationOwner::new(Duration::from_secs(2))
                    .unwrap()
                    .into_context(),
                test_permit(),
            )
            .await;
        server.await.unwrap();
        result
    }

    #[tokio::test]
    async fn response_size_bound_rejects_declared_and_streamed_excess() {
        let declared = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            RESPONSE_MAX_BYTES + 1
        )
        .into_bytes();
        assert!(matches!(
            dispatch_against_raw_response(declared).await,
            Err(ProviderCallError::ResponseTooLarge)
        ));

        let mut streamed = b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n".to_vec();
        streamed.extend(std::iter::repeat_n(b'x', RESPONSE_MAX_BYTES + 1));
        assert!(matches!(
            dispatch_against_raw_response(streamed).await,
            Err(ProviderCallError::ResponseTooLarge)
        ));
    }
}
