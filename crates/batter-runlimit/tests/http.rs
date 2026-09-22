#[path = "http/dispatch_and_concurrency.rs"]
mod dispatch_and_concurrency;
#[path = "http/multi_policy.rs"]
mod multi_policy;
#[path = "http/response_contract.rs"]
mod response_contract;
mod support;

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::Request,
    http::{StatusCode, header},
    middleware,
    middleware::Next,
    routing::post,
};
use batter_runlimit::{
    http::{
        AuthInput, Authenticated, DirectPeer, HttpQuota, PreparedHttp, PublicProbePathError,
        PublicProbes,
    },
    quota::{ConsumptionError, Quota},
};
use runlimit_core::{
    BatchDecision, ConsumptionStatus, Denial, FixedWindowPolicy, KeyHasher, Limiter, QuotaMode,
};
use std::{
    convert::Infallible,
    future::Future,
    io::{self, Write},
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use support::*;
use tracing::instrument::WithSubscriber;

#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;

#[derive(Clone, Debug)]
struct Principal(&'static str);
#[derive(Debug, thiserror::Error)]
#[error("secret-authentication-cause")]
struct AuthError;

fn prepare<L>(
    quota: Quota<L>,
    policies: Vec<FixedWindowPolicy>,
    policy: batter_axum::RequestPolicy,
    routes: Router,
) -> PreparedHttp
where
    L: Limiter<Policy = FixedWindowPolicy> + 'static,
    L::CheckAllError: ConsumptionError,
{
    let hasher = KeyHasher::new([9; 32]).unwrap();
    HttpQuota::new(
        quota,
        policies,
        |input: AuthInput| async move {
            // Prove async authentication fits without a caller-owned middleware layer.
            tokio::task::yield_now().await;
            match input
                .headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
            {
                Some("Bearer secret-a") => Ok(Principal("owner-a")),
                Some("Bearer secret-b") => Ok(Principal("owner-b")),
                Some("Bearer secret-c") => Ok(Principal("owner-c")),
                _ => Err(AuthError),
            }
        },
        move |principal: &Principal, peer: DirectPeer, policy: &FixedWindowPolicy| {
            if policy.scope().as_str() == "peer" {
                hasher
                    .hash_for(policy, peer.ip().to_string())
                    .into_unbound_subject_key()
            } else {
                hasher
                    .hash_for(policy, principal.0)
                    .into_unbound_subject_key()
            }
        },
    )
    .unwrap()
    .with_public_probes(
        PublicProbes::new()
            .get("/live", || async { "live" })
            .unwrap(),
    )
    .prepare(policy, routes)
}

fn peer() -> SocketAddr {
    "127.0.0.1:12345".parse().unwrap()
}
fn request(uri: &str, auth: &str) -> Request {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::AUTHORIZATION, auth)
        .header("x-request-id", "secret-client-id")
        .body(Body::empty())
        .unwrap()
}
fn routes() -> Router {
    Router::new().route(
        "/work",
        post(|principal: Authenticated<Principal>| async move { principal.principal().0 }),
    )
}

#[derive(Clone)]
struct Buffer(Arc<Mutex<Vec<u8>>>);
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Capture {
    buffer: Buffer,
    dispatch: tracing::Dispatch,
}
impl Capture {
    fn new(filter: &str) -> Self {
        let buffer = Buffer(Arc::default());
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .without_time()
            .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
            .finish();
        Self {
            buffer,
            dispatch: test_dispatch::new(subscriber),
        }
    }
    async fn run<F: Future>(&self, future: F) -> F::Output {
        future.with_subscriber(self.dispatch.clone()).await
    }
    fn text(&self) -> String {
        String::from_utf8(self.buffer.0.lock().unwrap().clone()).unwrap()
    }
    fn events(&self) -> Vec<String> {
        self.text()
            .lines()
            .filter(|line| line.contains("HTTP response boundary finished"))
            .map(str::to_owned)
            .collect()
    }
}

struct CountBody {
    polls: Arc<AtomicUsize>,
    bytes: Option<Bytes>,
}
impl http_body::Body for CountBody {
    type Data = Bytes;
    type Error = Infallible;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, Infallible>>> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Ready(
            self.bytes
                .take()
                .map(|bytes| Ok(http_body::Frame::data(bytes))),
        )
    }
}

#[tokio::test]
async fn denied_request_does_not_poll_body_or_call_handler_and_identity_cannot_be_forged() {
    let running = running().await;
    let entered = Arc::new(AtomicUsize::new(0));
    let count = entered.clone();
    let routes = Router::new().route(
        "/work",
        post(move |principal: Authenticated<Principal>, body: Bytes| {
            count.fetch_add(1, Ordering::SeqCst);
            async move { format!("{}:{}", principal.principal().0, body.len()) }
        }),
    );
    let client = prepare(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        request_policy(&running, 1000),
        routes,
    )
    .in_process();
    let first = client
        .request(request("/work", "Bearer secret-a"), peer())
        .await;
    assert_eq!(first.status(), StatusCode::OK);
    let polls = Arc::new(AtomicUsize::new(0));
    let mut hostile = request("/work", "Bearer secret-a");
    hostile
        .headers_mut()
        .insert("forwarded", "for=198.51.100.77".parse().unwrap());
    hostile
        .headers_mut()
        .insert("x-forwarded-for", "198.51.100.77".parse().unwrap());
    hostile
        .headers_mut()
        .insert("x-owner", "owner-b".parse().unwrap());
    hostile.extensions_mut().insert(Principal("owner-b"));
    hostile
        .extensions_mut()
        .insert(axum::extract::ConnectInfo::<SocketAddr>(
            "198.51.100.77:99".parse().unwrap(),
        ));
    *hostile.body_mut() = Body::new(CountBody {
        polls: polls.clone(),
        bytes: Some(Bytes::from_static(b"secret-body")),
    });
    let denied = client.request(hostile, peer()).await;
    assert_eq!(denied.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(denied.headers().contains_key(header::RETRY_AFTER));
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert_eq!(entered.load(Ordering::SeqCst), 1);
    let other_owner = client
        .request(request("/work", "Bearer secret-b"), peer())
        .await;
    assert_eq!(other_owner.status(), StatusCode::OK);
    finish(running).await;
}

#[tokio::test(start_paused = true)]
async fn timeout_renderer_cannot_reclaim_writer_from_cloned_request_parts() {
    use axum::response::IntoResponse;
    use batter_axum::quota_observation::QuotaRecorder;

    let running = running().await;
    let capture = Capture::new("warn");
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_in_renderer = attempts.clone();
    let http_policy = request_policy(&running, 10).with_failure_renderer(move |failure, parts| {
        assert_eq!(failure, batter_axum::HttpFailure::DeadlineExceeded);
        attempts_in_renderer.fetch_add(1, Ordering::SeqCst);
        let mut request = Request::from_parts(parts.clone(), Body::empty());
        if let Some(writer) = QuotaRecorder::take(&mut request) {
            // Negative control: a reclaimed writer could erase a terminal fact.
            drop(writer.start());
        }
        failure.status().into_response()
    });
    let client = prepare(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        http_policy,
        Router::new().route(
            "/work",
            post(|| async { std::future::pending::<String>().await }),
        ),
    )
    .in_process();
    let response = capture
        .run(client.request(request("/work", "Bearer secret-a"), peer()))
        .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    let events = capture.events();
    assert_eq!(events.len(), 1, "{}", capture.text());
    assert!(
        events[0].contains("quota_outcome=\"allowed\""),
        "{}",
        events[0]
    );
    assert!(
        events[0].contains("quota_consumption=\"consumed\""),
        "{}",
        events[0]
    );
    finish(running).await;
}

#[tokio::test]
async fn peer_quota_uses_exact_transport_peer_not_forwarded_headers_or_port() {
    let running = running().await;
    let client = prepare(
        Quota::new(memory()),
        vec![policy("peer", 1)],
        request_policy(&running, 1000),
        routes(),
    )
    .in_process();
    assert_eq!(
        client
            .request(request("/work", "Bearer secret-a"), peer())
            .await
            .status(),
        StatusCode::OK
    );
    let mut forged = request("/work", "Bearer secret-b");
    forged
        .headers_mut()
        .insert("x-forwarded-for", "198.51.100.8".parse().unwrap());
    assert_eq!(
        client
            .request(forged, "127.0.0.1:55555".parse().unwrap())
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        client
            .request(
                request("/work", "Bearer secret-b"),
                "127.0.0.2:55555".parse().unwrap()
            )
            .await
            .status(),
        StatusCode::OK
    );
    finish(running).await;
}

#[tokio::test]
async fn all_response_paths_are_observed_once_and_secrets_are_absent() {
    let running = running().await;
    let capture = Capture::new("info");
    let client = prepare(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        request_policy(&running, 1000),
        routes(),
    )
    .in_process();
    let statuses = capture
        .run(async {
            let mut statuses = Vec::new();
            for (path, auth) in [
                ("/work", "invalid"),
                ("/work", "Bearer secret-a"),
                ("/work", "Bearer secret-a"),
                ("/missing", "invalid"),
            ] {
                let response = client.request(request(path, auth), peer()).await;
                assert_ne!(
                    response.headers().get("x-request-id").unwrap(),
                    "secret-client-id"
                );
                statuses.push(response.status().as_u16());
            }
            statuses.push(
                client
                    .request(
                        Request::builder().uri("/live").body(Body::empty()).unwrap(),
                        peer(),
                    )
                    .await
                    .status()
                    .as_u16(),
            );
            statuses
        })
        .await;
    assert_eq!(statuses, [401, 200, 429, 404, 200]);
    let events = capture.events();
    assert_eq!(events.len(), 5, "{}", capture.text());
    for (event, (status, quota)) in events.iter().zip([
        (401, "not_checked"),
        (200, "allowed"),
        (429, "quota_denied"),
        (404, "not_checked"),
        (200, "not_checked"),
    ]) {
        assert!(event.contains(&format!("status={status}")), "{event}");
        assert!(
            event.contains(&format!("quota_outcome=\"{quota}\"")),
            "{event}"
        );
    }
    assert!(!capture.text().contains("secret-"));
    assert!(!capture.text().contains("owner-a"));
    finish(running).await;
}

#[tokio::test(start_paused = true)]
async fn outer_timeout_preserves_consumed_quota_on_the_single_warn_completion() {
    let running = running().await;
    let capture = Capture::new("warn");
    let routes = Router::new().route(
        "/work",
        post(|| async { std::future::pending::<String>().await }),
    );
    let client = prepare(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        request_policy(&running, 10),
        routes,
    )
    .in_process();
    let response = capture
        .run(client.request(request("/work", "Bearer secret-a"), peer()))
        .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let events = capture.events();
    assert_eq!(events.len(), 1, "{}", capture.text());
    assert!(events[0].contains("quota_outcome=\"allowed\""));
    assert!(events[0].contains("quota_consumption=\"consumed\""));
    assert!(events[0].contains("status=503"));
    assert!(events[0].contains("request_id="));
    finish(running).await;
}

#[tokio::test(start_paused = true)]
async fn outer_timeout_during_check_reports_unknown_and_drops_native_future() {
    let running = running().await;
    let capture = Capture::new("info");
    let backend = Backend::new(Mode::Pending);
    let client = prepare(
        Quota::new(backend.clone()),
        vec![policy("owner", 1)],
        request_policy(&running, 10),
        routes(),
    )
    .in_process();
    let response = capture
        .run(client.request(request("/work", "Bearer secret-a"), peer()))
        .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let events = capture.events();
    assert_eq!(events.len(), 1);
    assert!(events[0].contains("quota_outcome=\"unresolved\""));
    assert!(events[0].contains("quota_consumption=\"unknown\""));
    assert_eq!(backend.dropped.load(Ordering::SeqCst), 1);
    finish(running).await;
}

#[tokio::test]
async fn dropping_response_future_retains_quota_facts_and_original_dispatch() {
    let running = running().await;
    let capture = Capture::new("warn");
    let backend = Backend::new(Mode::Pending);
    let client = prepare(
        Quota::new(backend.clone()),
        vec![policy("owner", 1)],
        request_policy(&running, 1000),
        routes(),
    )
    .in_process();
    let mut future = Box::pin(
        client
            .request(request("/work", "Bearer secret-a"), peer())
            .with_subscriber(capture.dispatch.clone()),
    );
    tokio::select! {
        _ = &mut future => panic!("backend should be pending"),
        _ = backend.entered.notified() => {}
    }
    drop(future);
    assert_eq!(backend.dropped.load(Ordering::SeqCst), 1);
    let events = capture.events();
    assert_eq!(events.len(), 1, "{}", capture.text());
    assert!(events[0].contains("http_outcome=\"dropped\""));
    assert!(events[0].contains("quota_consumption=\"unknown\""));
    finish(running).await;
}

#[tokio::test]
async fn shadow_and_storage_and_backend_outcomes_remain_distinct() {
    let running = running().await;
    let capture = Capture::new("info");
    let shadow_policy = policy("owner", 1).with_quota_mode(QuotaMode::Shadow);
    let client = prepare(
        Quota::new(memory()),
        vec![shadow_policy],
        request_policy(&running, 1000),
        routes(),
    )
    .in_process();
    let _ = client
        .request(request("/work", "Bearer secret-a"), peer())
        .await;
    let response = capture
        .run(client.request(request("/work", "Bearer secret-a"), peer()))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(capture.events()[0].contains("quota_outcome=\"shadow_denied\""));
    assert!(capture.events()[0].contains("quota_consumption=\"not_consumed\""));
    for (mode, expected) in [
        (
            Mode::Return(
                BatchDecision::denied(0, 1, Denial::StorageCapacity { retry_after: None }).unwrap(),
            ),
            "storage_capacity",
        ),
        (
            Mode::Fail(ConsumptionStatus::PossiblyConsumed),
            "backend_failed",
        ),
    ] {
        let capture = Capture::new("info");
        let client = prepare(
            Quota::new(Backend::new(mode)),
            vec![policy("owner", 1)],
            request_policy(&running, 1000),
            routes(),
        )
        .in_process();
        let response = capture
            .run(client.request(request("/work", "Bearer secret-a"), peer()))
            .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(!response.headers().contains_key(header::RETRY_AFTER));
        let retained = response.extensions().get::<Arc<BackendFailure>>();
        if expected == "backend_failed" {
            assert_eq!(
                retained.expect("concrete backend cause").0,
                ConsumptionStatus::PossiblyConsumed
            );
        } else {
            assert!(retained.is_none(), "capacity denial is not a backend error");
        }
        let body = String::from_utf8(to_bytes(response.into_body(), 4096).await.unwrap().to_vec())
            .unwrap();
        assert!(!body.contains("secret-"));
        let events = capture.events();
        assert_eq!(events.len(), 1);
        assert!(events[0].contains(&format!("quota_outcome=\"{expected}\"")));
        assert!(!capture.text().contains("secret-"));
    }
    finish(running).await;
}

#[tokio::test]
async fn draining_and_failed_authentication_do_not_charge_quota() {
    let running = running().await;
    let backend = Backend::new(Mode::Return(allowed()));
    let client = prepare(
        Quota::new(backend.clone()),
        vec![policy("owner", 1)],
        request_policy(&running, 1000),
        routes(),
    )
    .in_process();
    let rejected = client.request(request("/work", "invalid"), peer()).await;
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    assert!(rejected.extensions().get::<Arc<AuthError>>().is_some());
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    finish(running).await;
    assert_eq!(
        client
            .request(request("/work", "Bearer secret-a"), peer())
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn native_registration_installs_direct_peer_and_serves_with_owned_shutdown() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut supervisor = supervisor();
    let policy = batter_axum::RequestPolicy::new(
        supervisor.operation_admission(),
        batter_axum::ResponseConstructionBudget::new(Duration::from_secs(1)).unwrap(),
    );
    let prepared = prepare(
        Quota::new(memory()),
        vec![policy_for_wire()],
        policy,
        routes(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    prepared.register_in(&mut supervisor, listener).unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    socket.write_all(b"POST /work HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer secret-a\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(2), socket.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.ends_with("owner-a"), "{response}");
    finish(running).await;
}
fn policy_for_wire() -> FixedWindowPolicy {
    policy("peer", 1)
}

#[tokio::test(start_paused = true)]
async fn authentication_timeout_never_reaches_quota_or_handler() {
    let running = running().await;
    let capture = Capture::new("warn");
    let backend = Backend::new(Mode::Return(allowed()));
    let hasher = KeyHasher::new([9; 32]).unwrap();
    let client = HttpQuota::new(
        Quota::new(backend.clone()),
        vec![policy("owner", 1)],
        |_input: AuthInput| std::future::pending::<Result<Principal, AuthError>>(),
        move |principal: &Principal, _: DirectPeer, policy: &FixedWindowPolicy| {
            hasher
                .hash_for(policy, principal.0)
                .into_unbound_subject_key()
        },
    )
    .unwrap()
    .prepare(
        request_policy(&running, 10),
        Router::new().route(
            "/work",
            post(|| async {
                panic!("handler must not run");
                #[allow(unreachable_code)]
                StatusCode::OK
            }),
        ),
    )
    .in_process();
    let response = capture
        .run(client.request(request("/work", "Bearer secret-a"), peer()))
        .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    assert_eq!(capture.events().len(), 1);
    assert!(capture.events()[0].contains("quota_outcome=\"not_checked\""));
    finish(running).await;
}

#[tokio::test]
async fn forged_principal_cannot_claim_authentication_or_quota_execution() {
    let running = running().await;
    let capture = Capture::new("info");
    let client = prepare(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        request_policy(&running, 1000),
        routes(),
    )
    .in_process();
    let mut request = request("/work", "invalid");
    request.extensions_mut().insert(Principal("owner-a"));
    let response = capture.run(client.request(request, peer())).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(capture.events().len(), 1);
    assert!(capture.events()[0].contains("quota_outcome=\"not_checked\""));
    assert!(capture.events()[0].contains("quota_consumption=\"not_consumed\""));
    finish(running).await;
}

#[tokio::test]
async fn protected_routes_are_guarded_by_default_and_public_probes_are_explicit() {
    let running = running().await;
    let hasher = KeyHasher::new([9; 32]).unwrap();
    let client = HttpQuota::new(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        |input: AuthInput| async move {
            if input
                .headers
                .get(header::AUTHORIZATION)
                .is_some_and(|value| value == "Bearer secret-a")
            {
                Ok::<_, AuthError>(Principal("owner-a"))
            } else {
                Err(AuthError)
            }
        },
        move |principal: &Principal, _: DirectPeer, policy: &FixedWindowPolicy| {
            hasher
                .hash_for(policy, principal.0)
                .into_unbound_subject_key()
        },
    )
    .unwrap()
    .prepare(request_policy(&running, 1000), routes())
    .in_process();

    assert_eq!(
        client.request(request("/work", ""), peer()).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        client.request(request("/live", ""), peer()).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .request(request("/work", "Bearer secret-a"), peer())
            .await
            .status(),
        StatusCode::OK
    );
    finish(running).await;
}

#[tokio::test]
async fn inner_raw_principal_extension_cannot_replace_authenticated_extractor() {
    let running = running().await;
    let routes = routes().route_layer(middleware::from_fn(
        |mut request: Request, next: Next| async move {
            request.extensions_mut().insert(Principal("forged"));
            next.run(request).await
        },
    ));
    let client = prepare(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        request_policy(&running, 1000),
        routes,
    )
    .in_process();
    let response = client
        .request(request("/work", "Bearer secret-a"), peer())
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_bytes(response.into_body(), 64).await.unwrap(), "owner-a");
    finish(running).await;
}

#[tokio::test]
async fn handler_cannot_obtain_the_quota_observation_writer() {
    let running = running().await;
    let client = prepare(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        request_policy(&running, 1000),
        Router::new().route(
            "/work",
            post(|mut request: Request| async move {
                assert!(
                    batter_axum::quota_observation::QuotaRecorder::take(&mut request).is_none()
                );
                StatusCode::OK
            }),
        ),
    )
    .in_process();
    assert_eq!(
        client
            .request(request("/work", "Bearer secret-a"), peer())
            .await
            .status(),
        StatusCode::OK
    );
    finish(running).await;
}
