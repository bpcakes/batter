use crate::capture::Capture;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::Request,
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use batter_axum::{
    GateView, Gated, GroupGate, GroupPolicy, GuardedRouter, HttpBoundary, HttpFailure,
    HttpObservationLevel, InProcessClient, ProbePath, RequestPolicy, ResponseConstructionBudget,
    RouteGroup, RouteInventory,
};
use batter_core::lifecycle::ShutdownHandle;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tracing::Level;

fn ready() -> ShutdownHandle {
    let (control, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    control
}
fn policy(control: &ShutdownHandle) -> RequestPolicy {
    RequestPolicy::new(
        control.operation_admission(),
        ResponseConstructionBudget::new(Duration::from_secs(5)).unwrap(),
    )
    .with_infrastructure_json()
}
fn request(method: Method, path: &str) -> Request {
    Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}
async fn text(response: Response) -> String {
    String::from_utf8(to_bytes(response.into_body(), 4096).await.unwrap().to_vec()).unwrap()
}
fn disabled() -> GroupGate {
    GroupGate::new(|| Gated::<()>::Disabled)
}
async fn disabled_response(app: &InProcessClient, method: Method, path: &str) {
    let response = app.request(request(method.clone(), path)).await;
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "{method} {path}"
    );
    assert_eq!(response.headers()["cache-control"], "no-store");
    let id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let body = text(response).await;
    if method != Method::HEAD {
        assert_eq!(
            body,
            format!(
                r#"{{"code":"disabled","message":"The service is temporarily disabled","request_id":"{id}"}}"#
            )
        );
    }
}

#[tokio::test]
async fn default_gate_covers_native_declared_and_all_guarded_fallbacks_method_blindly() {
    let control = ready();
    // The default Axum fallback and the explicit root/nested fallback paths
    // differ internally; exercise both, with and without declared dispatch.
    for declared in [false, true] {
        for custom_fallback in [false, true] {
            let calls = Arc::new(AtomicUsize::new(0));
            let gate = GroupGate::new({
                let calls = calls.clone();
                move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Gated::<()>::Disabled
                }
            });
            let routes = GuardedRouter::new().route("/work", get(|| async { "unreachable" }));
            let mut routes = if declared {
                routes.merge(GuardedRouter::from_router(
                    Router::new().route("/declared", get(|| async { "unreachable" })),
                    RouteInventory::new(["/declared"]).unwrap(),
                ))
            } else {
                routes
            };
            if custom_fallback {
                routes = routes.fallback(|| async { "unreachable" }).nest(
                    "/nested",
                    GuardedRouter::new().fallback(|| async { "unreachable" }),
                );
            }
            let app = HttpBoundary::new(GroupPolicy::gated(policy(&control), gate))
                .assemble(routes)
                .await
                .unwrap()
                .in_process();
            assert_eq!(calls.load(Ordering::SeqCst), 0, "assembly must not capture");
            let paths = if declared {
                vec!["/work", "/declared", "/missing", "/nested/missing"]
            } else {
                vec!["/work", "/missing", "/nested/missing"]
            };
            for path in &paths {
                for method in [
                    Method::GET,
                    Method::HEAD,
                    Method::POST,
                    Method::OPTIONS,
                    Method::from_bytes(b"EXTENSION").unwrap(),
                ] {
                    disabled_response(&app, method, path).await;
                }
            }
            assert_eq!(calls.load(Ordering::SeqCst), paths.len() * 5);
        }
    }
}

#[tokio::test]
async fn named_gate_does_not_reach_other_groups_probes_or_rendered_fallbacks() {
    use batter_axum::ReadinessPolicy;
    use batter_core::health::{HealthMonitor, HealthPolicy};
    let control = ready();
    let health = HealthMonitor::new(
        HealthPolicy::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(3),
            Duration::from_secs(1),
        )
        .unwrap(),
        || async { Ok::<_, std::convert::Infallible>(()) },
    );
    for gated_default in [false, true] {
        let base = if gated_default {
            GroupPolicy::gated(policy(&control), disabled())
        } else {
            GroupPolicy::new(policy(&control))
        };
        let app = HttpBoundary::new(base)
            .with_group(RouteGroup::new(
                "work",
                GroupPolicy::gated(policy(&control), disabled()),
                GuardedRouter::new().route("/gated", get(|| async { "unreachable" })),
            ))
            .unwrap()
            .with_liveness(ProbePath::new("/live").unwrap())
            .unwrap()
            .with_readiness(
                ProbePath::new("/ready").unwrap(),
                ReadinessPolicy::new(control.status(), health.reader()),
            )
            .unwrap()
            .with_rendered_fallback(|_| (StatusCode::NOT_FOUND, "missing").into_response())
            .unwrap()
            .assemble(GuardedRouter::new().route("/open", get(|| async { "open" })))
            .await
            .unwrap()
            .in_process();
        disabled_response(&app, Method::OPTIONS, "/gated").await;
        assert_eq!(
            app.request(request(Method::GET, "/live")).await.status(),
            StatusCode::OK
        );
        let readiness = app.request(request(Method::GET, "/ready")).await;
        assert_eq!(readiness.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(!text(readiness).await.contains("disabled"));
        assert_eq!(
            app.request(request(Method::POST, "/live")).await.status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            text(app.request(request(Method::GET, "/missing")).await).await,
            "missing"
        );
        if gated_default {
            disabled_response(&app, Method::GET, "/open").await;
        } else {
            assert_eq!(
                text(app.request(request(Method::GET, "/open")).await).await,
                "open"
            );
        }
    }
}

#[tokio::test]
async fn capture_is_once_and_two_extractions_keep_the_same_non_clone_value_during_engagement() {
    struct Frozen(usize);
    let control = ready();
    let engaged = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let gate = GroupGate::new({
        let engaged = engaged.clone();
        let calls = calls.clone();
        move || {
            let revision = calls.fetch_add(1, Ordering::SeqCst);
            if engaged.load(Ordering::SeqCst) {
                Gated::Disabled
            } else {
                Gated::Serve(Frozen(revision))
            }
        }
    });
    let routes = GuardedRouter::new().route(
        "/work",
        get({
            let entered = entered.clone();
            let release = release.clone();
            move |first: GateView<Frozen>, second: GateView<Frozen>| {
                let entered = entered.clone();
                let release = release.clone();
                async move {
                    assert!(std::ptr::eq(first.value(), second.value()));
                    entered.notify_one();
                    release.notified().await;
                    assert_eq!(first.value().0, 0);
                    first.value().0.to_string()
                }
            }
        }),
    );
    let app = HttpBoundary::new(GroupPolicy::gated(policy(&control), gate))
        .assemble(routes)
        .await
        .unwrap()
        .in_process();
    let first = tokio::spawn({
        let app = app.clone();
        async move { app.request(request(Method::GET, "/work")).await }
    });
    entered.notified().await;
    engaged.store(true, Ordering::SeqCst);
    disabled_response(&app, Method::GET, "/work").await;
    release.notify_one();
    assert_eq!(text(first.await.unwrap()).await, "0");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn absent_and_wrong_typed_views_are_sanitized_internal_failures() {
    for gate in [
        None,
        Some(GroupGate::new(|| Gated::Serve("sensitive-value"))),
    ] {
        let control = ready();
        let policy = match gate {
            None => GroupPolicy::new(policy(&control)),
            Some(gate) => GroupPolicy::gated(policy(&control), gate),
        };
        let app = HttpBoundary::new(policy)
            .assemble(
                GuardedRouter::new()
                    .route("/work", get(|_: GateView<u64>| async { "unreachable" })),
            )
            .await
            .unwrap()
            .in_process();
        let response = app.request(request(Method::GET, "/work")).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response.headers()["content-type"],
            "application/problem+json"
        );
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(
            text(response).await,
            r#"{"type":"about:blank","title":"Internal Server Error","status":500,"code":"internal_error"}"#
        );
    }
}

/// A handler that extracts a view no gate of its group captured gets the
/// sanitized 500, and its completion event names the wiring error, so it is
/// told apart from a server fault in the logs.
#[test]
fn an_unmatched_gate_view_is_named_in_the_completion_event() {
    for gated in [false, true] {
        let capture = Capture::new();
        capture.block_on(async {
            let control = ready();
            let group = if gated {
                GroupPolicy::gated(policy(&control), GroupGate::new(|| Gated::Serve("text")))
            } else {
                GroupPolicy::new(policy(&control))
            };
            let app = HttpBoundary::new(group)
                .assemble(
                    GuardedRouter::new()
                        .route("/work", get(|_: GateView<u64>| async { "unreachable" })),
                )
                .await
                .unwrap()
                .in_process();
            let response = app.request(request(Method::GET, "/work")).await;
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        });
        let logs = capture.text();
        let events: Vec<_> = logs
            .lines()
            .filter(|line| line.contains("HTTP response boundary finished"))
            .collect();
        assert_eq!(events.len(), 1, "{logs}");
        assert!(
            events[0].contains("gate_outcome=\"unmatched_view\""),
            "{}",
            events[0]
        );
        assert!(!logs.contains("text"), "the captured value is never logged");
    }
}

#[test]
fn disabled_severity_replaces_renderer_level_but_preserves_renderer_status() {
    for (level, filter, visible) in [
        (None, "info", true),
        (None, "info,batter=warn,batter::request=info", false),
        (
            Some(Level::WARN),
            "info,batter=warn,batter::request=info",
            true,
        ),
    ] {
        for status in [StatusCode::SERVICE_UNAVAILABLE, StatusCode::NOT_FOUND] {
            let capture = Capture::with_filter(filter);
            capture.block_on(async {
                let control = ready();
                let mut gate = disabled();
                if let Some(level) = level {
                    gate = gate.with_disabled_level(level);
                }
                let request_policy = policy(&control).with_failure_renderer(move |failure, _| {
                    assert_eq!(failure, HttpFailure::Disabled);
                    let mut response = (status, "disabled by policy").into_response();
                    response
                        .extensions_mut()
                        .insert(HttpObservationLevel(Level::ERROR));
                    response
                });
                let app = HttpBoundary::new(GroupPolicy::gated(request_policy, gate))
                    .assemble(GuardedRouter::new().route("/work", get(|| async { "unreachable" })))
                    .await
                    .unwrap()
                    .in_process();
                let response = app.request(request(Method::GET, "/work")).await;
                assert_eq!(response.status(), status);
                assert_eq!(
                    response.extensions().get::<HttpObservationLevel>(),
                    Some(&HttpObservationLevel(level.unwrap_or(Level::INFO)))
                );
                assert_eq!(text(response).await, "disabled by policy");
            });
            let logs = capture.text();
            let events: Vec<_> = logs
                .lines()
                .filter(|line| line.contains("HTTP response boundary finished"))
                .collect();
            assert_eq!(events.len(), usize::from(visible), "{logs}");
            if visible {
                let event = events[0];
                assert!(
                    event
                        .trim_start()
                        .starts_with(level.unwrap_or(Level::INFO).as_str()),
                    "{event}"
                );
                assert!(event.contains("gate_outcome=\"disabled\""), "{event}");
                assert!(
                    event.contains(&format!("status={}", status.as_u16())),
                    "{event}"
                );
                assert!(
                    event.contains(if status.is_server_error() {
                        "http_outcome=\"server_error\""
                    } else {
                        "http_outcome=\"client_error\""
                    }),
                    "{event}"
                );
            }
        }
    }
}

#[test]
fn gate_panics_propagate_with_one_dropped_observation_without_status_or_payload() {
    let capture = Capture::new();
    capture.block_on(async {
        let control = ready();
        let app = HttpBoundary::new(GroupPolicy::gated(
            policy(&control),
            GroupGate::new(|| -> Gated<()> { panic!("secret-gate-payload") }),
        ))
        .assemble(GuardedRouter::new().route("/work", get(|| async { "unreachable" })))
        .await
        .unwrap()
        .in_process();
        let result =
            tokio::spawn(async move { app.request(request(Method::GET, "/work")).await }).await;
        assert!(result.unwrap_err().is_panic());
    });
    let logs = capture.text();
    assert!(!logs.contains("secret-gate-payload"), "{logs}");
    let events: Vec<_> = logs
        .lines()
        .filter(|line| line.contains("HTTP response boundary finished"))
        .collect();
    assert_eq!(events.len(), 1, "{logs}");
    assert!(events[0].trim_start().starts_with("WARN"), "{logs}");
    assert!(events[0].contains("http_outcome=\"dropped\""), "{logs}");
    assert!(!events[0].contains("status="), "{logs}");
}

#[test]
fn disabled_renderer_panic_retains_gate_fact_without_completing_the_response() {
    let capture = Capture::new();
    capture.block_on(async {
        let control = ready();
        let request_policy = policy(&control).with_failure_renderer(|failure, _| {
            assert_eq!(failure, HttpFailure::Disabled);
            panic!("secret-renderer-payload")
        });
        let app = HttpBoundary::new(GroupPolicy::gated(
            request_policy,
            disabled().with_disabled_level(Level::ERROR),
        ))
        .assemble(GuardedRouter::new().route("/work", get(|| async { "unreachable" })))
        .await
        .unwrap()
        .in_process();
        let result =
            tokio::spawn(async move { app.request(request(Method::GET, "/work")).await }).await;
        assert!(result.unwrap_err().is_panic());
    });
    let logs = capture.text();
    let events: Vec<_> = logs
        .lines()
        .filter(|line| line.contains("HTTP response boundary finished"))
        .collect();
    assert_eq!(events.len(), 1, "{logs}");
    assert!(events[0].trim_start().starts_with("WARN"), "{logs}");
    assert!(events[0].contains("http_outcome=\"dropped\""), "{logs}");
    assert!(events[0].contains("gate_outcome=\"disabled\""), "{logs}");
    assert!(!events[0].contains("status="), "{logs}");
    assert!(!logs.contains("secret-renderer-payload"), "{logs}");
}
