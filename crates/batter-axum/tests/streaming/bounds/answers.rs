//! Answers a source gives at the edge of its bounds: late ones, which the
//! library refuses and destroys under protection, and empty frames, which are
//! no progress.

use super::*;
use futures_util::{FutureExt, StreamExt};

/// What a slow source's late poll finally returns.
#[derive(Clone, Copy, Debug)]
enum Late {
    Frame,
    Error,
    /// An error holding a span opened during that poll, inside the request
    /// span, which records its destruction.
    Spanned,
}

/// An application error that holds a span and reports where it is destroyed.
#[derive(Debug)]
struct SpannedError(#[allow(dead_code, reason = "held for its destruction")] tracing::Span);

impl std::fmt::Display for SpannedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the step was interrupted")
    }
}

impl std::error::Error for SpannedError {}

impl Drop for SpannedError {
    fn drop(&mut self) {
        tracing::info!("late answer destroyed");
    }
}

/// Answers its first poll with a frame and, if it `waits`, its second with
/// `Pending`, waking itself at once; its next poll blocks the task for
/// `blocks` before answering with `late`, as synchronous work inside a poll
/// may.
struct SlowToAnswer {
    polls: u8,
    waits: bool,
    blocks: Duration,
    late: Late,
}

impl HttpBody for SlowToAnswer {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, std::io::Error>>> {
        self.polls += 1;
        match (self.polls, self.waits) {
            (1, _) => Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"first;"))))),
            (2, true) => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            (2, false) | (3, true) => {
                std::thread::sleep(self.blocks);
                Poll::Ready(Some(match self.late {
                    Late::Frame => Ok(Frame::data(Bytes::from_static(b"late;"))),
                    Late::Error => Err(std::io::Error::other("the step was interrupted")),
                    Late::Spanned => Err(std::io::Error::other(SpannedError(tracing::info_span!(
                        "late.step"
                    )))),
                }))
            }
            _ => Poll::Pending,
        }
    }
}

/// The bounds are checked again once the source answers: a frame returned by
/// a poll that outlasted the idle bound, or the total bound, is not accepted,
/// whether the poll began inside an armed wait or straight after a delivered
/// frame, whose wait then starts with that poll; and an error returned then,
/// as a step interrupted by the body's own deadline yields, is reported as
/// that bound. On real time, because a paused clock cannot move during a poll.
#[test]
fn a_frame_returned_by_a_poll_that_outlasted_a_bound_is_not_accepted() {
    let blocks = Duration::from_millis(400);
    let cases = [(60_000, "idle_timeout"), (200, "deadline_exceeded")];
    for (((total, outcome), late), waits) in cases
        .into_iter()
        .flat_map(|case| [(case, Late::Frame), (case, Late::Error)])
        .flat_map(|case| [(case, true), (case, false)])
    {
        let capture = Capture::new();
        let escaped = Escaped::default();
        let (frames, error, _) = capture.block_on({
            let escaped = escaped.clone();
            async move {
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
                            admitted.into_streaming_response(budget, move |scope| {
                                escaped.keep(scope.context());
                                Body::new(SlowToAnswer {
                                    polls: 0,
                                    waits,
                                    blocks,
                                    late,
                                })
                            })
                        }),
                    ),
                )
                .await;
                let mut response = get(&app, "/").await;
                until_error(response.body_mut()).await
            }
        });
        assert_eq!(frames, 1, "{outcome} {late:?} waits={waits}");
        assert_eq!(error, format!("response body ended: {outcome}"));
        assert!(escaped.context().check().is_err());
        let event = assert_finished(&capture, "WARN", outcome);
        assert!(event.contains("bytes=6"), "{event}");
    }
}

/// An answer the source gave too late is destroyed by the library, not the
/// transport, so like the source it is destroyed under the handler's
/// subscriber and inside the request span, even when the transport polls
/// under another subscriber. A span it holds then releases its parents in the
/// registry that recorded them; under the transport's registry, which never
/// saw them, that registry panics.
#[test]
fn a_late_answer_is_destroyed_under_the_handlers_subscriber() {
    let capture = Capture::new();
    let transport = Capture::new();
    let (frames, error, _) = capture.block_on(async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    let budget =
                        BodyBudget::new(Duration::from_secs(60), Duration::from_millis(150))
                            .unwrap();
                    admitted.into_streaming_response(budget, |_scope| {
                        Body::new(SlowToAnswer {
                            polls: 0,
                            waits: true,
                            blocks: Duration::from_millis(400),
                            late: Late::Spanned,
                        })
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/")
            .instrument(tracing::info_span!("caller"))
            .await;
        async move {
            let ended = until_error(response.body_mut()).await;
            drop(response);
            ended
        }
        .with_subscriber(transport.dispatch.clone())
        .await
    });
    assert_eq!(frames, 1);
    assert_eq!(error, "response body ended: idle_timeout");
    let text = capture.text();
    let destroyed = text
        .lines()
        .find(|line| line.contains("late answer destroyed"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        destroyed.contains(" caller:") && destroyed.contains(":batter.http{"),
        "{destroyed}"
    );
    assert_finished(&capture, "WARN", "idle_timeout");
    let foreign = transport.text();
    assert!(!foreign.contains("late answer destroyed"), "{foreign}");
}

/// A source always ready with empty data frames makes no progress: Hyper
/// discards each and polls again at once, without yielding. The idle bound
/// still ends the body, and the task yields meanwhile, so other work on a
/// current-thread runtime keeps running.
#[test]
fn a_source_of_empty_frames_is_ended_at_the_idle_bound_and_yields() {
    let capture = Capture::new();
    let ((frames, error, elapsed), ticks) = capture.block_on(async {
        let ticks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let ticker = tokio::spawn({
            let ticks = ticks.clone();
            async move {
                loop {
                    ticks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    tokio::task::yield_now().await;
                }
            }
        });
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    let budget =
                        BodyBudget::new(Duration::from_secs(10), Duration::from_millis(100))
                            .unwrap();
                    admitted.into_streaming_response(budget, |_scope| {
                        Body::from_stream(stream::repeat_with(|| Ok::<_, Infallible>(Bytes::new())))
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        let before = ticks.load(std::sync::atomic::Ordering::Relaxed);
        let ended = until_error(response.body_mut()).await;
        ticker.abort();
        let during = ticks.load(std::sync::atomic::Ordering::Relaxed) - before;
        (ended, during)
    });
    assert_eq!(frames, 0);
    assert_eq!(error, "response body ended: idle_timeout");
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    assert!(ticks > 0);
    let event = assert_finished(&capture, "WARN", "idle_timeout");
    assert!(event.contains("bytes=0"), "{event}");
}

/// Answers with an empty data frame, then data, then its end, each ready at
/// once and without ever waking its task itself.
struct EmptyThenData {
    polls: u8,
}

impl HttpBody for EmptyThenData {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.polls += 1;
        match self.polls {
            1 => Poll::Ready(Some(Ok(Frame::data(Bytes::new())))),
            2 => Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"hello"))))),
            _ => Poll::Ready(None),
        }
    }
}

/// After swallowing an empty frame the body wakes its own task, so data the
/// source has ready follows at once instead of waiting for the idle bound.
#[test]
fn data_after_an_empty_frame_follows_at_once() {
    let capture = Capture::new();
    let (text, error, elapsed) = paused(&capture, async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(60, 5), |_scope| {
                        Body::new(EmptyThenData { polls: 0 })
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        let started = Instant::now();
        let (text, error) = drain(response.body_mut()).await;
        (text, error, started.elapsed())
    });
    assert_eq!(text, "hello");
    assert_eq!(error, None);
    assert!(elapsed < Duration::from_secs(1), "{elapsed:?}");
    let event = assert_finished(&capture, "INFO", "completed");
    assert!(event.contains("bytes=5"), "{event}");
}

/// The empty buffer of a swallowed frame, which reports its destruction.
struct LoggedEmpty;

impl AsRef<[u8]> for LoggedEmpty {
    fn as_ref(&self) -> &[u8] {
        &[]
    }
}

impl Drop for LoggedEmpty {
    fn drop(&mut self) {
        tracing::info!("empty frame destroyed");
    }
}

/// A swallowed empty frame never reaches the transport, so the library
/// destroys it as it polls the source: under the handler's subscriber and
/// inside the request span, even when the transport polls under another.
#[test]
fn a_swallowed_empty_frame_is_destroyed_under_the_handlers_subscriber() {
    let capture = Capture::new();
    let transport = Capture::new();
    let text = paused(&capture, async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(60, 5), |_scope| {
                        Body::from_stream(stream::iter([Ok::<_, Infallible>(Bytes::from_owner(
                            LoggedEmpty,
                        ))]))
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/")
            .instrument(tracing::info_span!("caller"))
            .await;
        async move { drain(response.body_mut()).await.0 }
            .with_subscriber(transport.dispatch.clone())
            .await
    });
    assert_eq!(text, "");
    let log = capture.text();
    let destroyed = log
        .lines()
        .find(|line| line.contains("empty frame destroyed"))
        .unwrap_or_else(|| panic!("{log}"));
    assert!(
        destroyed.contains(" caller:") && destroyed.contains(":batter.http{"),
        "{destroyed}"
    );
    assert!(!transport.text().contains("empty frame destroyed"));
    assert_finished(&capture, "INFO", "completed");
}

/// The empty buffer of a swallowed frame whose destruction panics.
struct PanickingEmpty;

impl AsRef<[u8]> for PanickingEmpty {
    fn as_ref(&self) -> &[u8] {
        &[]
    }
}

impl Drop for PanickingEmpty {
    fn drop(&mut self) {
        panic!("empty frame destructor failed");
    }
}

/// Destroying a swallowed empty frame runs application code after the poll
/// returned. A transport that catches that unwind and only then destroys the
/// body still sees the panic named, not a departed peer.
#[test]
fn a_swallowed_empty_frame_that_panics_when_destroyed_is_observed_as_panicked() {
    let capture = Capture::new();
    let unwound = paused(&capture, async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(60, 5), |_scope| {
                        let empty = Ok::<_, Infallible>(Bytes::from_owner(PanickingEmpty));
                        Body::from_stream(stream::iter([empty]).chain(stream::pending()))
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        let unwound = std::panic::AssertUnwindSafe(next(response.body_mut()))
            .catch_unwind()
            .await
            .is_err();
        drop(response);
        unwound
    });
    assert!(unwound);
    let event = assert_finished(&capture, "WARN", "panicked");
    assert!(event.contains("bytes=0"), "{event}");
}

/// Data, then a final empty frame after which the source reports its end, as
/// a full body mapped frame by frame can.
struct EndsWithEmpty {
    polls: u8,
}

impl HttpBody for EndsWithEmpty {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.polls += 1;
        match self.polls {
            1 => Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"hello"))))),
            2 => Poll::Ready(Some(Ok(Frame::data(Bytes::new())))),
            _ => Poll::Ready(None),
        }
    }

    fn is_end_stream(&self) -> bool {
        self.polls >= 2
    }
}

/// A final empty frame ends the message at once, as Hyper ends it, so a next
/// poll that comes only after the idle bound still finds the body completed.
#[test]
fn a_final_empty_frame_completes_the_body_at_once() {
    let capture = Capture::new();
    let (first, ended_at_once, later) = paused(&capture, async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(60, 5), |_scope| {
                        Body::new(EndsWithEmpty { polls: 0 })
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        let body = response.body_mut();
        let first = next(body).await.expect("a frame").expect("data");
        let after_empty = poll_fn(|cx| Poll::Ready(Pin::new(&mut *body).poll_frame(cx))).await;
        let ended_at_once = matches!(after_empty, Poll::Ready(None));
        tokio::time::advance(Duration::from_secs(6)).await;
        (first, ended_at_once, next(body).await.is_none())
    });
    assert_eq!(first, Bytes::from_static(b"hello"));
    assert!(ended_at_once);
    assert!(later);
    let event = assert_finished(&capture, "INFO", "completed");
    assert!(event.contains("bytes=5"), "{event}");
}
