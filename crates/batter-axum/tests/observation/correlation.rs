use super::{
    capture::Capture,
    support::{Boundary, completions, request},
};
use axum::{Router, extract::Request, http::StatusCode, routing::get};
use batter::{
    lifecycle::ShutdownHandle,
    operation::{Interruption, OperationContext},
};
use batter_axum::RequestPolicy;
use std::{
    future::{Future, poll_fn},
    sync::{Arc, Mutex},
    task::Poll,
    time::Duration,
};
use tower::ServiceExt;
use tracing::{Instrument, Span, instrument::WithSubscriber};

fn application_span(capture: &Capture, id: &str) -> Span {
    tracing::dispatcher::with_default(
        &capture.dispatch,
        || tracing::info_span!(target: "application", "application.request", request_id = id, status = 777),
    )
}

struct Resource;
impl Drop for Resource {
    fn drop(&mut self) {
        tracing::warn!(target: "application", "handler resource dropped");
    }
}

fn router(mode: Boundary) -> Router {
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    mode.apply(
        Router::new().route(
            "/work",
            get(|| async {
                let _resource = Resource;
                tokio::task::yield_now().await;
                tracing::warn!(target: "application", "handler resumed");
                StatusCode::INTERNAL_SERVER_ERROR
            }),
        ),
        RequestPolicy::new(handle, Duration::from_secs(60)).unwrap(),
    )
}

#[test]
fn first_poll_context_survives_interleaving_completion_and_drop() {
    for filter in [
        "application=info,batter=warn",
        "application=info,batter=info",
    ] {
        for mode in [Boundary::Observation, Boundary::Split, Boundary::Combined] {
            for complete in [false, true] {
                for same_subscriber in [false, true] {
                    let capture = Capture::with_filter(filter);
                    let ambient = Capture::new();
                    ambient.block_on(async {
                        let app = router(mode);
                        let creation = application_span(&capture, "creation");
                        let mut futures = creation.in_scope(|| {
                            ["request-a", "request-b"].map(|_| {
                                Box::pin(
                                    app.clone()
                                        .oneshot(request("GET", "/work"))
                                        .with_subscriber(capture.dispatch.clone()),
                                )
                            })
                        });
                        for (future, id) in futures.iter_mut().zip(["request-a", "request-b"]) {
                            let origin = application_span(&capture, id);
                            assert!(!origin.is_disabled());
                            poll_fn(|cx| {
                                origin.in_scope(|| assert!(future.as_mut().poll(cx).is_pending()));
                                Poll::Ready(())
                            })
                            .await;
                            // Only the observer's context may keep this parent alive now.
                            drop(origin);
                        }
                        let later = application_span(
                            if same_subscriber { &capture } else { &ambient },
                            "unrelated",
                        );
                        for future in futures {
                            if complete {
                                assert_eq!(
                                    future.instrument(later.clone()).await.unwrap().status(),
                                    StatusCode::INTERNAL_SERVER_ERROR
                                );
                            } else {
                                later.in_scope(|| drop(future));
                            }
                        }
                    });
                    let text = capture.text();
                    let events = completions(&text);
                    assert_eq!(events.len(), 2, "{text}");
                    for (event, id) in events.iter().zip(["request-a", "request-b"]) {
                        assert!(
                            event.contains(&format!("request_id=\"{id}\" status=777")),
                            "{text}"
                        );
                        assert_eq!(event.contains("status=500"), complete, "{text}");
                        assert!(
                            event.contains(if complete {
                                "http_outcome=\"server_error\""
                            } else {
                                "http_outcome=\"dropped\""
                            }),
                            "{text}"
                        );
                    }
                    for id in ["request-a", "request-b"] {
                        let destruction: Vec<_> = text
                            .lines()
                            .filter(|line| {
                                line.contains("handler resource dropped") && line.contains(id)
                            })
                            .collect();
                        assert_eq!(destruction.len(), 1, "{text}");
                        assert_eq!(text.lines().filter(|line| line.contains("handler resumed") && line.contains(id)).count(), usize::from(complete), "{text}");
                    }
                    for forbidden in ["unrelated", "creation", "secret-"] {
                        assert!(!text.contains(forbidden), "{text}");
                    }
                    assert!(ambient.text().is_empty(), "{}", ambient.text());
                }
            }
        }
    }
}

#[test]
fn missing_or_filtered_original_parent_cannot_be_replaced_at_drop() {
    for mode in [Boundary::Observation, Boundary::Split, Boundary::Combined] {
        for filtered_parent in [false, true] {
            let capture = Capture::with_filter("application=info,batter=warn,hidden=off");
            capture.block_on(async {
                let origin =
                    tracing::info_span!(target: "hidden", "hidden.request", request_id = "hidden");
                assert!(origin.is_disabled());
                let mut future = Box::pin(router(mode).oneshot(request("GET", "/work")));
                poll_fn(|cx| {
                    if filtered_parent {
                        origin.in_scope(|| assert!(future.as_mut().poll(cx).is_pending()));
                    } else {
                        assert!(future.as_mut().poll(cx).is_pending());
                    }
                    Poll::Ready(())
                })
                .await;
                application_span(&capture, "unrelated").in_scope(|| drop(future));
            });
            let text = capture.text();
            let events = completions(&text);
            assert_eq!(events.len(), 1, "{text}");
            assert!(!events[0].contains("request_id"), "{text}");
            assert!(events[0].contains("http_outcome=\"dropped\""), "{text}");
            assert!(!events[0].contains("status="), "{text}");
        }
    }
}

#[test]
fn handler_unwind_propagates_and_preserves_dropped_observation_and_context_cancellation() {
    for filter in [
        "application=info,batter=warn",
        "application=info,batter=info",
    ] {
        for mode in [Boundary::Observation, Boundary::Split, Boundary::Combined] {
            let capture = Capture::with_filter(filter);
            let ambient = Capture::new();
            let escaped = Arc::new(Mutex::new(None));
            let saved = escaped.clone();
            ambient.block_on(async {
                let handle = ShutdownHandle::new();
                handle.mark_ready();
                let app = mode.apply(
                    Router::new().route(
                        "/panic",
                        get(move |request: Request| {
                            let saved = saved.clone();
                            async move {
                                let _resource = Resource;
                                *saved.lock().unwrap() =
                                    request.extensions().get::<OperationContext>().cloned();
                                tokio::task::yield_now().await;
                                panic_handler().await
                            }
                        }),
                    ),
                    RequestPolicy::new(handle, Duration::from_secs(60)).unwrap(),
                );
                let origin = application_span(&capture, "panicking-request");
                let task = tokio::spawn(
                    app.oneshot(request("GET", "/panic"))
                        .instrument(origin)
                        .with_subscriber(capture.dispatch.clone()),
                );
                let error = tokio::time::timeout(Duration::from_secs(2), task)
                    .await
                    .unwrap()
                    .unwrap_err();
                assert!(error.is_panic() && !error.is_cancelled());
                assert_eq!(
                    error.into_panic().downcast_ref::<&str>(),
                    Some(&"secret-panic-payload")
                );
            });
            let text = capture.text();
            let events = completions(&text);
            assert_eq!(events.len(), 1, "{text}");
            assert!(
                events[0].contains("request_id=\"panicking-request\""),
                "{text}"
            );
            assert!(events[0].contains("http_outcome=\"dropped\""), "{text}");
            assert!(!events[0].contains("status=500"), "{text}");
            assert!(
                !events[0]
                    .split_once("HTTP response boundary finished")
                    .unwrap()
                    .1
                    .contains("status="),
                "the HTTP event must not invent a status: {text}"
            );
            assert!(events[0].trim_start().starts_with("WARN"), "{text}");
            assert!(!text.contains("secret-panic-payload"), "{text}");
            assert_eq!(
                text.matches("handler resource dropped").count(),
                1,
                "{text}"
            );
            let destruction = text
                .lines()
                .find(|line| line.contains("handler resource dropped"))
                .unwrap();
            assert!(
                destruction.contains("request_id=\"panicking-request\" status=777"),
                "{text}"
            );
            assert!(ambient.text().is_empty(), "{}", ambient.text());
            let escaped = escaped.lock().unwrap();
            if matches!(mode, Boundary::Observation) {
                assert!(escaped.is_none());
            } else {
                assert_eq!(
                    escaped.as_ref().unwrap().check(),
                    Err(Interruption::Cancelled)
                );
            }
        }
    }
}

async fn panic_handler() -> StatusCode {
    panic!("secret-panic-payload")
}
