//! Lifecycle admission answers before trusted metadata, authentication and fallbacks.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const COMMAND_PATH: &str = "/delivery-commands/ordering-probe";

fn without_peer(mut request: HttpRequest<Body>) -> HttpRequest<Body> {
    request.extensions_mut().remove::<ConnectInfo<SocketAddr>>();
    request
}

/// Require the shared infrastructure envelope without an authentication challenge.
async fn assert_infrastructure(response: Response, status: StatusCode, code: &str) {
    assert_eq!(response.status(), status);
    assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
    let (request_id, body) = json_response(response).await;
    assert_ne!(request_id, "forged-request-id");
    assert_eq!(body["code"], code);
}

#[tokio::test]
async fn admission_rejects_before_trusted_metadata_or_authentication_runs() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    let app = production_app_with(&handle).await;
    let peer = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 70));
    // Once admitted, authentication answers the first request with 401 and the
    // metadata layer answers the peerless second with 500; the third reaches the
    // handler. While starting or draining, admission must answer all three first.
    let requests = || {
        [
            direct_request("GET", COMMAND_PATH, Body::empty(), peer, None, true),
            without_peer(direct_request(
                "GET",
                COMMAND_PATH,
                Body::empty(),
                peer,
                None,
                false,
            )),
            direct_request(
                "GET",
                COMMAND_PATH,
                Body::empty(),
                peer,
                Some("Bearer fake-token"),
                true,
            ),
        ]
    };
    for request in requests() {
        let response = app.request(request).await;
        assert_infrastructure(
            response,
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
        )
        .await;
    }

    approval.approve();
    let [unauthenticated, peerless, _] = requests();
    let response = app.request(unauthenticated).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
    let (_, body) = json_response(response).await;
    assert_eq!(body["code"], "authentication_required");
    let response = app.request(peerless).await;
    assert_infrastructure(
        response,
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
    )
    .await;

    handle.request();
    for request in requests() {
        let response = app.request(request).await;
        assert_infrastructure(
            response,
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
        )
        .await;
    }
}

#[tokio::test]
async fn admitted_authenticated_request_succeeds_until_drain() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let routes = GuardedRouter::new().route(
        "/admitted",
        get(
            move |Extension(owner): Extension<OwnerId>,
                  Extension(metadata): Extension<TrustedRequestMetadata>,
                  Extension(context): Extension<OperationContext>| {
                counted.fetch_add(1, Ordering::SeqCst);
                async move {
                    Json(json!({
                        "owner_id": owner.as_uuid(),
                        "peer_ip": metadata.peer().ip(),
                        "request_id": metadata.correlation_id().as_str(),
                        "admitted": context.check().is_ok(),
                    }))
                }
            },
        ),
    );
    let app = boundary_app(routes, &handle).await;
    let peer = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 71));
    let request = || {
        direct_request(
            "GET",
            "/admitted",
            Body::empty(),
            peer,
            Some("Bearer fake-token"),
            true,
        )
    };

    let response = app.request(request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let (request_id, body) = json_response(response).await;
    assert_ne!(request_id, "forged-request-id");
    assert_eq!(
        body["owner_id"],
        Uuid::from_u128(AUTHENTICATED_OWNER).to_string()
    );
    assert_eq!(body["peer_ip"], peer.to_string());
    assert_eq!(body["admitted"], true);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    handle.request();
    let response = app.request(request()).await;
    assert_infrastructure(
        response,
        StatusCode::SERVICE_UNAVAILABLE,
        "service_unavailable",
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn fallbacks_answer_only_after_admission() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    let app = production_app_with(&handle).await;
    let peer = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let requests = || {
        [
            // No route matches, so the guarded 404 fallback answers once admitted.
            direct_request("GET", "/missing", Body::empty(), peer, None, false),
            // A route matches without this method; its 405 follows authentication.
            direct_request(
                "DELETE",
                "/deliveries/00000000-0000-0000-0000-000000000003",
                Body::empty(),
                peer,
                Some("Bearer fake-token"),
                false,
            ),
        ]
    };
    for request in requests() {
        let response = app.request(request).await;
        assert_infrastructure(
            response,
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
        )
        .await;
    }

    approval.approve();
    for (request, status) in requests()
        .into_iter()
        .zip([StatusCode::NOT_FOUND, StatusCode::METHOD_NOT_ALLOWED])
    {
        let response = app.request(request).await;
        assert_eq!(response.status(), status);
        assert!(response.headers().contains_key("x-request-id"));
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert!(body.is_empty());
    }
    let unauthenticated_method = direct_request(
        "DELETE",
        "/deliveries/00000000-0000-0000-0000-000000000003",
        Body::empty(),
        peer,
        None,
        false,
    );
    let response = app.request(unauthenticated_method).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let (_, body) = json_response(response).await;
    assert_eq!(body["code"], "authentication_required");

    handle.request();
    for request in requests() {
        let response = app.request(request).await;
        assert_infrastructure(
            response,
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
        )
        .await;
    }
}
