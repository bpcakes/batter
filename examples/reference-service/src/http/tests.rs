use super::*;
use crate::config::ServingSettings;
use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{HeaderValue, Request as HttpRequest, header},
    routing::get,
};
use batter::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, ShutdownHandle, Supervisor},
    operation::OperationContext,
    settings::SettingsSource,
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    convert::Infallible,
    future::{Future, poll_fn},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    task::Poll,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Barrier,
    time::timeout,
};
use tower::ServiceExt;
use uuid::Uuid;

const AUTHENTICATED_OWNER: u128 = 1;

fn settings() -> ServingSettings {
    ServingSettings::from_sources(
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
                std::ffi::OsString::from(Uuid::from_u128(AUTHENTICATED_OWNER).to_string()),
            ),
            ("BATTER_AUTH_TOKEN".into(), "fake-token".into()),
            // Real-time boundary tests own tighter explicit wall-clock guards.
            // Keep the production operation budget from becoming their hidden
            // and load-sensitive failure condition.
            ("BATTER_REQUEST_TIMEOUT_MS".into(), "10000".into()),
        ])
        .unwrap(),
    )
    .unwrap()
}

fn production_app() -> Router {
    let settings = settings();
    let prepared_http = settings.prepare_http();
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let pool = settings
        .pool_options()
        .connect_lazy_with(settings.connect_options_from_process().unwrap());
    // Isolated business-route tests deliberately have no published health.
    let second = Duration::from_secs(1);
    let monitor = batter::health::HealthMonitor::new(
        batter::health::HealthPolicy::new(second, second, Duration::from_secs(3), second).unwrap(),
        || async { Ok::<_, Infallible>(()) },
    );
    router(
        prepared_http,
        handle.status(),
        handle.operation_admission(),
        pool,
        monitor.reader(),
    )
}

fn boundary_app(routes: Router, handle: &ShutdownHandle) -> Router {
    let prepared = settings().prepare_http();
    business_boundary(routes, &prepared, handle.operation_admission())
        .layer(middleware::from_fn(operational_http))
}

fn direct_request(
    method: &str,
    path: &str,
    body: Body,
    peer: IpAddr,
    authorization: Option<&str>,
    forged_headers: bool,
) -> HttpRequest<Body> {
    let mut builder = HttpRequest::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(value) = authorization {
        builder = builder.header(header::AUTHORIZATION, value);
    }
    if forged_headers {
        builder = builder
            .header("x-request-id", "forged-request-id")
            .header("forwarded", "for=198.51.100.21;proto=https")
            .header("x-forwarded-for", "198.51.100.22")
            .header("x-real-ip", "198.51.100.23")
            .header(
                "traceparent",
                "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            )
            .header("tracestate", "vendor=forged");
    }
    let mut request = builder.body(body).unwrap();
    if forged_headers {
        request.headers_mut().append(
            "x-request-id",
            HeaderValue::from_static("second-forged-request-id"),
        );
    }
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::new(peer, 40_000)));
    request
        .extensions_mut()
        .insert(OwnerId::new(Uuid::from_u128(999)).expect("forged owner fixture is non-nil"));
    request
}

async fn json_response(response: Response) -> (String, Value) {
    let request_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["request_id"], request_id);
    (request_id, value)
}

async fn assert_native_rejection(response: Response, expected_status: StatusCode) {
    assert_eq!(response.status(), expected_status);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/plain; charset=utf-8"
    );
    assert!(response.headers().contains_key("x-request-id"));
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    assert!(!body.is_empty());
    assert!(serde_json::from_slice::<Value>(&body).is_err());
}

#[tokio::test]
async fn probes_do_not_require_business_peer_metadata() {
    let response = production_app()
        .oneshot(
            HttpRequest::builder()
                .uri("/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().contains_key("x-request-id"));
}

#[tokio::test]
async fn native_json_rejection_stays_outside_the_application_problem_envelope() {
    let response = production_app()
        .oneshot(direct_request(
            "POST",
            "/records/00000000-0000-0000-0000-000000000002/deliveries",
            Body::from("{"),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            Some("Bearer fake-token"),
            false,
        ))
        .await
        .unwrap();

    assert_native_rejection(response, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn native_path_rejection_stays_outside_the_application_problem_envelope() {
    let response = production_app()
        .oneshot(direct_request(
            "GET",
            "/deliveries/not-a-uuid",
            Body::empty(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            Some("Bearer fake-token"),
            false,
        ))
        .await
        .unwrap();

    assert_native_rejection(response, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn native_body_limit_rejection_stays_outside_the_application_problem_envelope() {
    let response = production_app()
        .oneshot(direct_request(
            "POST",
            "/records/00000000-0000-0000-0000-000000000002/deliveries",
            Body::from(vec![b' '; REQUEST_BODY_MAX_BYTES + 1]),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            Some("Bearer fake-token"),
            false,
        ))
        .await
        .unwrap();

    assert_native_rejection(response, StatusCode::PAYLOAD_TOO_LARGE).await;
}

#[tokio::test]
async fn production_routes_require_auth_and_share_generated_identity_on_failures() {
    let peer = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 90));
    for authorization in [None, Some("Bearer wrong")] {
        let response = production_app()
            .oneshot(direct_request(
                "POST",
                "/records/00000000-0000-0000-0000-000000000002/deliveries",
                Body::from("{}"),
                peer,
                authorization,
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let (request_id, body) = json_response(response).await;
        assert_ne!(request_id, "forged-request-id");
        assert_eq!(body["code"], "authentication_required");
        assert!(!body.to_string().contains("forged"));
        assert!(!body.to_string().contains(&peer.to_string()));
    }

    let response = production_app()
        .oneshot(direct_request(
            "POST",
            "/records/00000000-0000-0000-0000-000000000002/deliveries",
            Body::from(r#"{"expected_generation":0,"idempotency_key":"key","payload":{}}"#),
            peer,
            Some("Bearer fake-token"),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let (request_id, body) = json_response(response).await;
    assert_ne!(request_id, "forged-request-id");
    assert_eq!(body["code"], "invalid_delivery_command");
    assert!(!body.to_string().contains(&peer.to_string()));
}

#[tokio::test]
async fn canonical_registration_supplies_native_peer_to_the_business_boundary() {
    let settings = settings();
    let prepared = settings.prepare_http();
    let pool = settings
        .pool_options()
        .connect_lazy_with(settings.connect_options_from_process().unwrap());
    let second = Duration::from_secs(1);
    let monitor = batter::health::HealthMonitor::new(
        batter::health::HealthPolicy::new(second, second, Duration::from_secs(3), second).unwrap(),
        || async { Ok::<_, Infallible>(()) },
    );
    let mut supervisor = supervisor();
    let lifecycle = supervisor.status();
    let admission = supervisor.operation_admission();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    register_in(
        &mut supervisor,
        listener,
        prepared,
        lifecycle,
        admission,
        pool,
        monitor.reader(),
    )
    .unwrap();
    let handle = supervisor.handle();
    let running = supervisor.start();
    timeout(Duration::from_secs(3), handle.status().wait_ready())
        .await
        .expect("registered HTTP task did not acknowledge startup")
        .unwrap();

    let mut socket = TcpStream::connect(address).await.unwrap();
    let client_ip = socket.local_addr().unwrap().ip();
    socket
        .write_all(
            b"GET /delivery-commands/transport-probe HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
    let mut response = String::new();
    timeout(Duration::from_secs(3), socket.read_to_string(&mut response))
        .await
        .expect("registered HTTP response did not complete")
        .unwrap();
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(headers.starts_with("HTTP/1.1 401 "), "{response}");
    let request_id = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("x-request-id")
                .then(|| value.trim())
        })
        .expect("response omitted x-request-id");
    let body: Value = serde_json::from_str(body).unwrap();
    assert_eq!(body["code"], "authentication_required");
    assert_eq!(body["request_id"], request_id);
    assert!(!body.to_string().contains(&client_ip.to_string()));

    handle.request();
    let report = timeout(Duration::from_secs(3), running.wait())
        .await
        .expect("registered HTTP task did not settle")
        .unwrap();
    assert!(report.is_success(), "{report:?}");
}

#[tokio::test]
async fn trusted_metadata_debug_redacts_peer_and_correlation() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let routes = Router::new().route(
        "/debug",
        get(
            |Extension(metadata): Extension<TrustedRequestMetadata>| async move {
                format!("{metadata:?}|{:?}", metadata.peer())
            },
        ),
    );
    let response = boundary_app(routes, &handle)
        .oneshot(direct_request(
            "GET",
            "/debug",
            Body::empty(),
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 90)),
            Some("Bearer fake-token"),
            false,
        ))
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(
        body,
        "TrustedRequestMetadata([REDACTED])|TrustedPeer([REDACTED])"
    );
}

#[tokio::test]
async fn concurrent_forged_metadata_cannot_replace_authority_peer_or_correlation() {
    const REQUESTS: usize = 12;
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let barrier = Arc::new(Barrier::new(REQUESTS));
    let routes = Router::new().route(
        "/inspect",
        get(
            move |Extension(owner): Extension<OwnerId>,
                  Extension(metadata): Extension<TrustedRequestMetadata>,
                  Extension(context): Extension<OperationContext>| {
                let barrier = barrier.clone();
                async move {
                    let nested_metadata = metadata.clone();
                    let nested = context
                        .run("request.metadata.inspect", move |_| async move {
                            barrier.wait().await;
                            Ok::<_, Infallible>((
                                nested_metadata.correlation_id().to_string(),
                                nested_metadata.peer().ip().to_string(),
                            ))
                        })
                        .await
                        .unwrap();
                    Json(json!({
                        "owner_id": owner.as_uuid(),
                        "peer_ip": metadata.peer().ip(),
                        "request_id": metadata.correlation_id().as_str(),
                        "nested_request_id": nested.0,
                        "nested_peer_ip": nested.1,
                    }))
                }
            },
        ),
    );
    let app = boundary_app(routes, &handle);
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..REQUESTS {
        let peer = IpAddr::V4(Ipv4Addr::new(192, 0, 2, index as u8 + 1));
        let app = app.clone();
        tasks.spawn(async move {
            let response = app
                .oneshot(direct_request(
                    "GET",
                    "/inspect",
                    Body::empty(),
                    peer,
                    Some("Bearer fake-token"),
                    true,
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let (request_id, body) = json_response(response).await;
            (peer, request_id, body)
        });
    }

    let mut request_ids = HashSet::new();
    timeout(Duration::from_secs(3), async {
        while let Some(result) = tasks.join_next().await {
            let (peer, request_id, body) = result.unwrap();
            assert!(request_ids.insert(request_id.clone()));
            assert_eq!(
                body["owner_id"],
                Uuid::from_u128(AUTHENTICATED_OWNER).to_string()
            );
            assert_eq!(body["peer_ip"], peer.to_string());
            assert_eq!(body["nested_peer_ip"], peer.to_string());
            assert_eq!(body["nested_request_id"], request_id);
            let text = body.to_string();
            let forged_owner = Uuid::from_u128(999).to_string();
            for forged in [
                "forged-request-id",
                "198.51.100.21",
                "198.51.100.22",
                "198.51.100.23",
                "4bf92f3577b34da6a3ce929d0e0e4736",
                forged_owner.as_str(),
            ] {
                assert!(!text.contains(forged), "{text}");
            }
        }
    })
    .await
    .expect("concurrent metadata requests did not all reach the barrier");
    assert_eq!(request_ids.len(), REQUESTS);
}

#[tokio::test]
async fn absent_optional_headers_work_but_absent_native_peer_fails_closed() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let routes = Router::new().route(
        "/inspect",
        get(
            |Extension(metadata): Extension<TrustedRequestMetadata>| async move {
                Json(json!({
                    "request_id": metadata.correlation_id().as_str(),
                    "peer_ip": metadata.peer().ip(),
                }))
            },
        ),
    );
    let app = boundary_app(routes, &handle);
    let response = app
        .clone()
        .oneshot(direct_request(
            "GET",
            "/inspect",
            Body::empty(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            Some("Bearer fake-token"),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let (_, body) = json_response(response).await;
    assert_eq!(body["peer_ip"], Ipv4Addr::LOCALHOST.to_string());

    let response = app
        .oneshot(
            HttpRequest::builder()
                .uri("/inspect")
                .header(header::AUTHORIZATION, "Bearer fake-token")
                .header("x-forwarded-for", "198.51.100.50")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let (_, body) = json_response(response).await;
    assert_eq!(body["code"], "internal_error");
    assert!(!body.to_string().contains("198.51.100.50"));
}

fn supervisor() -> Supervisor {
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second, second, second).unwrap();
    Supervisor::new(
        ShutdownBudget::new(Duration::from_millis(10), second, second, cleanup).unwrap(),
    )
}

#[tokio::test(start_paused = true)]
async fn cancellation_keeps_each_request_identity_and_authority_is_separate() {
    let mut supervisor = supervisor();
    supervisor
        .register("control", |startup| async move {
            let shutdown = startup.acknowledge_started();
            shutdown.cancelled().await;
            Ok(())
        })
        .unwrap();
    let handle = supervisor.handle();
    let running = supervisor.start();
    handle.status().wait_ready().await.unwrap();

    type Seen = (String, IpAddr, OwnerId, OperationContext);
    let seen: Arc<Mutex<Option<Seen>>> = Arc::new(Mutex::new(None));
    let saved = seen.clone();
    let routes = Router::new()
        .route(
            "/pending",
            get(
                move |Extension(owner): Extension<OwnerId>,
                      Extension(metadata): Extension<TrustedRequestMetadata>,
                      Extension(context): Extension<OperationContext>| {
                    let saved = saved.clone();
                    async move {
                        *saved.lock().unwrap() = Some((
                            metadata.correlation_id().to_string(),
                            metadata.peer().ip(),
                            owner,
                            context,
                        ));
                        std::future::pending::<Response>().await
                    }
                },
            ),
        )
        .route(
            "/complete",
            get(
                |Extension(metadata): Extension<TrustedRequestMetadata>,
                 Extension(context): Extension<OperationContext>| async move {
                    let nested = metadata.clone();
                    let request_id = context
                        .run("request.metadata.complete", move |_| async move {
                            Ok::<_, Infallible>(nested.correlation_id().to_string())
                        })
                        .await
                        .unwrap();
                    Json(json!({
                        "request_id": metadata.correlation_id().as_str(),
                        "nested_request_id": request_id,
                    }))
                },
            ),
        );
    let app = boundary_app(routes, &handle);
    let pending_peer = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 80));
    let mut pending = Box::pin(app.clone().oneshot(direct_request(
        "GET",
        "/pending",
        Body::empty(),
        pending_peer,
        Some("Bearer fake-token"),
        true,
    )));
    assert!(
        poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    let (pending_id, observed_peer, owner, pending_context) = seen.lock().unwrap().take().unwrap();
    assert_eq!(observed_peer, pending_peer);
    assert_eq!(owner.as_uuid(), Uuid::from_u128(AUTHENTICATED_OWNER));
    assert!(pending_context.check().is_ok());

    let complete = app
        .oneshot(direct_request(
            "GET",
            "/complete",
            Body::empty(),
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 81)),
            Some("Bearer fake-token"),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(complete.status(), StatusCode::OK);
    let (complete_id, complete_body) = json_response(complete).await;
    assert_ne!(pending_id, complete_id);
    assert_eq!(complete_body["nested_request_id"], complete_id);

    handle.request();
    let cancelled = pending.await.unwrap();
    assert_eq!(cancelled.status(), StatusCode::SERVICE_UNAVAILABLE);
    let (cancelled_id, body) = json_response(cancelled).await;
    assert_eq!(cancelled_id, pending_id);
    assert_eq!(body["code"], "operation_cancelled");
    assert!(pending_context.check().is_err());
    assert!(running.wait().await.unwrap().is_success());
}
