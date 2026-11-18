//! Endings a holder other than Hyper can still reach: polls after trailers, a
//! final empty frame whose end is answered too late, a refusal reported after
//! the request's observer finished, and a body its handler kept while another
//! response was returned.

use super::support::{
    Escaped, assert_finished, budget, client, finished, get, next, paused, ready,
};
use crate::capture::Capture;
use axum::{
    body::{Body, Bytes, HttpBody},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    routing::get as route,
};
use batter_axum::{AdmittedRequest, BodyBudget, GuardedRouter};
use batter_core::operation::Interruption;
use http_body::{Frame, SizeHint};
use std::{
    convert::Infallible,
    future::poll_fn,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

const POLLED_AFTER_TRAILERS: &str = "source polled after its trailers";
const END_ASKED_AFTER_SLOW_FRAME: &str = "end asked after the slow frame's destruction";

/// Data, then trailers, then pending: the source never yields its end, and
/// reports any poll that follows its trailers. It reports `promised` as its
/// exact size, if any, whatever it goes on to yield.
struct Trailed {
    frames: u8,
    promised: Option<u64>,
}

impl HttpBody for Trailed {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.frames += 1;
        match self.frames {
            1 => Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"hello"))))),
            2 => Poll::Ready(Some(Ok(Frame::trailers(HeaderMap::from_iter([(
                HeaderName::from_static("x-digest"),
                HeaderValue::from_static("done"),
            )]))))),
            _ => {
                tracing::info!("{POLLED_AFTER_TRAILERS}");
                Poll::Pending
            }
        }
    }

    fn size_hint(&self) -> SizeHint {
        self.promised.map(SizeHint::with_exact).unwrap_or_default()
    }
}

/// What a holder that keeps the body saw around its trailers.
struct AroundTrailers {
    trailers: Option<HeaderMap>,
    /// The escaped scope and the number of completion events once the
    /// trailers were returned, before the body was polled again or dropped.
    with_trailers: (Result<(), Interruption>, usize),
    ended_at_once: bool,
    reported_end: bool,
    ended_later: bool,
}

/// Serve [`Trailed`] in-process and poll it as a holder that keeps the body:
/// data, trailers, once more, and again after the idle bound has passed.
fn polled_past_trailers(capture: &Capture, promised: Option<u64>) -> AroundTrailers {
    let escaped = Escaped::default();
    paused(capture, async move {
        let handle = ready();
        let kept = escaped.clone();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(move |admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(60, 5), move |scope| {
                        kept.keep(scope.context());
                        Body::new(Trailed {
                            frames: 0,
                            promised,
                        })
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        let body = response.body_mut();
        let mut poll =
            async || poll_fn(|cx| Poll::Ready(Pin::new(&mut *body).poll_frame(cx))).await;
        assert!(matches!(poll().await, Poll::Ready(Some(Ok(frame))) if frame.is_data()));
        assert!(escaped.context().check().is_ok());
        assert!(finished(capture).is_empty(), "{}", capture.text());
        let trailers = match poll().await {
            Poll::Ready(Some(Ok(frame))) => frame.into_trailers().ok(),
            _ => None,
        };
        let with_trailers = (escaped.context().check(), finished(capture).len());
        let ended_at_once = matches!(poll().await, Poll::Ready(None));
        let reported_end = response.body().is_end_stream();
        tokio::time::advance(Duration::from_secs(6)).await;
        AroundTrailers {
            trailers,
            with_trailers,
            ended_at_once,
            reported_end,
            ended_later: next(response.body_mut()).await.is_none(),
        }
    })
}

/// The body ended with its trailers, not at a later poll or its destruction:
/// its scope was cancelled and its one event emitted while the holder still
/// kept it unpolled, the next poll found the end, and the source, which would
/// stay pending until the idle bound, was never asked again.
fn assert_ended_with_trailers(capture: &Capture, seen: AroundTrailers) {
    assert_eq!(seen.trailers.unwrap()["x-digest"], "done");
    assert_eq!(seen.with_trailers, (Err(Interruption::Cancelled), 1));
    assert!(seen.ended_at_once);
    assert!(seen.reported_end);
    assert!(seen.ended_later);
    let text = capture.text();
    assert!(!text.contains(POLLED_AFTER_TRAILERS), "{text}");
}

/// Trailers are a body's last frame. Hyper destroys the body once it has
/// them, but a holder that collects a body polls until it sees the end: the
/// owned body ends as it yields its trailers, so that holder finds the end at
/// once, and one that only keeps the body holds no live scope.
#[test]
fn trailers_end_the_body_for_a_holder_that_polls_on() {
    let capture = Capture::new();
    let seen = polled_past_trailers(&capture, None);
    assert_ended_with_trailers(&capture, seen);
    let event = assert_finished(&capture, "INFO", "completed");
    assert!(event.contains("bytes=5"), "{event}");
}

/// Trailers end the source wherever it stands against its declaration. A
/// body that reported an exact size of ten at the hand-over, with no length
/// header, and yields five bytes before its trailers ended short of what its
/// own response declared: the trailers are still forwarded, and the body is
/// `failed`.
#[test]
fn trailers_short_of_the_declared_size_end_the_body_as_failed() {
    let capture = Capture::new();
    let seen = polled_past_trailers(&capture, Some(10));
    assert_ended_with_trailers(&capture, seen);
    let event = assert_finished(&capture, "WARN", "failed");
    assert!(event.contains("bytes=5"), "{event}");
}

/// What outlasts the bound once a final empty frame was returned in time.
#[derive(Clone, Copy, Debug)]
enum Slow {
    /// The source's answer about its end.
    EndQuery,
    /// The destruction of the empty frame's buffer.
    FrameDestructor,
}

/// The empty buffer of a final frame, slow to destroy if asked to be.
struct EmptyBuffer(Duration);

impl AsRef<[u8]> for EmptyBuffer {
    fn as_ref(&self) -> &[u8] {
        &[]
    }
}

impl Drop for EmptyBuffer {
    fn drop(&mut self) {
        std::thread::sleep(self.0);
    }
}

/// Data, then at once a final empty frame, after which it reports its end;
/// either that answer or the frame's destruction blocks the task for `blocks`.
struct EndsWithSlowEmpty {
    polls: u8,
    slow: Slow,
    blocks: Duration,
}

impl HttpBody for EndsWithSlowEmpty {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.polls += 1;
        let destroying = match self.slow {
            Slow::FrameDestructor => self.blocks,
            Slow::EndQuery => Duration::ZERO,
        };
        match self.polls {
            1 => Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"hello"))))),
            2 => Poll::Ready(Some(Ok(Frame::data(Bytes::from_owner(EmptyBuffer(
                destroying,
            )))))),
            _ => Poll::Ready(None),
        }
    }

    fn is_end_stream(&self) -> bool {
        if self.polls == 2 {
            match self.slow {
                Slow::EndQuery => std::thread::sleep(self.blocks),
                Slow::FrameDestructor => tracing::info!("{END_ASKED_AFTER_SLOW_FRAME}"),
            }
        }
        self.polls >= 2
    }
}

/// A final empty frame ends the message only inside its bounds. The frame's
/// destruction and the end query that follows are application code like the
/// poll, so one that outlasts the idle or total bound leaves the bound, not a
/// clean end, as the outcome: a chunked peer must see the interruption. A
/// body the frame's destruction left expired is not asked for its end at
/// all, since that query could block or unwind in turn. On real time,
/// because a paused clock cannot move during a synchronous call.
#[test]
fn a_final_empty_frame_answered_after_a_bound_is_not_a_clean_end() {
    let blocks = Duration::from_millis(400);
    let cases = [(60_000, "idle_timeout"), (200, "deadline_exceeded")];
    for ((total, outcome), slow) in cases
        .into_iter()
        .flat_map(|case| [(case, Slow::EndQuery), (case, Slow::FrameDestructor)])
    {
        let capture = Capture::new();
        let (first, second) = capture.block_on(async move {
            let handle = ready();
            let app = client(
                &handle,
                GuardedRouter::new().route(
                    "/",
                    route(move |admitted: AdmittedRequest| async move {
                        let budget = BodyBudget::new(
                            Duration::from_millis(total),
                            Duration::from_millis(150),
                        )
                        .unwrap();
                        admitted.into_streaming_response(budget, move |_scope| {
                            Body::new(EndsWithSlowEmpty {
                                polls: 0,
                                slow,
                                blocks,
                            })
                        })
                    }),
                ),
            )
            .await;
            let mut response = get(&app, "/").await;
            let first = next(response.body_mut()).await;
            (first, next(response.body_mut()).await)
        });
        assert_eq!(first.unwrap().unwrap(), "hello", "{outcome} {slow:?}");
        let error = second
            .unwrap_or_else(|| panic!("a clean end for {outcome} {slow:?}"))
            .unwrap_err();
        assert_eq!(
            std::error::Error::source(&error).unwrap().to_string(),
            format!("response body ended: {outcome}"),
            "{slow:?}"
        );
        let event = assert_finished(&capture, "WARN", outcome);
        assert!(event.contains("bytes=5"), "{event}");
        let text = capture.text();
        assert!(
            !text.contains(END_ASKED_AFTER_SLOW_FRAME),
            "{slow:?}: {text}"
        );
    }
}

/// A request kept beyond its response can no longer start a body, and its
/// observer has finished and released the request span. The refusal is still
/// that request's event: it carries the request's id, so it is reported to
/// the subscriber the request was admitted under, which still exists here,
/// not to whichever one is current where the kept request is finally used.
#[test]
fn a_start_refused_after_the_response_reports_to_the_requests_subscriber() {
    let capture = Capture::new();
    let other = Capture::new();
    let kept: Arc<Mutex<Option<AdmittedRequest>>> = Arc::default();
    paused(&capture, {
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
    assert!(finished(&capture).is_empty(), "{}", capture.text());
    let stale = kept.lock().unwrap().take().unwrap();
    let request_id = stale.correlation_id().as_str().to_owned();
    // Long after the response, under an unrelated subscriber, and while the
    // request's own subscriber has another request's span entered on this
    // thread, which a contextual event would adopt as its parent.
    tracing::dispatcher::with_default(&capture.dispatch, || {
        let unrelated = tracing::info_span!("unrelated");
        let _entered = unrelated.enter();
        tracing::dispatcher::with_default(&other.dispatch, || {
            drop(stale.into_streaming_response(budget(60, 5), |_scope| Body::empty()));
        });
    });
    let event = assert_finished(&capture, "WARN", "cancelled");
    assert!(
        event.contains(&format!("request_id=\"{request_id}\"")),
        "{event}"
    );
    // The request span closed with its observer, so nothing reopens it, and
    // the event is a root: it joins no span that merely happens to be current.
    assert!(!event.contains("batter.http{"), "{event}");
    assert!(!event.contains("unrelated"), "{event}");
    let elsewhere = other.text();
    assert!(elsewhere.is_empty(), "{elsewhere}");
}

/// A handler can keep the body it started and return another response. The
/// returned response says nothing about the kept body: an empty 200 has an
/// exact length of zero, which the kept body, never polled, did not produce.
/// Destroyed later, it is `dropped`, not `completed`.
#[test]
fn a_body_kept_while_another_response_is_returned_is_not_judged_by_that_response() {
    let capture = Capture::new();
    let kept: Arc<Mutex<Option<Body>>> = Arc::default();
    let events = &capture;
    paused(events, async move {
        let handle = ready();
        let stash = kept.clone();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(move |admitted: AdmittedRequest| async move {
                    let started = admitted.into_streaming_response(budget(60, 5), |_scope| {
                        Body::from_stream(
                            futures_util::stream::pending::<Result<Bytes, Infallible>>(),
                        )
                    });
                    *stash.lock().unwrap() = Some(started.into_body());
                    StatusCode::OK
                }),
            ),
        )
        .await;
        let response = get(&app, "/").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body().size_hint().exact(), Some(0));
        drop(response);
        assert!(finished(events).is_empty(), "{}", events.text());
        drop(kept.lock().unwrap().take());
    });
    let event = assert_finished(&capture, "INFO", "dropped");
    assert!(event.contains("bytes=0"), "{event}");
}

/// Serve one in-process request whose handler's tuple adds `length` after the
/// hand-over, and drain the body as a collecting holder does.
fn drained_with_late_length(
    capture: &Capture,
    length: &'static str,
    source: fn() -> Body,
) -> (String, Option<String>) {
    paused(capture, async move {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(move |admitted: AdmittedRequest| async move {
                    let started =
                        admitted.into_streaming_response(budget(60, 5), move |_scope| source());
                    ([(axum::http::header::CONTENT_LENGTH, length)], started)
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        super::support::drain(response.body_mut()).await
    })
}

/// The rule is the same in-process as on a server: a length the handler's
/// tuple adds after the hand-over is not the builder's, so a source that ran
/// to its end short of it is `completed` here too, as
/// `sockets::undeclared` shows through Hyper.
#[test]
fn a_length_added_after_the_hand_over_is_ignored_in_process_too() {
    let capture = Capture::new();
    let drained = drained_with_late_length(&capture, "10", || {
        Body::from_stream(futures_util::stream::iter([Ok::<_, Infallible>(
            Bytes::from_static(b"hello"),
        )]))
    });
    assert_eq!(drained, ("hello".to_owned(), None));
    let event = assert_finished(&capture, "INFO", "completed");
    assert!(event.contains("bytes=5"), "{event}");
}

/// The same rule does not make the same outcome. What a body meets depends on
/// its holder: Hyper, framing by a late five-byte length, stops polling and
/// drops this body (`dropped`, in `sockets::undeclared`), while a holder that
/// collects the in-process response polls on, and the source that never ends
/// then runs into its idle bound. An in-process result is evidence for the
/// rule, not for what a served response reports.
#[test]
fn a_collecting_holder_meets_an_outcome_a_transport_that_stops_does_not() {
    let capture = Capture::new();
    let drained = drained_with_late_length(&capture, "5", || {
        let first = futures_util::stream::iter([Ok::<_, Infallible>(Bytes::from_static(b"hello"))]);
        Body::from_stream(futures_util::StreamExt::chain(
            first,
            futures_util::stream::pending(),
        ))
    });
    assert_eq!(
        drained,
        (
            "hello".to_owned(),
            Some("response body ended: idle_timeout".to_owned())
        )
    );
    let event = assert_finished(&capture, "WARN", "idle_timeout");
    assert!(event.contains("bytes=5"), "{event}");
}
