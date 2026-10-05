//! A selected probe response policy covers every probe method, including the
//! method rejections no renderer sees, without changing status, admission,
//! correlation or observation, and without reaching anything but the probes.

use crate::capture::Capture;
use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, Method, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use batter_axum::{
    BrowserPolicy, GroupPolicy, GuardedRouter, HttpBoundary, InProcessClient, ProbePath,
    ReadinessPolicy, RequestPolicy, ResponseConstructionBudget, browser::PrivateResponsePolicy,
};
use batter_core::lifecycle::ShutdownHandle;
use std::time::Duration;

const SECOND: Duration = Duration::from_secs(1);
const PROBE_METHODS: [Method; 4] = [Method::GET, Method::HEAD, Method::POST, Method::OPTIONS];

/// Starting, ready and draining, in the order one process passes through them.
#[derive(Clone, Copy, Debug)]
enum Phase {
    Starting,
    Ready,
    Draining,
}

fn request_policy(handle: &ShutdownHandle) -> RequestPolicy {
    RequestPolicy::new(
        handle.operation_admission(),
        ResponseConstructionBudget::new(SECOND).unwrap(),
    )
    .with_infrastructure_json()
}

/// The boundary a private administrative surface assembles: a browser group for
/// its guarded routes and the same policy selected for its probes.
async fn private_boundary(
    handle: &ShutdownHandle,
    probe_policy: Option<PrivateResponsePolicy>,
    rendered: bool,
) -> InProcessClient {
    let private = PrivateResponsePolicy::NoReferrer;
    let group = GroupPolicy::browser(
        request_policy(handle),
        BrowserPolicy::without_mutation_checks(private),
    );
    let mut boundary = HttpBoundary::new(group);
    if let Some(policy) = probe_policy {
        boundary = boundary.with_probe_response_policy(policy);
    }
    let live = ProbePath::new("/live").unwrap();
    let ready = ProbePath::new("/ready").unwrap();
    let readiness = ReadinessPolicy::lifecycle_only(handle.status());
    boundary = if rendered {
        // A renderer applying the same policy itself must not duplicate headers.
        boundary
            .with_rendered_liveness(live, move |_| {
                let mut response = "live".into_response();
                private.apply(response.headers_mut());
                response
            })
            .unwrap()
            .with_rendered_readiness(ready, readiness, move |_, _| {
                let mut response = "ready".into_response();
                private.apply(response.headers_mut());
                response
            })
            .unwrap()
    } else {
        boundary
            .with_liveness(live)
            .unwrap()
            .with_readiness(ready, readiness)
            .unwrap()
    };
    boundary
        .assemble(GuardedRouter::new().route("/work", get(|| async { "work" })))
        .await
        .unwrap()
        .in_process()
}

fn probe(method: &Method, path: &str) -> Request<Body> {
    Request::builder()
        .method(method.clone())
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

fn assert_private(headers: &HeaderMap, referrer: &str) {
    assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{headers:?}");
    assert_eq!(headers["referrer-policy"], referrer, "{headers:?}");
    assert_eq!(headers["x-content-type-options"], "nosniff", "{headers:?}");
    // A renderer may apply the same policy; values are set, never appended.
    for name in ["cache-control", "referrer-policy", "x-content-type-options"] {
        assert_eq!(
            headers.get_all(name).iter().count(),
            1,
            "{name}: {headers:?}"
        );
    }
}

fn assert_not_private(headers: &HeaderMap) {
    assert!(!headers.contains_key(header::CACHE_CONTROL), "{headers:?}");
    assert!(!headers.contains_key("referrer-policy"), "{headers:?}");
    assert!(
        !headers.contains_key("x-content-type-options"),
        "{headers:?}"
    );
}

/// The status a probe answers with on its own, independent of this policy.
fn expected_status(method: &Method, path: &str, phase: Phase) -> StatusCode {
    if !matches!(*method, Method::GET | Method::HEAD) {
        return StatusCode::METHOD_NOT_ALLOWED;
    }
    match (path, phase) {
        ("/live", _) | ("/ready", Phase::Ready) => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

fn completions(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| line.contains("HTTP response boundary finished"))
        .collect()
}

async fn body_text(response: Response) -> String {
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Drive every method against both probes in one phase, returning the
/// completion-event count so exactly one observation per request is provable.
fn exercise(rendered: bool, phase: Phase) -> usize {
    let capture = Capture::new();
    let requests = capture.block_on(async move {
        let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
        if !matches!(phase, Phase::Starting) {
            approval.approve();
        }
        if matches!(phase, Phase::Draining) {
            handle.request();
        }
        let client =
            private_boundary(&handle, Some(PrivateResponsePolicy::NoReferrer), rendered).await;
        let mut count = 0;
        for path in ["/live", "/ready"] {
            for method in &PROBE_METHODS {
                let response = client.request(probe(method, path)).await;
                count += 1;
                assert_eq!(
                    response.status(),
                    expected_status(method, path, phase),
                    "{method} {path} {phase:?} rendered={rendered}"
                );
                // The boundary owns correlation for probes in every phase.
                assert_eq!(
                    response.headers()["x-request-id"].to_str().unwrap().len(),
                    36,
                    "{method} {path} {phase:?}"
                );
                assert_private(response.headers(), "no-referrer");
                if matches!(*method, Method::POST | Method::OPTIONS) {
                    // Status and method semantics are untouched: Axum's own
                    // rejection keeps its Allow header and empty body.
                    assert_eq!(response.headers()[header::ALLOW], "GET,HEAD");
                    assert!(body_text(response).await.is_empty());
                }
            }
        }
        count
    });
    let text = capture.text();
    let events = completions(&text);
    assert_eq!(events.len(), requests, "{text}");
    requests
}

#[test]
fn selected_policy_covers_every_probe_method_in_every_phase() {
    for rendered in [false, true] {
        for phase in [Phase::Starting, Phase::Ready, Phase::Draining] {
            assert_eq!(exercise(rendered, phase), 8);
        }
    }
}

#[tokio::test]
async fn probes_stay_outside_admission_while_guarded_work_is_rejected() {
    let handle = ShutdownHandle::new_unapproved();
    let client = private_boundary(&handle, Some(PrivateResponsePolicy::NoReferrer), false).await;

    // The probe answers its own 503 with an empty body, not the admission
    // envelope that a guarded route receives while unapproved.
    let ready = client.request(probe(&Method::GET, "/ready")).await;
    assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_private(ready.headers(), "no-referrer");
    assert!(body_text(ready).await.is_empty());

    let work = client.request(probe(&Method::GET, "/work")).await;
    assert_eq!(work.status(), StatusCode::SERVICE_UNAVAILABLE);
    let rejection = body_text(work).await;
    assert!(rejection.contains("service_unavailable"), "{rejection}");
}

#[tokio::test]
async fn an_unselected_policy_leaves_probe_responses_exactly_as_before() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let client = private_boundary(&handle, None, false).await;
    for method in &PROBE_METHODS {
        let response = client.request(probe(method, "/live")).await;
        assert_eq!(
            response.status(),
            expected_status(method, "/live", Phase::Ready)
        );
        assert_not_private(response.headers());
    }
}

#[tokio::test]
async fn the_selection_reaches_neither_guarded_routes_nor_unmatched_paths() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    // A guarded group without a browser policy, so any private header on a
    // guarded route or an unmatched path could only come from the probe
    // selection leaking out of the probe routers.
    let client = HttpBoundary::new(request_policy(&handle))
        .with_probe_response_policy(PrivateResponsePolicy::NoReferrer)
        .with_liveness(ProbePath::new("/live").unwrap())
        .unwrap()
        .assemble(GuardedRouter::new().route("/work", get(|| async { "work" })))
        .await
        .unwrap()
        .in_process();

    let live = client.request(probe(&Method::POST, "/live")).await;
    assert_eq!(live.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_private(live.headers(), "no-referrer");

    for path in ["/work", "/missing"] {
        let response = client.request(probe(&Method::GET, path)).await;
        assert_not_private(response.headers());
    }
}

#[tokio::test]
async fn the_last_selected_policy_wins() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let client = HttpBoundary::new(request_policy(&handle))
        .with_probe_response_policy(PrivateResponsePolicy::NoReferrer)
        .with_probe_response_policy(PrivateResponsePolicy::SameOriginReferrer)
        .with_liveness(ProbePath::new("/live").unwrap())
        .unwrap()
        .assemble(GuardedRouter::new())
        .await
        .unwrap()
        .in_process();
    let response = client.request(probe(&Method::OPTIONS, "/live")).await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_private(response.headers(), "same-origin");
}
