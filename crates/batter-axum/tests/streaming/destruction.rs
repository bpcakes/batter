//! How an owned body is destroyed: application panics on each side of the
//! hand-over, the order of the completion event and application destructors,
//! and the request span's lifetime when an admitted request is kept.

use super::support::{
    DESTROYED, Escaped, FINISHED, Marker, assert_finished, budget, client, drain, finished, get,
    next, paused, paused_with, ready,
};
use crate::capture::{Capture, test_dispatch};
use axum::{
    body::{Body, Bytes, HttpBody},
    http::{Method, Request, StatusCode, header},
    routing::get as route,
};
use batter_axum::{AdmittedRequest, GuardedRouter};
use batter_core::operation::Interruption;
use futures_util::{
    FutureExt,
    stream::{self, StreamExt},
};
use std::{
    io,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
    task::Poll,
    time::Duration,
};
use tracing_subscriber::{layer::Context, prelude::*, registry::LookupSpan};

/// Run one request whose handler panics, and return whether the panic reached
/// the caller.
fn panicking_request(capture: &Capture, guarded: GuardedRouter) -> bool {
    paused(capture, async move {
        let handle = ready();
        let app = client(&handle, guarded).await;
        let request = tokio::spawn(async move { get(&app, "/").await });
        request.await.unwrap_err().is_panic()
    })
}

#[test]
fn a_panicking_builder_still_cancels_the_scope_it_received_and_is_observed() {
    let capture = Capture::new();
    let escaped = Escaped::default();
    let kept = escaped.clone();
    let panicked = panicking_request(
        &capture,
        GuardedRouter::new().route(
            "/",
            route(move |admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(60, 5), |scope| -> Body {
                    kept.keep(scope.context());
                    panic!("builder failed")
                })
            }),
        ),
    );
    assert!(panicked);
    assert_eq!(escaped.context().check(), Err(Interruption::Cancelled));
    // An application failure, not a peer that went away.
    let event = assert_finished(&capture, "WARN", "panicked");
    assert!(event.contains("bytes=0"), "{event}");
    assert!(!event.contains("builder failed"), "{event}");
}

#[test]
fn a_handler_that_panics_after_the_hand_over_is_observed_as_panicked() {
    let capture = Capture::new();
    let escaped = Escaped::default();
    let kept = escaped.clone();
    let panicked = panicking_request(
        &capture,
        GuardedRouter::new().route(
            "/",
            route(move |admitted: AdmittedRequest| async move {
                let response = admitted.into_streaming_response(budget(60, 5), |scope| {
                    kept.keep(scope.context());
                    Body::from_stream(stream::pending::<Result<Bytes, io::Error>>())
                });
                if std::hint::black_box(true) {
                    panic!("handler failed after the hand-over");
                }
                response
            }),
        ),
    );
    assert!(panicked);
    assert_eq!(escaped.context().check(), Err(Interruption::Cancelled));
    let event = assert_finished(&capture, "WARN", "panicked");
    assert!(!event.contains("handler failed"), "{event}");
}

/// A transport's task catches a panicking poll and only then destroys the
/// body, outside the unwind. The body still names the panic, not the peer.
#[test]
fn a_stream_that_panics_while_polled_is_observed_as_panicked_when_destroyed() {
    let capture = Capture::new();
    let escaped = Escaped::default();
    let unwound = paused(&capture, {
        let kept = escaped.clone();
        let observed = escaped.clone();
        async move {
            let handle = ready();
            let app = client(
                &handle,
                GuardedRouter::new().route(
                    "/",
                    route(move |admitted: AdmittedRequest| async move {
                        admitted.into_streaming_response(budget(60, 5), |scope| {
                            kept.keep(scope.context());
                            Body::from_stream(stream::poll_fn(
                                |_| -> Poll<Option<Result<Bytes, io::Error>>> {
                                    panic!("stream failed")
                                },
                            ))
                        })
                    }),
                ),
            )
            .await;
            let mut response = get(&app, "/").await;
            let unwound = AssertUnwindSafe(next(response.body_mut()))
                .catch_unwind()
                .await
                .is_err();
            assert!(observed.context().check().is_ok());
            drop(response);
            unwound
        }
    });
    assert!(unwound);
    assert_eq!(escaped.context().check(), Err(Interruption::Cancelled));
    let event = assert_finished(&capture, "WARN", "panicked");
    assert!(!event.contains("stream failed"), "{event}");
}

/// A body its holder discards before its end is `dropped`, not a failure: here
/// Axum strips the body of a HEAD request routed to the GET handler, and
/// admission discards a response the handler built before its
/// response-construction budget ran out.
#[test]
fn a_body_discarded_by_head_or_by_a_construction_timeout_is_observed_as_dropped() {
    for head in [true, false] {
        let capture = Capture::new();
        let escaped = Escaped::default();
        let kept = escaped.clone();
        let status = paused(&capture, async move {
            let handle = ready();
            let app = client(
                &handle,
                GuardedRouter::new().route(
                    "/",
                    route(move |admitted: AdmittedRequest| async move {
                        let response = admitted.into_streaming_response(budget(60, 5), |scope| {
                            kept.keep(scope.context());
                            Body::from_stream(stream::pending::<Result<Bytes, io::Error>>())
                        });
                        if !head {
                            // Outlasts the one-second construction budget.
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                        response
                    }),
                ),
            )
            .await;
            let method = if head { Method::HEAD } else { Method::GET };
            let request = Request::builder()
                .method(method)
                .uri("/")
                .body(Body::empty())
                .unwrap();
            app.request(request).await.status()
        });
        if head {
            assert_eq!(status, StatusCode::OK);
        } else {
            assert!(status.is_server_error(), "{status}");
        }
        assert_eq!(escaped.context().check(), Err(Interruption::Cancelled));
        let event = assert_finished(&capture, "INFO", "dropped");
        assert!(event.contains("bytes=0"), "{event}");
    }
}

/// A response without a body frames none, so a length it declares is not one
/// the body must produce: an empty body behind a 304, or answering a HEAD
/// request, completed rather than ending short.
#[test]
fn a_declared_length_on_a_response_without_a_body_is_observed_as_completed() {
    for (method, status) in [
        (Method::GET, StatusCode::NOT_MODIFIED),
        (Method::HEAD, StatusCode::OK),
    ] {
        let capture = Capture::new();
        let answered = paused(&capture, async move {
            let handle = ready();
            let app = client(
                &handle,
                GuardedRouter::new().route(
                    "/",
                    route(move |admitted: AdmittedRequest| async move {
                        admitted.into_streaming_response(budget(60, 5), move |_scope| {
                            (status, [(header::CONTENT_LENGTH, "10")], Body::empty())
                        })
                    }),
                ),
            )
            .await;
            let request = Request::builder()
                .method(method)
                .uri("/")
                .body(Body::empty())
                .unwrap();
            app.request(request).await.status()
        });
        assert_eq!(answered, status);
        let event = assert_finished(&capture, "INFO", "completed");
        assert!(event.contains("bytes=0"), "{event}");
    }
}

/// The completion event precedes the application's destructors, which may
/// panic, both when the stream ends and when the transport drops the body.
#[test]
fn the_completion_event_precedes_the_application_bodys_destruction() {
    for (ends, outcome) in [(true, "completed"), (false, "dropped")] {
        let capture = Capture::new();
        paused(&capture, async move {
            let handle = ready();
            let app = client(
                &handle,
                GuardedRouter::new().route(
                    "/",
                    route(move |admitted: AdmittedRequest| async move {
                        admitted.into_streaming_response(budget(60, 5), move |_scope| {
                            let marker = Marker;
                            let first =
                                stream::iter([Ok::<_, io::Error>(Bytes::from_static(b"x"))]);
                            let rest = if ends {
                                stream::empty().boxed()
                            } else {
                                stream::pending().boxed()
                            };
                            Body::from_stream(first.chain(rest).map(move |item| {
                                let _owned = &marker;
                                item
                            }))
                        })
                    }),
                ),
            )
            .await;
            let mut response = get(&app, "/").await;
            if ends {
                assert_eq!(drain(response.body_mut()).await, ("x".to_owned(), None));
            } else {
                assert!(next(response.body_mut()).await.unwrap().is_ok());
            }
            drop(response);
        });
        assert_finished(&capture, "INFO", outcome);
        let text = capture.text();
        let lines: Vec<_> = text.lines().collect();
        let finished = lines.iter().position(|line| line.contains(FINISHED));
        let destroyed: Vec<_> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.contains(DESTROYED))
            .map(|(index, _)| index)
            .collect();
        assert_eq!(destroyed.len(), 1, "{text}");
        assert!(finished.unwrap() < destroyed[0], "{text}");
    }
}

/// A transport learns an empty body's end from `is_end_stream` and drops it
/// unpolled, so a refused start is reported on destruction, while an admitted
/// empty body is still a completed one.
#[test]
fn a_refused_start_with_an_empty_body_is_reported_cancelled_unpolled() {
    let capture = Capture::new();
    let status = paused(&capture, async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|once: AdmittedRequest, again: AdmittedRequest| async move {
                    let admitted = once.into_streaming_response(budget(60, 5), |_scope| ());
                    // The request's one body already started.
                    let refused = again.into_streaming_response(budget(60, 5), |_scope| ());
                    assert!(refused.body().is_end_stream());
                    drop(refused);
                    admitted
                }),
            ),
        )
        .await;
        get(&app, "/").await.status()
    });
    assert_eq!(status, StatusCode::OK);
    let events = finished(&capture);
    assert_eq!(events.len(), 2, "{}", capture.text());
    let (refused, admitted) = (&events[0], &events[1]);
    assert!(refused.trim_start().starts_with("WARN"), "{refused}");
    assert!(refused.contains("body_outcome=\"cancelled\""), "{refused}");
    assert!(admitted.trim_start().starts_with("INFO"), "{admitted}");
    assert!(
        admitted.contains("body_outcome=\"completed\""),
        "{admitted}"
    );
}

/// Names the spans the registry closes.
#[derive(Clone, Default)]
struct Closed(Arc<Mutex<Vec<&'static str>>>);

impl<S> tracing_subscriber::Layer<S> for Closed
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_close(&self, id: tracing::span::Id, context: Context<'_, S>) {
        if let Some(span) = context.span(&id) {
            self.0.lock().unwrap().push(span.name());
        }
    }
}

/// The request span belongs to the observer. An admitted request the
/// application keeps beyond the response neither holds it open nor releases it
/// later under another subscriber.
#[test]
fn an_admitted_request_kept_beyond_its_response_does_not_hold_the_request_span() {
    let closed = Closed::default();
    let dispatch = test_dispatch::new(tracing_subscriber::registry().with(closed.clone()));
    let kept: Arc<Mutex<Option<AdmittedRequest>>> = Arc::default();
    paused_with(&dispatch, {
        let kept = kept.clone();
        async move {
            let handle = ready();
            let app = client(
                &handle,
                GuardedRouter::new().route(
                    "/",
                    route(move |admitted: AdmittedRequest| async move {
                        *kept.lock().unwrap() = Some(admitted);
                        "kept"
                    }),
                ),
            )
            .await;
            assert_eq!(get(&app, "/").await.status(), StatusCode::OK);
        }
    });
    assert!(kept.lock().unwrap().is_some());
    let names = closed.0.lock().unwrap().clone();
    assert!(names.contains(&"batter.http"), "{names:?}");
}
