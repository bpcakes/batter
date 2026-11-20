//! The polling task's waker, which a pending body's wait holds: who is
//! woken when a question unwinds, and that every path out of the ending
//! releases it.

use super::*;

/// A task waker that counts how often it is woken.
#[derive(Default)]
struct CountsWakes(AtomicUsize);

impl std::task::Wake for CountsWakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// Pending once, then trailers. Its destruction panics if asked to.
struct PendingThenTrailers {
    polled: bool,
    _destroyed: Marker,
    _panics: Option<PanicsWhenDestroyed>,
}

impl HttpBody for PendingThenTrailers {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        if std::mem::replace(&mut self.polled, true) {
            Poll::Ready(Some(Ok(Frame::trailers(HeaderMap::new()))))
        } else {
            Poll::Pending
        }
    }
}

/// Poll `body` once with `waker`.
fn poll_with(
    body: &mut Body,
    waker: &std::task::Waker,
) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
    Pin::new(body).poll_frame(&mut Context::from_waker(waker))
}

/// A response whose body is `source`, with the context its scope handed out.
async fn served(source: fn() -> Body) -> (axum::response::Response, Escaped) {
    let handle = ready();
    let escaped = Escaped::default();
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
    (get(&app, "/").await, escaped)
}

fn pending_then_trailers() -> Body {
    Body::new(PendingThenTrailers {
        polled: false,
        _destroyed: Marker,
        _panics: Some(PanicsWhenDestroyed),
    })
}

/// A question that unwinds was the body's last call into its source, and
/// its scope is cancelled once that unwind is caught. Another task may be
/// waiting in a pending poll of the same body: cancelling wakes it, so it
/// polls again and gets the body's end instead of waiting for a wakeup that
/// would never come.
#[test]
fn a_question_that_unwinds_still_wakes_a_pending_poll() {
    let capture = Capture::new();
    let task = Arc::new(CountsWakes::default());
    let (unwound, woken, next) = paused(&capture, async {
        let (mut response, _escaped) = served(|| Body::new(PanicsWhenAsked)).await;
        let waker = std::task::Waker::from(task.clone());
        assert!(poll_with(response.body_mut(), &waker).is_pending());
        let unwound = catch_unwind(AssertUnwindSafe(|| response.body().is_end_stream())).is_err();
        let woken = task.0.load(Ordering::SeqCst);
        let next = match poll_with(response.body_mut(), &waker) {
            Poll::Ready(Some(Err(error))) => std::error::Error::source(&error).unwrap().to_string(),
            _ => String::from("not an error"),
        };
        (unwound, woken, next)
    });
    assert!(unwound);
    assert!(woken >= 1, "{woken}");
    assert_eq!(next, "response body ended: panicked");
    assert_finished(&capture, "WARN", "panicked");
}

#[test]
fn yielding_data_releases_the_previous_poller_before_the_next_frame_wait() {
    let capture = Capture::new();
    paused(&capture, async {
        let (mut response, _escaped) = served(|| {
            let mut polls = 0;
            Body::from_stream(futures_util::stream::poll_fn(move |_| {
                polls += 1;
                if polls == 2 {
                    Poll::Ready(Some(Ok::<_, Infallible>(Bytes::from_static(b"data"))))
                } else {
                    Poll::Pending
                }
            }))
        })
        .await;
        let body = response.body_mut();
        let old = Arc::new(CountsWakes::default());
        assert!(poll_with(body, &old.clone().into()).is_pending());
        let Poll::Ready(Some(Ok(frame))) = poll_with(body, std::task::Waker::noop()) else {
            panic!("expected the data frame");
        };
        assert_eq!(frame.into_data().unwrap(), "data");
        assert_eq!(Arc::strong_count(&old), 1, "old waker still registered");
        let allowance = std::time::Duration::from_secs(6);
        tokio::time::sleep(allowance).await;
        assert_eq!(old.0.load(Ordering::SeqCst), 0);
        let next = Arc::new(CountsWakes::default());
        assert!(poll_with(body, &next.clone().into()).is_pending());
        tokio::time::sleep(allowance).await;
        assert!(
            next.0.load(Ordering::SeqCst) > 0,
            "next wait must wake its poller"
        );
    });
}

/// A task waker that does nothing, counted by its handles.
struct Quiet;

#[allow(clippy::manual_noop_waker, reason = "its handles are counted")]
impl std::task::Wake for Quiet {
    fn wake(self: Arc<Self>) {}
}

/// The wait, and the task's waker in it, are released on every path out of
/// the ending. Here the source's destruction unwinds: a holder that catches
/// it and keeps the exhausted response keeps no handle to the task that
/// polled it.
#[test]
fn an_ending_that_unwinds_still_releases_the_polling_tasks_waker() {
    let capture = Capture::new();
    let task = Arc::new(Quiet);
    let (held_while_pending, unwound, held_after) = paused(&capture, async {
        let (mut response, _escaped) = served(pending_then_trailers).await;
        let waker = std::task::Waker::from(task.clone());
        assert!(poll_with(response.body_mut(), &waker).is_pending());
        drop(waker);
        let held_while_pending = Arc::strong_count(&task);
        let unwound = catch_unwind(AssertUnwindSafe(|| {
            poll_with(response.body_mut(), std::task::Waker::noop())
        }))
        .is_err();
        // The response is still held.
        let held_after = Arc::strong_count(&task);
        drop(response);
        (held_while_pending, unwound, held_after)
    });
    assert!(held_while_pending > 1, "{held_while_pending}");
    assert!(unwound);
    assert_eq!(held_after, 1);
}

const WAKER_RELEASED: &str = "polling task's waker released";

/// A task waker that marks the destruction of its last handle.
struct MarksItsRelease;

#[allow(
    clippy::manual_noop_waker,
    reason = "the waker exists for its destructor"
)]
impl std::task::Wake for MarksItsRelease {
    fn wake(self: Arc<Self>) {}
}

impl Drop for MarksItsRelease {
    fn drop(&mut self) {
        tracing::info!("{WAKER_RELEASED}");
    }
}

/// A pending body's wait is created, polled and destroyed under one
/// dispatcher, the one the body was handed over under, whichever subscriber
/// the task that polls or drops it has current. A timer can carry tracing
/// state of its own, as Tokio's does when built with it, and that state
/// must be released by the subscriber it was created under. Here the wait's
/// release is seen through the task waker it holds.
#[test]
fn a_pending_bodys_wait_is_released_under_the_hand_over_subscriber() {
    let requests = Capture::new();
    // Both at TRACE, so that a timer's own tracing state, where the build
    // has any, exists under them.
    let handed_over = Capture::with_max_level(tracing::Level::TRACE);
    let polling = Capture::with_max_level(tracing::Level::TRACE);
    paused(&requests, async {
        let handle = ready();
        let under = handed_over.dispatch.clone();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(move |admitted: AdmittedRequest| async move {
                    tracing::dispatcher::with_default(&under, || {
                        admitted.into_streaming_response(budget(60, 5), |_scope| {
                            Body::new(PendingThenTrailers {
                                polled: false,
                                _destroyed: Marker,
                                _panics: None,
                            })
                        })
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        let waker = std::task::Waker::from(Arc::new(MarksItsRelease));
        tracing::dispatcher::with_default(&polling.dispatch, || {
            assert!(poll_with(response.body_mut(), &waker).is_pending());
            drop(waker);
            drop(response);
        });
    });
    let text = handed_over.text();
    assert!(text.contains(WAKER_RELEASED), "{text}");
    for elsewhere in [&requests, &polling] {
        let text = elsewhere.text();
        assert!(!text.contains(WAKER_RELEASED), "{text}");
    }
    assert_finished(&requests, "INFO", "dropped");
}
