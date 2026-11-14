//! The owned body's context, observation and destruction, without shutdown.

use super::support::{
    Closed, DESTROYED, Escaped, FINISHED, Marker, assert_finished, budget, client, drain, finished,
    get, next, paused, paused_with, ready,
};
use crate::capture::{Capture, test_dispatch};
use axum::{
    body::{Body, Bytes, HttpBody},
    http::{StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, Sse},
    },
    routing::get as route,
};
use batter_axum::{AdmittedRequest, BodyScope, GuardedRouter};
use batter_core::operation::{Interruption, OperationContext, OperationError};
use futures_util::stream::{self, Stream, StreamExt};
use std::{
    convert::Infallible,
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
use tracing::{Instrument, instrument::WithSubscriber};
use tracing_subscriber::prelude::*;

const STEP: Duration = Duration::from_millis(10);

/// `count` chunks, each produced by a bounded step that outlives the handler.
fn chunks(
    context: OperationContext,
    count: u32,
) -> impl Stream<Item = Result<Bytes, io::Error>> + Send + 'static {
    stream::unfold((context, 0), move |(context, index)| async move {
        if index == count {
            return None;
        }
        let step = context
            .run("stream.step", |_step| async move {
                tokio::time::sleep(STEP).await;
                tracing::info!(index, "stream step produced");
                Ok::<_, Infallible>(Bytes::from(format!("chunk {index};")))
            })
            .await;
        match step {
            Ok(chunk) => Some((Ok(chunk), (context, index + 1))),
            Err(OperationError::Interrupted(reason)) => Some((
                Ok(Bytes::from(format!("interrupted: {reason}"))),
                (context, count),
            )),
            Err(OperationError::Failed(never)) => match never {},
        }
    })
}

async fn owned(admitted: AdmittedRequest) -> Response {
    admitted.into_streaming_response(budget(60, 5), |scope: BodyScope| {
        (
            [(header::CONTENT_TYPE, "text/plain")],
            Body::from_stream(chunks(scope.context().clone(), 3)),
        )
    })
}

/// The composition the owner replaces: the request's context moved into the body.
async fn unowned(admitted: AdmittedRequest) -> Response {
    Body::from_stream(chunks(admitted.context().clone(), 3)).into_response()
}

#[test]
fn owned_steps_run_after_the_handler_returned_where_the_request_context_is_cancelled() {
    let capture = Capture::new();
    let (owned_text, unowned_text) = paused(&capture, async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new()
                .route("/owned", route(owned))
                .route("/unowned", route(unowned)),
        )
        .await;
        let mut owned = get(&app, "/owned").await;
        assert_eq!(owned.status(), StatusCode::OK);
        assert_eq!(owned.headers()[header::CONTENT_TYPE], "text/plain");
        let owned_text = drain(owned.body_mut()).await;
        let mut unowned = get(&app, "/unowned").await;
        (owned_text, drain(unowned.body_mut()).await)
    });
    assert_eq!(owned_text, ("chunk 0;chunk 1;chunk 2;".to_owned(), None));
    // Control: the escaped request context is already cancelled at frame one.
    assert!(
        unowned_text
            .0
            .starts_with("interrupted: operation cancelled"),
        "{unowned_text:?}"
    );
    assert_eq!(unowned_text.1, None);
    let event = assert_finished(&capture, "INFO", "completed");
    assert!(event.contains("bytes=24"), "{event}");
    assert!(event.contains("batter.http{"), "{event}");
    // The request's own completion still describes response construction.
    let text = capture.text();
    assert_eq!(text.matches("HTTP response boundary finished").count(), 2);
    // Steps polled after the handler returned stay inside the request's span.
    let steps: Vec<_> = text
        .lines()
        .filter(|line| line.contains("stream step produced"))
        .collect();
    assert_eq!(steps.len(), 3, "{text}");
    assert!(steps.iter().all(|line| line.contains("batter.http{")));
}

#[test]
fn a_context_kept_beyond_the_body_observes_its_end_as_cancellation() {
    let capture = Capture::new();
    let escaped = Escaped::default();
    paused(&capture, {
        let escaped = escaped.clone();
        async move {
            let handle = ready();
            let kept = escaped.clone();
            let app = client(
                &handle,
                GuardedRouter::new().route(
                    "/",
                    route(move |admitted: AdmittedRequest| async move {
                        admitted.into_streaming_response(budget(60, 5), |scope| {
                            kept.keep(scope.context());
                            Body::from_stream(chunks(scope.context().clone(), 2))
                        })
                    }),
                ),
            )
            .await;
            let mut response = get(&app, "/").await;
            assert!(next(response.body_mut()).await.unwrap().is_ok());
            // The handler returned long ago; the body's context is still live.
            assert!(escaped.context().check().is_ok());
            drain(response.body_mut()).await;
        }
    });
    assert_eq!(escaped.context().check(), Err(Interruption::Cancelled));
    assert_finished(&capture, "INFO", "completed");
}

#[test]
fn a_body_dropped_mid_stream_or_never_polled_is_observed_once_and_cancels_its_scope() {
    for frames in [1, 0] {
        let capture = Capture::new();
        let escaped = Escaped::default();
        paused(&capture, {
            let escaped = escaped.clone();
            async move {
                let handle = ready();
                let kept = escaped.clone();
                let app = client(
                    &handle,
                    GuardedRouter::new().route(
                        "/",
                        route(move |admitted: AdmittedRequest| async move {
                            admitted.into_streaming_response(budget(60, 5), |scope| {
                                kept.keep(scope.context());
                                Body::from_stream(chunks(scope.context().clone(), 3))
                            })
                        }),
                    ),
                )
                .await;
                let mut response = get(&app, "/").await;
                for _ in 0..frames {
                    assert!(next(response.body_mut()).await.unwrap().is_ok());
                }
                assert!(escaped.context().check().is_ok());
                drop(response);
            }
        });
        assert_eq!(escaped.context().check(), Err(Interruption::Cancelled));
        let event = assert_finished(&capture, "INFO", "dropped");
        assert!(event.contains(&format!("bytes={}", frames * 8)), "{event}");
    }
}

#[test]
fn a_stream_error_reaches_the_transport_and_is_observed_as_failed() {
    let capture = Capture::new();
    let outcome = paused(&capture, async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(60, 5), |_scope| {
                        Body::from_stream(stream::iter([
                            Ok(Bytes::from_static(b"partial")),
                            Err(io::Error::other("source failed")),
                        ]))
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        drain(response.body_mut()).await
    });
    assert_eq!(
        outcome,
        ("partial".to_owned(), Some("source failed".to_owned()))
    );
    let event = assert_finished(&capture, "WARN", "failed");
    assert!(event.contains("bytes=7"), "{event}");
    // The application's error text is never copied into the event.
    assert!(!event.contains("source failed"), "{event}");
}

#[test]
fn any_response_is_owned_including_server_sent_events_and_a_sized_body() {
    let capture = Capture::new();
    let (events, sized, exact) = paused(&capture, async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new()
                .route(
                    "/events",
                    route(|admitted: AdmittedRequest| async move {
                        admitted.into_streaming_response(budget(60, 5), |scope| {
                            Sse::new(stream::unfold((scope, 0), |(scope, tick)| async move {
                                if tick == 2 {
                                    return None;
                                }
                                tokio::time::sleep(STEP).await;
                                let event = Event::default().data(tick.to_string());
                                Some((Ok::<_, Infallible>(event), (scope, tick + 1)))
                            }))
                        })
                    }),
                )
                .route(
                    "/sized",
                    route(|admitted: AdmittedRequest| async move {
                        admitted.into_streaming_response(budget(60, 5), |_scope| "hello")
                    }),
                ),
        )
        .await;
        let mut events = get(&app, "/events").await;
        assert_eq!(events.headers()[header::CONTENT_TYPE], "text/event-stream");
        let events = drain(events.body_mut()).await;
        let mut sized = get(&app, "/sized").await;
        let exact = sized.body().size_hint().exact();
        (events, drain(sized.body_mut()).await, exact)
    });
    assert_eq!(events, ("data: 0\n\ndata: 1\n\n".to_owned(), None));
    assert_eq!(sized, ("hello".to_owned(), None));
    // A sized body keeps its length, so the transport still sends it.
    assert_eq!(exact, Some(5));
    let finished = finished(&capture);
    assert_eq!(finished.len(), 2, "{}", capture.text());
    assert!(finished.iter().all(|event| event.contains("\"completed\"")));
}

/// The transport polls and destroys the body under whatever subscriber its own
/// task has. The body keeps the one current when the handler handed it over,
/// for its polls, the application's destructors and the release of the request
/// span. A registry releases a closing span's parent through the current
/// default, so the request runs inside a parent span: releasing it under the
/// transport's registry would name a span that registry never saw.
#[test]
fn polling_and_destruction_keep_the_handlers_subscriber_and_span() {
    let capture = Capture::new();
    let transport = Capture::new();
    paused(&capture, async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(60, 5), |scope: BodyScope| {
                        let marker = Marker;
                        let steps = chunks(scope.context().clone(), 3);
                        Body::from_stream(steps.map(move |item| {
                            let _owned = &marker;
                            item
                        }))
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/")
            .instrument(tracing::info_span!("caller"))
            .await;
        let elsewhere = transport.dispatch.clone();
        async move {
            assert!(next(response.body_mut()).await.unwrap().is_ok());
            drop(response);
        }
        .with_subscriber(elsewhere)
        .await;
    });
    let text = capture.text();
    // The request span has a parent, so its release must reach this registry.
    let step = text
        .lines()
        .find(|line| line.contains("stream step produced"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        step.contains(" caller:") && step.contains(":batter.http{"),
        "{step}"
    );
    assert!(text.contains(DESTROYED), "{text}");
    assert_finished(&capture, "INFO", "dropped");
    let foreign = transport.text();
    assert!(!foreign.contains("stream step produced"), "{foreign}");
    assert!(!foreign.contains(FINISHED), "{foreign}");
    assert!(!foreign.contains(DESTROYED), "{foreign}");
}

/// Serve one request whose handler hands its body over under `switched`, read
/// a frame inside a `caller` span, then drop the body, which then holds the
/// request span's last reference.
async fn hand_over_under(switched: tracing::Dispatch) {
    let handle = ready();
    let app = client(
        &handle,
        GuardedRouter::new().route(
            "/",
            route(move |admitted: AdmittedRequest| {
                let switched = switched.clone();
                async move {
                    tracing::dispatcher::with_default(&switched, || {
                        admitted.into_streaming_response(budget(60, 5), |scope: BodyScope| {
                            let marker = Marker;
                            let steps = chunks(scope.context().clone(), 3);
                            Body::from_stream(steps.map(move |item| {
                                let _owned = &marker;
                                item
                            }))
                        })
                    })
                }
            }),
        ),
    )
    .await;
    let mut response = get(&app, "/")
        .instrument(tracing::info_span!("caller"))
        .await;
    assert!(next(response.body_mut()).await.unwrap().is_ok());
    drop(response);
}

/// A handler may switch subscribers before handing its body over. The body's
/// own work follows that switch, but the request span belongs to the request's
/// subscriber: the completion event is parented to it there, and it is
/// released and closed there. Releasing it through the other registry would
/// hand that registry its parent, a span it never saw, and its registry panics.
#[test]
fn a_body_handed_over_under_another_subscriber_keeps_the_request_span_in_its_own_registry() {
    let capture = Capture::new();
    let other = Capture::new();
    paused(&capture, hand_over_under(other.dispatch.clone()));
    let event = assert_finished(&capture, "INFO", "dropped");
    assert!(
        event.contains(" caller:") && event.contains(":batter.http{"),
        "{event}"
    );
    let text = capture.text();
    assert!(!text.contains("stream step produced"), "{text}");
    assert!(!text.contains(DESTROYED), "{text}");
    let elsewhere = other.text();
    assert!(elsewhere.contains("stream step produced"), "{elsewhere}");
    assert!(elsewhere.contains(DESTROYED), "{elsewhere}");
    assert!(!elsewhere.contains(FINISHED), "{elsewhere}");

    // The same request against a registry that records closes: the request
    // span, and the parent it holds, close there once the body is dropped.
    let closed = Closed::default();
    let requests = test_dispatch::new(tracing_subscriber::registry().with(closed.clone()));
    let other = Capture::new();
    paused_with(&requests, hand_over_under(other.dispatch.clone()));
    let names = closed.names();
    assert!(names.contains(&"batter.http"), "{names:?}");
    assert!(names.contains(&"caller"), "{names:?}");
}

const END_ANSWERED: &str = "end query answered";
const SIZE_ANSWERED: &str = "size query answered";

/// Never produces a frame, and observes each question it is asked.
struct Answers;

impl http_body::Body for Answers {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Bytes>, io::Error>>> {
        std::task::Poll::Pending
    }

    fn is_end_stream(&self) -> bool {
        tracing::info!("{END_ANSWERED}");
        false
    }

    fn size_hint(&self) -> http_body::SizeHint {
        tracing::info!("{SIZE_ANSWERED}");
        http_body::SizeHint::default()
    }
}

/// A transport asks the body for its end and size as well as polling it, on
/// its own task. Answering runs application code, so it is observed under the
/// handler's subscriber and inside the request span, like a poll.
#[test]
fn a_transports_end_and_size_queries_keep_the_handlers_subscriber_and_span() {
    let capture = Capture::new();
    let transport = Capture::new();
    paused(&capture, async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(60, 5), |_scope| Body::new(Answers))
                }),
            ),
        )
        .await;
        let response = get(&app, "/").await;
        tracing::dispatcher::with_default(&transport.dispatch, || {
            assert!(!response.body().is_end_stream());
            assert_eq!(response.body().size_hint().exact(), None);
            drop(response);
        });
    });
    let text = capture.text();
    for answered in [END_ANSWERED, SIZE_ANSWERED] {
        let line = text
            .lines()
            .find(|line| line.contains(answered))
            .unwrap_or_else(|| panic!("{text}"));
        assert!(line.contains("batter.http{"), "{line}");
    }
    assert_finished(&capture, "INFO", "dropped");
    let foreign = transport.text();
    assert!(!foreign.contains(END_ANSWERED), "{foreign}");
    assert!(!foreign.contains(SIZE_ANSWERED), "{foreign}");
}

/// The authority to start a body belongs to the request, not to one extracted
/// value: a second extraction cannot start another body, and a request kept
/// past its response cannot start one at all, even once the process drains.
#[test]
fn one_body_starts_per_request_and_only_during_response_construction() {
    let capture = Capture::new();
    let (first, second, late) = (Escaped::default(), Escaped::default(), Escaped::default());
    let kept: Arc<Mutex<Option<AdmittedRequest>>> = Arc::default();
    let (second_text, late_text) = paused(&capture, {
        let (first, second, late) = (first.clone(), second.clone(), late.clone());
        let kept = kept.clone();
        async move {
            let handle = ready();
            let stash = kept.clone();
            let app = client(
                &handle,
                GuardedRouter::new()
                    .route(
                        "/",
                        route(
                            move |once: AdmittedRequest, again: AdmittedRequest| async move {
                                drop(once.into_streaming_response(budget(60, 5), |scope| {
                                    first.keep(scope.context());
                                    Body::from_stream(chunks(scope.context().clone(), 3))
                                }));
                                again.into_streaming_response(budget(60, 5), |scope| {
                                    second.keep(scope.context());
                                    Body::from_stream(chunks(scope.context().clone(), 3))
                                })
                            },
                        ),
                    )
                    // This request never starts a body; it is only kept.
                    .route(
                        "/kept",
                        route(move |keep: AdmittedRequest| async move {
                            *stash.lock().unwrap() = Some(keep);
                            "kept"
                        }),
                    ),
            )
            .await;
            let mut response = get(&app, "/").await;
            let second_text = drain(response.body_mut()).await;
            assert_eq!(get(&app, "/kept").await.status(), StatusCode::OK);
            handle.request();
            let stale = kept.lock().unwrap().take().unwrap();
            let mut late_response = stale.into_streaming_response(budget(60, 5), |scope| {
                late.keep(scope.context());
                Body::from_stream(chunks(scope.context().clone(), 3))
            });
            (second_text, drain(late_response.body_mut()).await)
        }
    });
    let refused = (
        String::new(),
        Some("response body ended: cancelled".to_owned()),
    );
    assert_eq!(second_text, refused);
    assert_eq!(late_text, refused);
    for scope in [&first, &second, &late] {
        assert_eq!(scope.context().check(), Err(Interruption::Cancelled));
    }
    let text = capture.text();
    assert!(!text.contains("stream step produced"), "{text}");
    let events = finished(&capture);
    assert_eq!(events.len(), 3, "{text}");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.contains("body_outcome=\"cancelled\""))
            .count(),
        2,
        "{text}"
    );
    assert!(events[0].contains("body_outcome=\"dropped\""), "{text}");
}
