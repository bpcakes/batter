use super::*;
use batter::admission::{Admission, AdmissionError, Bulkhead, BulkheadCapacity, BulkheadError};
use serde_json::json;

fn capacity() -> Bulkhead {
    Bulkhead::new(BulkheadCapacity::new(1).unwrap())
}
async fn is_free(capacity: &Bulkhead) -> bool {
    let owner = batter::operation::OperationOwner::new(Duration::from_secs(1)).unwrap();
    capacity
        .run(
            owner.context(),
            Admission::Reject,
            "test.capacity",
            |_| async { Ok::<_, ()>(()) },
        )
        .await
        .is_ok()
}
async fn run_dispatch(
    capacity: &Bulkhead,
    context: &OperationContext,
    prepared: PreparedDispatch,
) -> Result<DispatchOutcome, ProviderCallError> {
    match capacity
        .run(
            context,
            Admission::Wait,
            "test.dispatch",
            |scope| async move { prepared.execute(&scope).await },
        )
        .await
    {
        Ok(outcome) => Ok(outcome),
        Err(BulkheadError::NotAdmitted(AdmissionError::Interrupted(_))) => {
            Ok(DispatchOutcome::KnownNotDispatched {
                retry_not_before: Duration::ZERO,
            })
        }
        Err(BulkheadError::NotAdmitted(error)) => panic!("unexpected refusal: {error:?}"),
        Err(BulkheadError::Operation(OperationError::Failed(error))) => Err(error),
        Err(BulkheadError::Operation(OperationError::Interrupted(reason))) => {
            Err(ProviderCallError::Interrupted(reason))
        }
    }
}

mod http2;
mod tracked;

#[path = "../../../../crates/batter-otlp/tests/support/client.rs"]
mod tls_fixture;

#[tokio::test]
async fn client_uses_explicit_tls_without_trusting_a_fixture_certificate() {
    tls_fixture::assert_bundled_tls(client_builder().build().unwrap()).await;
}

#[test]
fn invalid_retained_header_fails_before_dispatch() {
    let client = ProviderClient::new(
        Url::parse("http://127.0.0.1:0/").unwrap(),
        SecretString::new("fixture-secret".to_owned()),
    )
    .unwrap();
    assert!(
        matches!(client.prepare_dispatch("invalid\nkey", &request()), Err(error) if error.is_builder())
    );
}

#[test]
fn retry_policy_preserves_the_provider_lower_bound() {
    for milliseconds in [0, 25, MAX_ACCEPTED_RETRY_DELAY.as_millis() as u64] {
        let body =
            serde_json::to_vec(&json!({"code":"rate_limited", "retry_after_ms":milliseconds}))
                .unwrap();
        assert_eq!(
            classify_dispatch_body(StatusCode::TOO_MANY_REQUESTS, &body, &request()),
            DispatchOutcome::KnownNotDispatched {
                retry_not_before: Duration::from_millis(milliseconds)
            }
        );
    }
    let body =
        serde_json::to_vec(&json!({"code":"rate_limited", "retry_after_ms":u64::MAX})).unwrap();
    assert_eq!(
        classify_dispatch_body(StatusCode::TOO_MANY_REQUESTS, &body, &request()),
        DispatchOutcome::Indeterminate
    );
}

#[tokio::test]
async fn truncated_body_is_uncertain_and_exchange_releases_capacity() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        assert!(stream.read(&mut request).await.unwrap() > 0);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 128\r\nconnection: close\r\n\r\n{")
            .await
            .unwrap();
    });
    let client = ProviderClient::new(
        Url::parse(&format!("http://{address}/")).unwrap(),
        SecretString::new("fixture-secret".to_owned()),
    )
    .unwrap();
    let capacity = capacity();
    let request = request();
    let prepared = client
        .prepare_dispatch(&request.idempotency_key(), &request)
        .unwrap();
    let owner = batter::operation::OperationOwner::new(Duration::from_secs(2)).unwrap();
    let result = run_dispatch(&capacity, owner.context(), prepared).await;
    server.await.unwrap();
    let error = result.unwrap_err();
    assert!(matches!(error, ProviderCallError::Body(_)));
    assert_eq!(error.code(), "delivery.provider_body_failed");
    assert!(
        is_free(&capacity).await,
        "transport result retained the permit"
    );
}

#[test]
fn diagnostic_codes_separate_interruption_from_protocol_failure() {
    for (error, expected) in [
        (
            ProviderCallError::Interrupted(Interruption::DeadlineExceeded),
            "delivery.provider_deadline",
        ),
        (
            ProviderCallError::Interrupted(Interruption::Cancelled),
            "delivery.provider_cancelled",
        ),
        (
            ProviderCallError::ResponseTooLarge,
            "delivery.provider_response_too_large",
        ),
    ] {
        assert_eq!(error.code(), expected);
    }
}

#[tokio::test]
async fn native_transport_timeouts_keep_their_code_and_release_capacity() {
    for after_headers in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = ProviderClient {
            client: client_builder()
                .read_timeout(Duration::from_millis(200))
                .build()
                .unwrap(),
            base_url: Url::parse(&format!("http://{address}/")).unwrap(),
            token: SecretString::new("fixture-secret"),
        };
        let capacity = capacity();
        let request = request();
        let prepared = client
            .prepare_dispatch(&request.idempotency_key(), &request)
            .unwrap();
        let (release, released) = tokio::sync::oneshot::channel();
        let exchange = async {
            let owner = batter::operation::OperationOwner::new(Duration::from_secs(3)).unwrap();
            let result = run_dispatch(&capacity, owner.context(), prepared).await;
            let _ = release.send(());
            result
        };
        let (result, served) = tokio::join!(
            exchange,
            tokio::time::timeout(
                Duration::from_secs(4),
                stalled_response(
                    listener,
                    if after_headers {
                        StallPoint::Body
                    } else {
                        StallPoint::Headers
                    },
                    released
                )
            )
        );
        served.unwrap().unwrap();
        let error = result.unwrap_err();
        match (&error, after_headers) {
            (ProviderCallError::Send(cause), false) | (ProviderCallError::Body(cause), true) => {
                assert!(
                    cause.is_timeout(),
                    "must exercise reqwest's native timeout classifier"
                );
            }
            _ => panic!("unexpected timeout phase: {error:?}"),
        }
        assert_eq!(error.code(), "delivery.provider_timeout");
        assert!(
            is_free(&capacity).await,
            "timeout retained provider capacity"
        );
    }
}

async fn stalled_response(
    listener: tokio::net::TcpListener,
    after_headers: StallPoint,
    released: tokio::sync::oneshot::Receiver<()>,
) -> std::io::Result<()> {
    let after_headers = after_headers == StallPoint::Body;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut stream, _) = listener.accept().await?;
    let mut request = [0; 4096];
    assert!(stream.read(&mut request).await? > 0);
    if after_headers {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 128\r\nconnection: close\r\n\r\n{")
            .await?;
    }
    // Keep the socket open until the exchange has returned its native error.
    let _ = released.await;
    drop(stream);
    Ok(())
}

fn request() -> ProviderEffectRequest {
    ProviderEffectRequest {
        version: EFFECT_PROTOCOL_VERSION,
        effect_id: Uuid::from_u128(21),
        owner_id: Uuid::from_u128(22),
        record_id: Uuid::from_u128(23),
        record_generation: 24,
        payload: json!({"channel": "example"}),
    }
}

#[test]
fn dispatch_and_reconcile_require_the_same_canonical_acceptance_identity() {
    let expected = request();
    let exact = serde_json::to_vec(&json!({
        "effect_id": expected.effect_id,
        "provider_effect_id": "provider:exact",
        "request": expected,
    }))
    .unwrap();
    assert_eq!(
        classify_dispatch_body(StatusCode::OK, &exact, &expected),
        DispatchOutcome::Accepted {
            provider_effect_id: "provider:exact".to_owned(),
        }
    );
    assert_eq!(
        classify_reconcile(
            BufferedResponse {
                status: StatusCode::OK,
                body: exact,
            },
            &expected,
        ),
        ReconcileOutcome::Accepted {
            provider_effect_id: "provider:exact".to_owned(),
        }
    );

    let missing_echo = serde_json::to_vec(&json!({
        "effect_id": expected.effect_id,
        "provider_effect_id": "provider:missing",
    }))
    .unwrap();
    assert_eq!(
        classify_dispatch_body(StatusCode::OK, &missing_echo, &expected),
        DispatchOutcome::Indeterminate
    );
    assert_eq!(
        classify_reconcile(
            BufferedResponse {
                status: StatusCode::OK,
                body: missing_echo,
            },
            &expected,
        ),
        ReconcileOutcome::Unavailable
    );

    let mut changed = expected.clone();
    changed.payload = json!({"channel": "changed"});
    let mismatch = serde_json::to_vec(&json!({
        "effect_id": expected.effect_id,
        "provider_effect_id": "provider:mismatch",
        "request": changed,
    }))
    .unwrap();
    assert_eq!(
        classify_dispatch_body(StatusCode::OK, &mismatch, &expected),
        DispatchOutcome::Conflict
    );
    assert_eq!(
        classify_reconcile(
            BufferedResponse {
                status: StatusCode::OK,
                body: mismatch,
            },
            &expected,
        ),
        ReconcileOutcome::Conflict
    );
}

#[tokio::test]
async fn connector_failure_is_known_not_dispatched() {
    let client = ProviderClient::new(
        Url::parse("http://127.0.0.1:0/").unwrap(),
        SecretString::new("fixture-secret".to_owned()),
    )
    .unwrap();
    let request = request();
    assert_eq!(
        client
            .prepare_dispatch(&request.idempotency_key(), &request)
            .unwrap()
            .execute(
                &batter::operation::OperationOwner::new(Duration::from_secs(2))
                    .unwrap()
                    .into_context()
            )
            .await
            .unwrap(),
        DispatchOutcome::KnownNotDispatched {
            retry_not_before: Duration::ZERO,
        }
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StallPoint {
    Body,
    Headers,
}

#[tokio::test]
async fn consumer_gzip_feature_does_not_change_response_bytes() {
    tls_fixture::assert_no_decompression(
        client_builder().build().unwrap(),
        client_builder().gzip(true).build().unwrap(),
    )
    .await;
}
