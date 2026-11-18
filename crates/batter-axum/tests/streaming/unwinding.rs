//! Application code of a body that unwinds, or that is never needed: an
//! unwind a holder caught is the body's end whatever the source would yield
//! next, and the hand-over asks a body for its size only where that decides.
//! `kept` covers what the source yielded that a poll has yet to return when
//! the source's destruction unwinds, and `wakers` the polling task's waker.
//!
//! These cases panic in the application's own code for the body. A tracing
//! subscriber or a task waker that panics is outside the library's contract.

use super::support::{
    DESTROYED, Escaped, Marker, assert_finished, budget, client, get, paused, ready,
};
use crate::capture::Capture;
use axum::{
    body::{Body, Bytes, HttpBody},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::IntoResponse,
    routing::get as route,
};
use batter_axum::{AdmittedRequest, GuardedRouter};
use batter_core::operation::Interruption;
use http_body::{Frame, SizeHint};
use std::{
    convert::Infallible,
    future::poll_fn,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

const ENTERED_AFTER_UNWIND: &str = "source entered after its call unwound";

/// The empty buffer of a frame, which panics when destroyed.
struct PanicsWhenDestroyed;

impl AsRef<[u8]> for PanicsWhenDestroyed {
    fn as_ref(&self) -> &[u8] {
        &[]
    }
}

impl Drop for PanicsWhenDestroyed {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            panic!("empty frame destroyed");
        }
    }
}

/// An empty frame whose destruction panics, then trailers. It reports every
/// entry that follows the first poll.
struct PanicsThenTrailers {
    polled: bool,
}

impl HttpBody for PanicsThenTrailers {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        if std::mem::replace(&mut self.polled, true) {
            tracing::info!("{ENTERED_AFTER_UNWIND}");
            return Poll::Ready(Some(Ok(Frame::trailers(HeaderMap::new()))));
        }
        Poll::Ready(Some(Ok(Frame::data(Bytes::from_owner(
            PanicsWhenDestroyed,
        )))))
    }

    fn is_end_stream(&self) -> bool {
        if self.polled {
            tracing::info!("{ENTERED_AFTER_UNWIND}");
        }
        false
    }

    fn size_hint(&self) -> SizeHint {
        if self.polled {
            tracing::info!("{ENTERED_AFTER_UNWIND}");
        }
        SizeHint::default()
    }
}

/// A source whose first poll panics. It reports every entry that follows.
struct PanicsWhenPolled {
    polled: bool,
}

impl HttpBody for PanicsWhenPolled {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        if std::mem::replace(&mut self.polled, true) {
            tracing::info!("{ENTERED_AFTER_UNWIND}");
            return Poll::Ready(None);
        }
        panic!("source polled");
    }

    fn is_end_stream(&self) -> bool {
        if self.polled {
            tracing::info!("{ENTERED_AFTER_UNWIND}");
        }
        false
    }

    fn size_hint(&self) -> SizeHint {
        if self.polled {
            tracing::info!("{ENTERED_AFTER_UNWIND}");
        }
        SizeHint::default()
    }
}

/// A holder may catch the unwind of a body's call and poll on. The unwind
/// was the body's end: its scope is cancelled at once, the next poll returns
/// that end as an error, the queries in between answer without the source,
/// which is never entered again, and the one event is `panicked`. That holds whether the source's
/// own poll unwound or the destruction of an empty frame it had yielded,
/// which the owner tracks apart, and whatever the source would yield next:
/// its end, or trailers that would otherwise complete the body.
#[test]
fn a_body_whose_call_unwound_ends_as_panicked_for_a_holder_that_polls_on() {
    let sources: [fn() -> Body; 2] = [
        || Body::new(PanicsThenTrailers { polled: false }),
        || Body::new(PanicsWhenPolled { polled: false }),
    ];
    for (case, source) in sources.into_iter().enumerate() {
        let capture = Capture::new();
        let escaped = Escaped::default();
        let (unwound, cancelled_at_once, at_end, sized, second, third) = paused(&capture, async {
            let handle = ready();
            let kept = escaped.clone();
            let app = client(
                &handle,
                GuardedRouter::new().route(
                    "/",
                    route(move |admitted: AdmittedRequest| async move {
                        admitted.into_streaming_response(budget(60, 5), move |scope| {
                            kept.keep(scope.context());
                            source()
                        })
                    }),
                ),
            )
            .await;
            let mut response = get(&app, "/").await;
            assert!(escaped.context().check().is_ok());
            let unwound = poll_fn(|cx| {
                let body = Pin::new(response.body_mut());
                Poll::Ready(catch_unwind(AssertUnwindSafe(|| body.poll_frame(cx))).is_err())
            })
            .await;
            // Before the body is touched again.
            let cancelled_at_once = escaped.context().check();
            let at_end = response.body().is_end_stream();
            let sized = response.body().size_hint().exact();
            let mut poll = async || {
                poll_fn(|cx| Pin::new(response.body_mut()).poll_frame(cx))
                    .await
                    .map(|frame| {
                        frame
                            .map(|_| ())
                            .map_err(|error| std::error::Error::source(&error).unwrap().to_string())
                    })
            };
            let second = poll().await;
            let third = poll().await;
            let exhausted = (
                response.body().is_end_stream(),
                response.body().size_hint().exact(),
            );
            (
                unwound,
                cancelled_at_once,
                at_end,
                sized,
                second,
                (third, exhausted),
            )
        });
        assert!(unwound, "case {case}");
        assert_eq!(
            cancelled_at_once,
            Err(Interruption::Cancelled),
            "case {case}"
        );
        assert!(!at_end, "case {case}");
        assert_eq!(sized, None, "case {case}");
        assert_eq!(
            second,
            Some(Err("response body ended: panicked".to_owned())),
            "case {case}"
        );
        // Once the error was delivered the body is at its end like any
        // other: no more frames, ended, and nothing left to send.
        assert_eq!(third, (None, (true, Some(0))), "case {case}");
        assert_finished(&capture, "WARN", "panicked");
        let text = capture.text();
        assert!(!text.contains(ENTERED_AFTER_UNWIND), "case {case}: {text}");
    }
}

const ASKED: &str = "source asked for its end or size";

/// `hello`, then its end. It marks its destruction and every question.
struct Watched {
    sent: bool,
    _destroyed: Marker,
}

impl HttpBody for Watched {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        if std::mem::replace(&mut self.sent, true) {
            return Poll::Ready(None);
        }
        Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"hello")))))
    }

    fn is_end_stream(&self) -> bool {
        tracing::info!("{ASKED}");
        false
    }

    fn size_hint(&self) -> SizeHint {
        tracing::info!("{ASKED}");
        SizeHint::default()
    }
}

/// A body whose size query panics.
struct UnaskableSize;

impl HttpBody for UnaskableSize {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        Poll::Pending
    }

    fn size_hint(&self) -> SizeHint {
        panic!("size asked");
    }
}

/// The hand-over asks a body for its size only to learn a length nothing
/// else declares. Where the built response states a `Content-Length`, or
/// carries no content, the size decides nothing and is not asked: a handler
/// that keeps such a body and returns another response is not unwound by a
/// query nobody needed.
#[test]
fn the_hand_over_asks_no_size_the_declaration_does_not_need() {
    let capture = Capture::new();
    let kept: Arc<Mutex<Vec<Body>>> = Arc::default();
    let events = &capture;
    paused(events, {
        let kept = kept.clone();
        async move {
            let handle = ready();
            let lengthy = kept.clone();
            let contentless = kept.clone();
            let app = client(
                &handle,
                GuardedRouter::new()
                    .route(
                        "/declared",
                        route(move |admitted: AdmittedRequest| async move {
                            let started =
                                admitted.into_streaming_response(budget(60, 5), |_scope| {
                                    ([(header::CONTENT_LENGTH, "5")], Body::new(UnaskableSize))
                                });
                            lengthy.lock().unwrap().push(started.into_body());
                            StatusCode::OK
                        }),
                    )
                    .route(
                        "/contentless",
                        route(move |admitted: AdmittedRequest| async move {
                            let started =
                                admitted.into_streaming_response(budget(60, 5), |_scope| {
                                    (StatusCode::NO_CONTENT, Body::new(UnaskableSize))
                                        .into_response()
                                });
                            contentless.lock().unwrap().push(started.into_body());
                            StatusCode::OK
                        }),
                    ),
            )
            .await;
            assert_eq!(get(&app, "/declared").await.status(), StatusCode::OK);
            assert_eq!(get(&app, "/contentless").await.status(), StatusCode::OK);
        }
    });
    assert_eq!(kept.lock().unwrap().len(), 2);
    let text = capture.text();
    assert!(!text.contains("panicked"), "{text}");
}

/// A body can also be destroyed while its holder unwinds for a reason of its
/// own. Nothing is wrong with the subscriber then, so the source is still
/// destroyed inside the request span, after the `panicked` event.
#[test]
fn a_body_destroyed_while_its_holder_unwinds_keeps_its_span() {
    let capture = Capture::new();
    paused(&capture, async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(60, 5), |_scope| {
                        Body::new(Watched {
                            sent: false,
                            _destroyed: Marker,
                        })
                    })
                }),
            ),
        )
        .await;
        let response = get(&app, "/").await;
        let unwound = catch_unwind(AssertUnwindSafe(move || {
            let _held = response;
            panic!("holder panicked");
        }));
        assert!(unwound.is_err());
    });
    assert_finished(&capture, "WARN", "panicked");
    let text = capture.text();
    let destroyed = text.lines().find(|line| line.contains(DESTROYED)).unwrap();
    assert!(destroyed.contains("batter.http{"), "{text}");
}

/// A source that panics when asked whether it has ended.
struct PanicsWhenAsked;

impl HttpBody for PanicsWhenAsked {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        Poll::Pending
    }

    fn is_end_stream(&self) -> bool {
        panic!("end asked");
    }
}

/// A question unwinds like a poll: the scope is cancelled at once, while the
/// holder that caught the unwind still keeps the body untouched.
#[test]
fn a_question_that_unwinds_cancels_the_scope_at_once() {
    let capture = Capture::new();
    let escaped = Escaped::default();
    let (unwound, cancelled_at_once) = paused(&capture, async {
        let handle = ready();
        let kept = escaped.clone();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(move |admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(60, 5), move |scope| {
                        kept.keep(scope.context());
                        Body::new(PanicsWhenAsked)
                    })
                }),
            ),
        )
        .await;
        let response = get(&app, "/").await;
        assert!(escaped.context().check().is_ok());
        let unwound = catch_unwind(AssertUnwindSafe(|| response.body().is_end_stream())).is_err();
        let cancelled_at_once = escaped.context().check();
        drop(response);
        (unwound, cancelled_at_once)
    });
    assert!(unwound);
    assert_eq!(cancelled_at_once, Err(Interruption::Cancelled));
    assert_finished(&capture, "WARN", "panicked");
}

mod kept;
mod wakers;
