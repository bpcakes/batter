use crate::capture::Capture;
use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    http::{HeaderMap, HeaderValue, Request, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use batter_axum::{
    CorrelationId, GuardedRouter, HttpBoundary, HttpFailure, ProbePath, RequestPolicy,
    ResponseConstructionBudget, liveness, observe_http, operational_http,
    operational_http_with_quota,
    quota_observation::{QuotaRecorder, QuotaTerminalFacts},
    render_infrastructure_failure, request_admission, request_scope,
};
use batter_core::{lifecycle::ShutdownHandle, operation::OperationContext};
use std::{
    collections::HashSet,
    future::{Future, poll_fn},
    sync::{Arc, Mutex},
    task::Poll,
    time::Duration,
};
use tokio::sync::Barrier;
use tower::ServiceExt;
use tower_http::request_id::RequestId;
use tracing::instrument::WithSubscriber;

fn request(path: &str) -> Request<Body> {
    let mut request = Request::builder()
        .uri(path)
        .header("x-request-id", "secret-forged-header")
        .header("authorization", "secret-credential")
        .body(Body::empty())
        .unwrap();
    request.headers_mut().append(
        "x-request-id",
        HeaderValue::from_static("secret-second-header"),
    );
    request
        .extensions_mut()
        .insert(RequestId::new(HeaderValue::from_static(
            "secret-forged-extension",
        )));
    request
}

fn boundary(app: Router, handle: ShutdownHandle) -> Router {
    app.route_layer(middleware::from_fn_with_state(
        RequestPolicy::new(
            handle.operation_admission(),
            ResponseConstructionBudget::new(Duration::from_secs(2)).unwrap(),
        )
        .with_infrastructure_json(),
        request_admission,
    ))
    .route("/live", get(liveness))
    .fallback(|| async { StatusCode::NOT_FOUND })
    .layer(middleware::from_fn(operational_http))
}

async fn id_and_body(response: Response) -> (String, String) {
    let id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    (id, String::from_utf8(body.to_vec()).unwrap())
}

fn completion_fields(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|line| {
            line.split_once("HTTP response boundary finished")
                .map(|(_, fields)| fields)
        })
        .collect()
}

#[test]
fn concurrent_requests_replace_forged_identity_and_agree_with_body_and_nested_operations() {
    let capture = Capture::new();
    let ids = capture.block_on(async {
        let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
        approval.approve();
        let barrier = Arc::new(Barrier::new(16));
        let app = boundary(Router::new().route("/work", get(move |
            headers: HeaderMap,
            Extension(id): Extension<CorrelationId>,
            Extension(native): Extension<RequestId>,
            Extension(context): Extension<OperationContext>,
        | {
            let barrier = barrier.clone();
            async move {
                assert_eq!(headers.get_all("x-request-id").iter().count(), 1);
                assert_eq!(headers["x-request-id"], *native.header_value());
                assert_eq!(id.as_str(), native.header_value().to_str().unwrap());
                context.run("nested.read", |_| async {
                    barrier.wait().await;
                    tracing::info!(request_id = %id, "nested identity");
                    Ok::<_, std::convert::Infallible>(())
                }).await.unwrap();
                let mut response = render_infrastructure_failure(HttpFailure::Internal, Some(&id));
                // A response rewriter must not be able to replace the outer ID.
                response.headers_mut().insert("x-request-id", HeaderValue::from_static("secret-response"));
                response
            }
        })), handle);
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..16 {
            tasks.spawn(app.clone().oneshot(request("/work")));
        }
        let mut ids = HashSet::new();
        while let Some(result) = tasks.join_next().await {
            let response = result.unwrap().unwrap();
            assert_eq!(response.headers().get_all("x-request-id").iter().count(), 1);
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(response.headers()["cache-control"], "no-store");
            let (id, body) = id_and_body(response).await;
            assert_eq!(body, format!(r#"{{"code":"internal_error","message":"An internal error occurred","request_id":"{id}"}}"#));
            assert!(ids.insert(id));
        }
        ids
    });
    let text = capture.text();
    assert!(!text.contains("secret-"), "{text}");
    let fields = completion_fields(&text);
    assert_eq!(fields.len(), 16, "{text}");
    for id in ids {
        assert_eq!(
            fields
                .iter()
                .filter(|line| line.contains(&format!("request_id=\"{id}\"")))
                .count(),
            1
        );
        assert!(
            text.lines()
                .any(|line| line.contains("nested identity") && line.contains(&id))
        );
        assert!(
            text.lines()
                .any(|line| line.contains("operation boundary finished")
                    && line.contains("nested.read")
                    && line.contains(&id))
        );
    }
}

#[test]
fn assembled_routes_probes_405_fallback_and_admission_emit_once_with_sanitized_fields() {
    let capture = Capture::new();
    let observations = capture.block_on(async {
        let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
        let app = boundary(
            Router::new().route("/work", get(|| async { "ok" })),
            handle.clone(),
        );
        let mut observations = Vec::new();
        for (path, method, status, route) in [
            ("/work", "GET", 503, "/work"),
            ("/live", "GET", 200, "/live"),
            ("/secret-path?secret-query", "GET", 404, "<unmatched>"),
            ("/live", "POST", 405, "/live"),
        ] {
            let mut request = request(path);
            *request.method_mut() = method.parse().unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status().as_u16(), status);
            let (id, body) = id_and_body(response).await;
            if status == 503 {
                assert!(body.contains(&format!("\"request_id\":\"{id}\"")));
            }
            observations.push((id, status, route, method));
        }
        approval.approve();
        let response = app.oneshot(request("/work")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let (id, body) = id_and_body(response).await;
        assert_eq!(body, "ok");
        observations.push((id, 200, "/work", "GET"));
        observations
    });
    let text = capture.text();
    assert!(!text.contains("secret-"), "{text}");
    let fields = completion_fields(&text);
    assert_eq!(fields.len(), observations.len(), "{text}");
    for (id, status, route, method) in observations {
        let matching: Vec<_> = fields
            .iter()
            .filter(|line| line.contains(&format!("request_id=\"{id}\"")))
            .collect();
        assert_eq!(matching.len(), 1, "{text}");
        let event = matching[0];
        for expected in [
            format!("status={status}"),
            format!("route=\"{route}\""),
            format!("method=\"{method}\""),
            "latency_ms=".into(),
        ] {
            assert!(event.contains(&expected), "{event}");
        }
        let outcome = if status >= 500 {
            "server_error"
        } else if status >= 400 {
            "client_error"
        } else {
            "completed"
        };
        assert!(event.contains(&format!("http_outcome=\"{outcome}\"")));
    }
}

#[test]
fn replayed_adapter_extension_is_replaced_and_custom_renderer_remains_in_control() {
    Capture::new().block_on(async {
        let seed: Router = Router::new()
            .route(
                "/",
                get(|Extension(id): Extension<CorrelationId>| async { (Extension(id), "ok") }),
            )
            .layer(middleware::from_fn(operational_http));
        let response = seed.oneshot(request("/")).await.unwrap();
        let old = response
            .extensions()
            .get::<CorrelationId>()
            .unwrap()
            .clone();
        let old_id = old.as_str().to_owned();
        let policy = RequestPolicy::new(
            ShutdownHandle::new_unapproved().operation_admission(),
            ResponseConstructionBudget::new(Duration::from_secs(1)).unwrap(),
        )
        .with_infrastructure_json()
        .with_failure_renderer(|_, parts| {
            let id = parts.extensions.get::<CorrelationId>().unwrap();
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("custom", "retained")],
                id.to_string(),
            )
                .into_response()
        });
        let app: Router = Router::new()
            .route("/", get(|| async { StatusCode::IM_A_TEAPOT }))
            .route_layer(middleware::from_fn_with_state(policy, request_admission))
            .layer(middleware::from_fn(operational_http));
        let mut request = request("/");
        request.extensions_mut().insert(old);
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()["custom"], "retained");
        let (id, body) = id_and_body(response).await;
        assert_eq!(id, body);
        assert_ne!(id, old_id);
    });
}

#[tokio::test]
async fn dropped_request_retains_event_local_id_with_info_disabled_under_another_dispatch() {
    // Keep all registries strongly owned across repeated cases, not just the active poll.
    let captures: Vec<_> = (0..8)
        .map(|_| Capture::with_max_level(tracing::Level::WARN))
        .collect();
    let other = Capture::new();
    for capture in &captures {
        let seen = Arc::new(Mutex::new(None));
        let saved = seen.clone();
        let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
        approval.approve();
        let app = boundary(
            Router::new().route(
                "/work",
                get(
                    move |Extension(id): Extension<CorrelationId>,
                          Extension(context): Extension<OperationContext>| async move {
                        *saved.lock().unwrap() = Some((id, context));
                        std::future::pending::<Response>().await
                    },
                ),
            ),
            handle,
        );
        let future = app.oneshot(request("/work"));
        let mut future = Box::pin(future);
        assert!(
            poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx)))
                .with_subscriber(capture.dispatch.clone())
                .await
                .is_pending()
        );
        let (id, context) = seen.lock().unwrap().take().unwrap();
        assert!(context.check().is_ok());
        tracing::dispatcher::with_default(&other.dispatch, || drop(future));
        assert!(context.check().is_err());
        let text = capture.text();
        let fields = completion_fields(&text);
        assert_eq!(fields.len(), 1, "{text}");
        assert!(
            fields[0].contains(&format!("request_id=\"{id}\"")),
            "{text}"
        );
        assert!(fields[0].contains("http_outcome=\"dropped\""));
        assert!(!fields[0].contains("status="));
        assert!(
            !text.contains("batter.request{"),
            "INFO span unexpectedly enabled: {text}"
        );
        assert!(!text.contains("secret-"));
    }
    assert!(other.text().is_empty(), "{}", other.text());
}

#[tokio::test(start_paused = true)]
async fn deadline_failure_uses_generated_id_even_when_info_spans_are_disabled() {
    let capture = Capture::with_max_level(tracing::Level::WARN);
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let app = boundary(
        Router::new().route("/work", get(std::future::pending::<Response>)),
        handle,
    );
    let response = app
        .oneshot(request("/work"))
        .with_subscriber(capture.dispatch.clone())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let (id, body) = id_and_body(response).await;
    assert_eq!(
        body,
        format!(
            r#"{{"code":"deadline_exceeded","message":"The operation exceeded its time budget","request_id":"{id}"}}"#
        )
    );
    let text = capture.text();
    let fields = completion_fields(&text);
    assert_eq!(fields.len(), 1);
    assert!(fields[0].contains(&format!("request_id=\"{id}\"")));
    assert!(fields[0].contains("status=503"));
}

#[test]
fn manual_request_scope_inside_operational_retains_identity_on_every_short_circuit() {
    let capture = Capture::new();
    let responses = capture.block_on(async {
        let starting = ShutdownHandle::new_unapproved();
        let starting_policy = RequestPolicy::new(
            starting.operation_admission(),
            ResponseConstructionBudget::new(Duration::from_secs(1)).unwrap(),
        )
        .with_infrastructure_json();
        let starting_app = Router::new()
            .route("/work", get(|| async { "unreachable" }))
            .route_layer(middleware::from_fn_with_state(
                starting_policy,
                request_scope,
            ))
            .layer(middleware::from_fn(operational_http));
        let starting = starting_app.oneshot(request("/work")).await.unwrap();
        assert_eq!(starting.status(), StatusCode::SERVICE_UNAVAILABLE);
        let starting = id_and_body(starting).await;

        let (ready, approval) = ShutdownHandle::new_with_readiness_approval();
        approval.approve();
        let deadline_policy = RequestPolicy::new(
            ready.operation_admission(),
            ResponseConstructionBudget::new(Duration::from_millis(10)).unwrap(),
        )
        .with_infrastructure_json();
        let deadline_app = Router::new()
            .route("/work", get(std::future::pending::<Response>))
            .route_layer(middleware::from_fn_with_state(
                deadline_policy,
                request_scope,
            ))
            .layer(middleware::from_fn(operational_http));
        let deadline = deadline_app.oneshot(request("/work")).await.unwrap();
        assert_eq!(deadline.status(), StatusCode::SERVICE_UNAVAILABLE);
        let deadline = id_and_body(deadline).await;
        [starting, deadline]
    });

    for ((id, body), code) in responses
        .iter()
        .zip(["service_unavailable", "deadline_exceeded"])
    {
        assert!(body.contains(&format!(r#""code":"{code}""#)), "{body}");
        assert!(body.contains(&format!(r#""request_id":"{id}""#)), "{body}");
    }
    let text = capture.text();
    let fields = completion_fields(&text);
    assert_eq!(fields.len(), 2, "{text}");
    for (id, _) in responses {
        assert_eq!(
            fields
                .iter()
                .filter(|event| event.contains(&format!("request_id=\"{id}\"")))
                .count(),
            1,
            "{text}"
        );
    }
}

#[tokio::test]
async fn missing_typed_correlation_never_falls_back_to_untrusted_headers() {
    let policy = RequestPolicy::new(
        ShutdownHandle::new_unapproved().operation_admission(),
        ResponseConstructionBudget::new(Duration::from_secs(1)).unwrap(),
    )
    .with_infrastructure_json();
    let app: Router = Router::new()
        .route("/", get(|| async { StatusCode::IM_A_TEAPOT }))
        .layer(middleware::from_fn_with_state(policy, request_admission));
    let response = app.oneshot(request("/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["content-type"], "application/json");
    assert!(!response.headers().contains_key("x-request-id"));
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(body.as_ref(), br#"{"code":"service_unavailable","message":"The service is unavailable","request_id":null}"#);
}

#[tokio::test(start_paused = true)]
async fn forced_process_cancellation_preserves_handler_body_header_and_event_identity() {
    use batter_core::{
        cleanup::CleanupBudget,
        lifecycle::{ShutdownBudget, Supervisor},
    };
    let capture = Capture::with_max_level(tracing::Level::WARN);
    let cleanup = CleanupBudget::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap();
    let budget = ShutdownBudget::new(
        Duration::from_millis(10),
        Duration::from_secs(1),
        Duration::from_secs(1),
        cleanup,
    )
    .unwrap();
    let mut supervisor = Supervisor::new(budget);
    let handle = supervisor.handle();
    supervisor
        .register("control", |startup| async move {
            let signal = startup.acknowledge_started();
            signal.cancelled().await;
            Ok(signal.stopped())
        })
        .unwrap();
    let running = supervisor.start();
    handle.status().wait_ready().await.unwrap();
    let seen = Arc::new(Mutex::new(None));
    let saved = seen.clone();
    let app = boundary(
        Router::new().route(
            "/work",
            get(move |Extension(id): Extension<CorrelationId>| async move {
                *saved.lock().unwrap() = Some(id);
                std::future::pending::<Response>().await
            }),
        ),
        handle.clone(),
    );
    let mut response = Box::pin(app.oneshot(request("/work")));
    assert!(
        poll_fn(|cx| Poll::Ready(response.as_mut().poll(cx)))
            .with_subscriber(capture.dispatch.clone())
            .await
            .is_pending()
    );
    let expected = seen.lock().unwrap().take().unwrap();
    handle.request();
    let response = response
        .with_subscriber(capture.dispatch.clone())
        .await
        .unwrap();
    let report = running.wait().await.unwrap();
    assert!(report.is_success());
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let (id, body) = id_and_body(response).await;
    assert_eq!(id, expected.as_str());
    assert_eq!(
        body,
        format!(
            r#"{{"code":"operation_cancelled","message":"The operation was cancelled","request_id":"{id}"}}"#
        )
    );
    let text = capture.text();
    let fields = completion_fields(&text);
    assert_eq!(fields.len(), 1);
    assert!(fields[0].contains(&format!("request_id=\"{id}\"")));
    assert!(fields[0].contains("status=503"));
}

#[test]
fn request_context_target_keeps_nested_events_without_enabling_operation_info() {
    let capture = Capture::with_filter("info,batter=warn,batter::request=info");
    let id = capture.block_on(async {
        let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
        approval.approve();
        let app = boundary(
            Router::new().route(
                "/work",
                get(
                    |Extension(context): Extension<OperationContext>| async move {
                        context
                            .run("nested.success", |_| async { Ok::<_, std::io::Error>(()) })
                            .await
                            .unwrap();
                        let result = context
                            .run("nested.failure", |_| async {
                                tracing::info!("application progress");
                                tracing::warn!("application warning");
                                Err::<(), _>(std::io::Error::other("secret-failure"))
                            })
                            .await;
                        assert!(result.is_err());
                        StatusCode::INTERNAL_SERVER_ERROR
                    },
                ),
            ),
            handle,
        );
        let response = app.oneshot(request("/work")).await.unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        id_and_body(response).await.0
    });
    let text = capture.text();
    for message in [
        "application progress",
        "application warning",
        "operation boundary finished",
        "HTTP response boundary finished",
    ] {
        let events: Vec<_> = text.lines().filter(|line| line.contains(message)).collect();
        assert_eq!(events.len(), 1, "{text}");
        assert!(
            events[0].contains(&format!("request_id={id}")),
            "{}",
            events[0]
        );
    }
    assert!(
        text.lines()
            .filter(|line| line.contains("operation boundary finished"))
            .all(|line| line.trim_start().starts_with("WARN") && line.contains("failed")),
        "{text}"
    );
    assert!(
        !text.contains("batter.operation") && !text.contains("secret-"),
        "{text}"
    );
}

#[test]
fn never_polled_operational_entry_does_no_application_work_or_observation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    let capture = Capture::new();
    let status = capture.block_on(async {
        let app = Router::new()
            .route(
                "/work",
                get(move || {
                    called.fetch_add(1, Ordering::SeqCst);
                    async { "work" }
                }),
            )
            .layer(middleware::from_fn(|request, next| async {
                // Construct the public entry itself, then destroy it without polling.
                drop(operational_http(request, next));
                StatusCode::NO_CONTENT
            }));
        app.oneshot(request("/work")).await.unwrap().status()
    });
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(capture.text().is_empty(), "{}", capture.text());
}

#[tokio::test]
async fn canonical_boundary_gates_default_custom_and_method_fallbacks_but_not_probes() {
    for phase in ["starting", "ready", "draining"] {
        let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
        if phase != "starting" {
            approval.approve();
        }
        if phase == "draining" {
            handle.request();
        }
        let policy = || {
            RequestPolicy::new(
                handle.operation_admission(),
                ResponseConstructionBudget::new(Duration::from_secs(1)).unwrap(),
            )
        };
        let default = HttpBoundary::new(policy())
            .with_liveness(ProbePath::new("/live").unwrap())
            .unwrap()
            .assemble(GuardedRouter::new().route("/work", get(|| async { "ok" })))
            .await
            .unwrap()
            .into_router();
        let custom = HttpBoundary::new(policy())
            .with_liveness(ProbePath::new("/live").unwrap())
            .unwrap()
            .assemble(
                GuardedRouter::new()
                    .route("/work", get(|| async { "ok" }))
                    .fallback(|| async { StatusCode::IM_A_TEAPOT }),
            )
            .await
            .unwrap()
            .into_router();
        let nested = HttpBoundary::new(policy())
            .with_liveness(ProbePath::new("/live").unwrap())
            .unwrap()
            .assemble(
                GuardedRouter::new().nest(
                    "/nested",
                    GuardedRouter::new()
                        .route("/work", get(|| async { "ok" }))
                        .fallback(|| async { StatusCode::IM_A_TEAPOT }),
                ),
            )
            .await
            .unwrap()
            .into_router();

        for router in [&default, &custom] {
            assert_eq!(
                router
                    .clone()
                    .oneshot(request("/live"))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::OK,
                "probe was gated while {phase}"
            );
            let mut method = request("/work");
            *method.method_mut() = axum::http::Method::POST;
            let expected = if phase == "ready" {
                StatusCode::METHOD_NOT_ALLOWED
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            assert_eq!(
                router.clone().oneshot(method).await.unwrap().status(),
                expected,
                "method fallback escaped admission while {phase}"
            );
        }

        let default_missing = if phase == "ready" {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };
        let custom_missing = if phase == "ready" {
            StatusCode::IM_A_TEAPOT
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };
        assert_eq!(
            default.oneshot(request("/missing")).await.unwrap().status(),
            default_missing,
            "default fallback escaped admission while {phase}"
        );
        assert_eq!(
            custom.oneshot(request("/missing")).await.unwrap().status(),
            custom_missing,
            "custom fallback escaped admission while {phase}"
        );
        assert_eq!(
            nested
                .oneshot(request("/nested/missing"))
                .await
                .unwrap()
                .status(),
            custom_missing,
            "nested fallback escaped admission while {phase}"
        );
    }
}

#[test]
fn outer_observer_does_not_suppress_inner_operational_identity() {
    let capture = Capture::new();
    let (id, body) = capture.block_on(async {
        let app = Router::new()
            .route(
                "/work",
                get(
                    |headers: HeaderMap,
                     Extension(id): Extension<CorrelationId>,
                     Extension(native): Extension<RequestId>| async move {
                        assert_eq!(headers.get_all("x-request-id").iter().count(), 1);
                        assert_eq!(headers["x-request-id"], *native.header_value());
                        assert_eq!(id.as_str(), native.header_value().to_str().unwrap());
                        id.to_string()
                    },
                ),
            )
            .layer(middleware::from_fn(operational_http))
            .layer(middleware::from_fn(observe_http));
        let response = app.oneshot(request("/work")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        id_and_body(response).await
    });
    assert_eq!(body, id);
    let text = capture.text();
    let events = completion_fields(&text);
    assert_eq!(events.len(), 1, "{text}");
    assert!(
        events[0].contains(&format!("request_id=\"{id}\"")),
        "{text}"
    );
    assert!(!text.contains("secret-"), "{text}");
}

#[test]
fn quota_facts_reach_the_single_outer_event_in_both_wrapper_orders() {
    for quota_outermost in [false, true] {
        let capture = Capture::new();
        let (id, body) = capture.block_on(async {
            let app = Router::new().route(
                "/work",
                get(
                    |headers: HeaderMap,
                     Extension(id): Extension<CorrelationId>,
                     Extension(native): Extension<RequestId>,
                     mut request: Request<Body>| async move {
                        assert_eq!(headers.get_all("x-request-id").iter().count(), 1);
                        assert_eq!(headers["x-request-id"], *native.header_value());
                        assert_eq!(id.as_str(), native.header_value().to_str().unwrap());
                        QuotaRecorder::take(&mut request)
                            .expect("quota wrapper installed the sole writer")
                            .start()
                            .finish(QuotaTerminalFacts::Allowed);
                        OperationContext::new(Duration::from_secs(1))
                            .unwrap()
                            .run("nested.quota", |_| async {
                                tracing::info!(request_id = %id, "nested quota identity");
                                Ok::<_, std::convert::Infallible>(())
                            })
                            .await
                            .unwrap();
                        id.to_string()
                    },
                ),
            );
            let app = if quota_outermost {
                app.layer(middleware::from_fn(operational_http))
                    .layer(middleware::from_fn(operational_http_with_quota))
            } else {
                app.layer(middleware::from_fn(operational_http_with_quota))
                    .layer(middleware::from_fn(operational_http))
            };
            let response = app.oneshot(request("/work")).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            id_and_body(response).await
        });
        assert_eq!(body, id);
        let text = capture.text();
        let events = completion_fields(&text);
        assert_eq!(events.len(), 1, "{text}");
        assert!(
            events[0].contains(&format!("request_id=\"{id}\"")),
            "{text}"
        );
        assert!(events[0].contains("quota_outcome=\"allowed\""));
        assert!(events[0].contains("quota_consumption=\"consumed\""));
        assert!(
            text.lines()
                .any(|line| line.contains("nested quota identity") && line.contains(&id)),
            "{text}"
        );
        assert!(
            text.lines().any(|line| {
                line.contains("operation boundary finished")
                    && line.contains("nested.quota")
                    && line.contains(&id)
            }),
            "{text}"
        );
    }
}
