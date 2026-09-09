use super::{
    capture::Capture,
    support::{Boundary, assert_completion, completions, request},
};
use axum::{
    Extension, Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use batter::{
    lifecycle::ShutdownHandle,
    operation::{Interruption, OperationContext},
};
use batter_axum::{
    HttpObservationLevel, RequestPolicy, observe_http, request_admission, request_scope,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;
use tracing::{Instrument, instrument::WithSubscriber};

struct DropTrace;
impl Drop for DropTrace {
    fn drop(&mut self) {
        tracing::info!("handler resource dropped");
    }
}

#[test]
fn abort_preserves_observer_and_handler_destruction_under_the_first_poll_subscriber() {
    for mode in [
        Boundary::Observation,
        Boundary::Admission,
        Boundary::Split,
        Boundary::Combined,
    ] {
        let scoped = Capture::new();
        let ambient = Capture::new();
        ambient.block_on(async {
            let handle = ShutdownHandle::new();
            handle.mark_ready();
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let started = Arc::new(Mutex::new(Some(started_tx)));
            let escaped = Arc::new(Mutex::new(None));
            let saved = escaped.clone();
            let router = mode.apply(
                Router::new().route(
                    "/pending",
                    get(move |request: Request| {
                        let started = started.clone();
                        let saved = saved.clone();
                        async move {
                            let span = tracing::info_span!("handler.nested");
                            async move {
                                let _resource = DropTrace;
                                // A constructed but unreturned response cannot override
                                // the event produced when the pending future is dropped.
                                let _unreturned_response = (
                                    Extension(HttpObservationLevel(tracing::Level::INFO)),
                                    StatusCode::SERVICE_UNAVAILABLE,
                                )
                                    .into_response();
                                *saved.lock().unwrap() =
                                    request.extensions().get::<OperationContext>().cloned();
                                started.lock().unwrap().take().unwrap().send(()).unwrap();
                                std::future::pending::<&'static str>().await
                            }
                            .instrument(span)
                            .await
                        }
                    }),
                ),
                RequestPolicy::new(handle, Duration::from_secs(60)).unwrap(),
            );
            // Construct under a different subscriber: capture must occur at first poll.
            let task = tokio::spawn(
                router
                    .oneshot(request("GET", "/pending"))
                    .with_subscriber(scoped.dispatch.clone()),
            );
            tokio::time::timeout(Duration::from_secs(2), started_rx)
                .await
                .unwrap()
                .unwrap();
            task.abort();
            let error = tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap_err();
            assert!(
                error.is_cancelled() && !error.is_panic(),
                "{mode:?}: {error}"
            );
            if matches!(mode, Boundary::Observation) {
                assert!(escaped.lock().unwrap().is_none());
            } else {
                assert_eq!(
                    escaped.lock().unwrap().as_ref().unwrap().check(),
                    Err(Interruption::Cancelled)
                );
            }
        });
        let text = scoped.text();
        assert_eq!(
            text.matches("handler resource dropped").count(),
            1,
            "{mode:?}: {text}"
        );
        assert_eq!(
            text.matches("operation boundary finished").count(),
            usize::from(!matches!(mode, Boundary::Observation)),
            "{mode:?}: {text}"
        );
        let events = completions(&text);
        if matches!(mode, Boundary::Admission) {
            assert!(events.is_empty(), "{text}");
        } else {
            assert_eq!(events.len(), 1, "{mode:?}: {text}");
            assert!(events[0].contains("http_outcome=\"dropped\""), "{text}");
            assert!(events[0].trim_start().starts_with("WARN "), "{text}");
            assert!(!events[0].contains("status="), "{text}");
        }
        assert!(ambient.text().is_empty(), "{mode:?}: {}", ambient.text());
    }
}

async fn discard_unpolled(State(mode): State<Boundary>, request: Request, next: Next) -> Response {
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    let policy = RequestPolicy::new(handle, Duration::from_secs(1)).unwrap();
    match mode {
        Boundary::Observation => drop(observe_http(request, next)),
        Boundary::Admission => drop(request_admission(State(policy), request, next)),
        Boundary::Combined => drop(request_scope(State(policy), request, next)),
        Boundary::Split => unreachable!("split is composed from the other entry points"),
    }
    StatusCode::NO_CONTENT.into_response()
}

#[tokio::test]
async fn constructing_and_dropping_each_unpolled_entry_point_does_no_work_or_observation() {
    for mode in [
        Boundary::Observation,
        Boundary::Admission,
        Boundary::Combined,
    ] {
        let capture = Capture::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let router = Router::new()
            .route(
                "/work",
                get(move || {
                    called.fetch_add(1, Ordering::SeqCst);
                    async { "must not run" }
                }),
            )
            .layer(middleware::from_fn_with_state(mode, discard_unpolled));
        let response = router
            .oneshot(request("GET", "/work"))
            .with_subscriber(capture.dispatch.clone())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(capture.text().is_empty(), "{}", capture.text());
    }
}

#[tokio::test(start_paused = true)]
async fn standalone_observation_adds_no_context_or_deadline_and_ends_before_body_drop() {
    let capture = Capture::new();
    let router = Router::new()
        .route(
            "/slow",
            get(|request: Request| async move {
                assert!(request.extensions().get::<OperationContext>().is_none());
                tokio::time::sleep(Duration::from_secs(5)).await;
                "secret-response-body"
            }),
        )
        .layer(middleware::from_fn(observe_http));
    let response = router
        .oneshot(request("GET", "/slow"))
        .with_subscriber(capture.dispatch.clone())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let before = capture.text();
    assert_completion(&before, 200, "completed", "/slow", "GET");
    assert!(before.contains("latency_ms=5000"), "{before}");
    assert!(!before.contains("operation boundary finished"), "{before}");
    drop(response.into_body());
    assert_eq!(
        capture.text(),
        before,
        "body drop cannot create a second or dropped observation"
    );
}

#[tokio::test]
async fn split_admission_allows_drain_then_cancels_the_context_at_response_completion() {
    let capture = Capture::new();
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    let inside = handle.clone();
    let escaped = Arc::new(Mutex::new(None));
    let saved = escaped.clone();
    let router = Boundary::Split.apply(
        Router::new().route(
            "/work",
            get(move |request: Request| {
                let inside = inside.clone();
                let saved = saved.clone();
                async move {
                    let context = request
                        .extensions()
                        .get::<OperationContext>()
                        .unwrap()
                        .clone();
                    inside.request();
                    assert!(
                        context.check().is_ok(),
                        "drain must not cancel admitted work"
                    );
                    *saved.lock().unwrap() = Some(context);
                    "ok"
                }
            }),
        ),
        RequestPolicy::new(handle, Duration::from_secs(1)).unwrap(),
    );
    let response = router
        .oneshot(request("GET", "/work"))
        .with_subscriber(capture.dispatch.clone())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        escaped.lock().unwrap().as_ref().unwrap().check(),
        Err(Interruption::Cancelled)
    );
    assert_completion(&capture.text(), 200, "completed", "/work", "GET");
}
