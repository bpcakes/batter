//! What a source yielded that its poll has yet to return, trailers or a
//! stream error, when the source's own destruction unwinds as the body ends:
//! it is destroyed where the source was, and the holder is told.

use super::*;

const UNRETURNED_DESTROYED: &str = "unreturned answer destroyed";

/// The bytes of a trailer value, which mark their destruction.
struct TrailerValue;

impl AsRef<[u8]> for TrailerValue {
    fn as_ref(&self) -> &[u8] {
        b"done"
    }
}

impl Drop for TrailerValue {
    fn drop(&mut self) {
        tracing::info!("{UNRETURNED_DESTROYED}");
    }
}

/// A stream error, which marks its destruction.
#[derive(Debug)]
struct MarkedError;

impl std::fmt::Display for MarkedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("source failed")
    }
}

impl std::error::Error for MarkedError {}

impl Drop for MarkedError {
    fn drop(&mut self) {
        tracing::info!("{UNRETURNED_DESTROYED}");
    }
}

/// At once, the last thing a source yields: trailers whose value is backed
/// by [`TrailerValue`], or a [`MarkedError`].
struct EndsAtOnce {
    with_trailers: bool,
    _destroyed: Marker,
    /// Dropped after the marker: the source's destruction panics.
    _panics: PanicsWhenDestroyed,
}

impl HttpBody for EndsAtOnce {
    type Data = Bytes;
    type Error = MarkedError;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, MarkedError>>> {
        if !self.with_trailers {
            return Poll::Ready(Some(Err(MarkedError)));
        }
        let value = HeaderValue::from_maybe_shared(Bytes::from_owner(TrailerValue)).unwrap();
        Poll::Ready(Some(Ok(Frame::trailers(HeaderMap::from_iter([(
            HeaderName::from_static("x-digest"),
            value,
        )])))))
    }
}

/// The ending can also unwind in the source's own destructor, with nothing
/// wrong with the subscriber. What the poll then never returns is destroyed
/// where the source was: under the hand-over dispatcher and still inside the
/// request span.
#[test]
fn an_answer_a_panicking_source_destructor_kept_back_is_destroyed_inside_the_request_span() {
    for with_trailers in [true, false] {
        let capture = Capture::new();
        let (unwound, at_end, later, exhausted) = paused(&capture, async {
            let handle = ready();
            let app = client(
                &handle,
                GuardedRouter::new().route(
                    "/",
                    route(move |admitted: AdmittedRequest| async move {
                        admitted.into_streaming_response(budget(60, 5), move |_scope| {
                            Body::new(EndsAtOnce {
                                with_trailers,
                                _destroyed: Marker,
                                _panics: PanicsWhenDestroyed,
                            })
                        })
                    }),
                ),
            )
            .await;
            let mut response = get(&app, "/").await;
            let unwound = poll_fn(|cx| {
                let body = Pin::new(response.body_mut());
                Poll::Ready(catch_unwind(AssertUnwindSafe(|| body.poll_frame(cx))).is_err())
            })
            .await;
            // The response is still held.
            let at_end = response.body().is_end_stream();
            let mut poll = async || {
                poll_fn(|cx| Pin::new(response.body_mut()).poll_frame(cx))
                    .await
                    .map(|frame| {
                        frame
                            .map(|_| ())
                            .map_err(|error| std::error::Error::source(&error).unwrap().to_string())
                    })
            };
            let later = (poll().await, poll().await);
            let exhausted = (
                response.body().is_end_stream(),
                response.body().size_hint().exact(),
            );
            (unwound, at_end, later, exhausted)
        });
        assert!(unwound, "trailers: {with_trailers}");
        let text = capture.text();
        let kept_back = text
            .lines()
            .find(|line| line.contains(UNRETURNED_DESTROYED))
            .unwrap_or_else(|| panic!("trailers: {with_trailers}: {text}"));
        assert!(kept_back.contains("batter.http{"), "{text}");
        // The poll that unwound returned nothing, so the body is not at a
        // clean end: the next poll says so once, and only then is it over.
        assert!(!at_end, "trailers: {with_trailers}");
        assert_eq!(
            later,
            (Some(Err("response body ended: panicked".to_owned())), None),
            "trailers: {with_trailers}"
        );
        assert_eq!(exhausted, (true, Some(0)), "trailers: {with_trailers}");
        // The ending had emitted its one event before the destructor ran.
        let outcome = if with_trailers { "completed" } else { "failed" };
        let level = if with_trailers { "INFO" } else { "WARN" };
        assert_finished(&capture, level, outcome);
    }
}

/// The bytes of a trailer value whose destruction always panics.
struct TrailerValuePanics;

impl AsRef<[u8]> for TrailerValuePanics {
    fn as_ref(&self) -> &[u8] {
        b"done"
    }
}

impl Drop for TrailerValuePanics {
    fn drop(&mut self) {
        panic!("trailer value destroyed");
    }
}

/// Trailers at once, backed by [`TrailerValuePanics`], from a source whose
/// own destruction panics too.
struct BothPanic {
    /// How long the poll that yields the trailers blocks first.
    blocks: std::time::Duration,
    _destroyed: Marker,
    _panics: PanicsWhenDestroyed,
}

impl HttpBody for BothPanic {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        std::thread::sleep(self.blocks);
        let value = HeaderValue::from_maybe_shared(Bytes::from_owner(TrailerValuePanics)).unwrap();
        Poll::Ready(Some(Ok(Frame::trailers(HeaderMap::from_iter([(
            HeaderName::from_static("x-digest"),
            value,
        )])))))
    }
}

/// The source and what it yielded are both the application's, and both
/// destructors may panic in the same ending. The second one runs with the
/// first unwind caught, so it replaces that panic: the holder catches an
/// unwind as before, where a destructor panicking inside the first unwind
/// would have aborted the process.
#[test]
fn a_kept_answer_whose_destructor_panics_after_the_sources_does_not_abort() {
    let capture = Capture::new();
    let (unwound, next) = paused(&capture, async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(60, 5), |_scope| {
                        Body::new(BothPanic {
                            blocks: std::time::Duration::ZERO,
                            _destroyed: Marker,
                            _panics: PanicsWhenDestroyed,
                        })
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        let unwound = poll_fn(|cx| {
            let body = Pin::new(response.body_mut());
            Poll::Ready(catch_unwind(AssertUnwindSafe(|| body.poll_frame(cx))).is_err())
        })
        .await;
        let next = poll_fn(|cx| Pin::new(response.body_mut()).poll_frame(cx))
            .await
            .map(|frame| {
                frame
                    .map(|_| ())
                    .map_err(|error| std::error::Error::source(&error).unwrap().to_string())
            });
        (unwound, next)
    });
    assert!(unwound);
    assert_eq!(next, Some(Err("response body ended: panicked".to_owned())));
    assert_finished(&capture, "INFO", "completed");
    let text = capture.text();
    assert!(text.contains(DESTROYED), "{text}");
}

/// The same when the trailers arrive too late: the poll that yields them
/// outlasts the idle bound, so the library rejects them and destroys them
/// itself, with the source. They are destroyed apart from it, each with an
/// unwind from the other caught. On real time, because a paused clock
/// cannot move during a synchronous call.
#[test]
fn a_rejected_answer_whose_destructor_panics_after_the_sources_does_not_abort() {
    let capture = Capture::new();
    let (unwound, next) = capture.block_on(async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    let budget = batter_axum::BodyBudget::new(
                        std::time::Duration::from_secs(60),
                        std::time::Duration::from_millis(50),
                    )
                    .unwrap();
                    admitted.into_streaming_response(budget, |_scope| {
                        Body::new(BothPanic {
                            blocks: std::time::Duration::from_millis(250),
                            _destroyed: Marker,
                            _panics: PanicsWhenDestroyed,
                        })
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        let unwound = poll_fn(|cx| {
            let body = Pin::new(response.body_mut());
            Poll::Ready(catch_unwind(AssertUnwindSafe(|| body.poll_frame(cx))).is_err())
        })
        .await;
        let next = poll_fn(|cx| Pin::new(response.body_mut()).poll_frame(cx))
            .await
            .map(|frame| {
                frame
                    .map(|_| ())
                    .map_err(|error| std::error::Error::source(&error).unwrap().to_string())
            });
        (unwound, next)
    });
    assert!(unwound);
    assert_eq!(next, Some(Err("response body ended: panicked".to_owned())));
    assert_finished(&capture, "WARN", "idle_timeout");
    let text = capture.text();
    assert!(text.contains(DESTROYED), "{text}");
}

const PAYLOAD_DESTROYED: &str = "superseded panic payload destroyed";

/// A panic payload that marks its destruction.
struct MarkedPayload;

impl Drop for MarkedPayload {
    fn drop(&mut self) {
        tracing::info!("{PAYLOAD_DESTROYED}");
    }
}

/// Panics with a [`MarkedPayload`] when destroyed.
struct PanicsWithAPayload;

impl Drop for PanicsWithAPayload {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            std::panic::panic_any(MarkedPayload);
        }
    }
}

/// Trailers at once, backed by [`TrailerValuePanics`], from a source whose
/// own destruction panics with a [`MarkedPayload`].
struct BothPanicWithPayloads {
    _panics: PanicsWithAPayload,
}

impl HttpBody for BothPanicWithPayloads {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        let value = HeaderValue::from_maybe_shared(Bytes::from_owner(TrailerValuePanics)).unwrap();
        Poll::Ready(Some(Ok(Frame::trailers(HeaderMap::from_iter([(
            HeaderName::from_static("x-digest"),
            value,
        )])))))
    }
}

/// When a second destructor's panic replaces the first, the first panic's
/// payload is the application's as well. It is destroyed where the values
/// were: under the dispatcher the body was handed over under, not under the
/// request's.
#[test]
fn a_superseded_panic_payload_is_destroyed_under_the_hand_over_dispatcher() {
    let requests = Capture::new();
    let handed_over = Capture::new();
    let unwound = paused(&requests, async {
        let handle = ready();
        let under = handed_over.dispatch.clone();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(move |admitted: AdmittedRequest| async move {
                    tracing::dispatcher::with_default(&under, || {
                        admitted.into_streaming_response(budget(60, 5), |_scope| {
                            Body::new(BothPanicWithPayloads {
                                _panics: PanicsWithAPayload,
                            })
                        })
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        poll_fn(|cx| {
            let body = Pin::new(response.body_mut());
            Poll::Ready(catch_unwind(AssertUnwindSafe(|| body.poll_frame(cx))).is_err())
        })
        .await
    });
    assert!(unwound);
    let text = handed_over.text();
    assert!(text.contains(PAYLOAD_DESTROYED), "{text}");
    let elsewhere = requests.text();
    assert!(!elsewhere.contains(PAYLOAD_DESTROYED), "{elsewhere}");
}
