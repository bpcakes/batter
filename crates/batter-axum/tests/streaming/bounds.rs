//! The idle and total bounds end a body the application left running.

use super::support::{Escaped, assert_finished, budget, client, get, next, paused, ready};
use crate::capture::Capture;
use axum::{
    body::{Body, Bytes, HttpBody},
    routing::get as route,
};
use batter_axum::{AdmittedRequest, BodyBudget, GuardedRouter};
use batter_core::{ConfigurationError, operation::Interruption};
use futures_util::stream;
use http_body::Frame;
use std::{
    convert::Infallible,
    future::poll_fn,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::{sync::mpsc, time::Instant};

/// One chunk every `every`, forever, ignoring every signal it was given.
fn endless(every: Duration) -> Body {
    Body::from_stream(stream::unfold((), move |()| async move {
        tokio::time::sleep(every).await;
        Some((Ok::<_, Infallible>(Bytes::from_static(b"tick;")), ()))
    }))
}

/// Poll frames until the body errors, returning the frames seen, the error and
/// the time it took on the paused clock.
async fn until_error(body: &mut Body) -> (usize, String, Duration) {
    let started = Instant::now();
    let mut frames = 0;
    loop {
        match next(body).await.expect("an endless body cannot complete") {
            Ok(_) => frames += 1,
            Err(error) => {
                let source = std::error::Error::source(&error).unwrap().to_string();
                return (frames, source, started.elapsed());
            }
        }
    }
}

#[test]
fn budget_rejects_zero_unrepresentable_and_inverted_bounds() {
    let second = Duration::from_secs(1);
    assert_eq!(
        BodyBudget::new(Duration::ZERO, second),
        Err(ConfigurationError::Zero("HTTP body total budget"))
    );
    assert_eq!(
        BodyBudget::new(second, Duration::ZERO),
        Err(ConfigurationError::Zero("HTTP body idle budget"))
    );
    assert_eq!(
        BodyBudget::new(Duration::MAX, second),
        Err(ConfigurationError::TooLarge("HTTP body total budget"))
    );
    // The total is capped at one year, well inside what Tokio can represent.
    let year = Duration::from_secs(365 * 24 * 60 * 60);
    assert_eq!(
        BodyBudget::new(year, second).map(BodyBudget::total),
        Ok(year)
    );
    assert_eq!(
        BodyBudget::new(year + second, second),
        Err(ConfigurationError::TooLarge("HTTP body total budget"))
    );
    assert_eq!(
        BodyBudget::new(second, 2 * second),
        Err(ConfigurationError::TooLarge("HTTP body idle budget"))
    );
    assert!(BodyBudget::new(second, second).is_ok());
}

#[test]
fn a_source_that_stops_producing_is_ended_at_the_idle_bound() {
    let capture = Capture::new();
    let escaped = Escaped::default();
    let (frames, error, elapsed) = paused(&capture, {
        let escaped = escaped.clone();
        async move {
            let handle = ready();
            let app = client(
                &handle,
                GuardedRouter::new().route(
                    "/",
                    route(move |admitted: AdmittedRequest| async move {
                        admitted.into_streaming_response(budget(60, 5), |scope| {
                            escaped.keep(scope.context());
                            // One frame, then nothing.
                            Body::from_stream(stream::unfold(true, |first| async move {
                                if !first {
                                    std::future::pending::<()>().await;
                                }
                                Some((Ok::<_, Infallible>(Bytes::from_static(b"first;")), false))
                            }))
                        })
                    }),
                ),
            )
            .await;
            let mut response = get(&app, "/").await;
            until_error(response.body_mut()).await
        }
    });
    assert_eq!(frames, 1);
    assert_eq!(error, "response body ended: idle_timeout");
    assert_eq!(elapsed, Duration::from_secs(5));
    assert_eq!(escaped.context().check(), Err(Interruption::Cancelled));
    let event = assert_finished(&capture, "WARN", "idle_timeout");
    assert!(event.contains("bytes=6"), "{event}");
}

/// The idle bound is checked before a frame is accepted: a source that
/// becomes ready only after its armed wait expired, before the body was
/// polled again, does not extend the body.
#[test]
fn a_frame_ready_only_after_the_idle_bound_expired_is_not_accepted() {
    let capture = Capture::new();
    let late = paused(&capture, async {
        let handle = ready();
        let (sender, receiver) = mpsc::unbounded_channel::<Bytes>();
        let receiver = Arc::new(Mutex::new(Some(receiver)));
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(move |admitted: AdmittedRequest| {
                    let receiver = receiver.lock().unwrap().take().unwrap();
                    async move {
                        admitted.into_streaming_response(budget(60, 5), move |_scope| {
                            Body::from_stream(stream::unfold(receiver, |mut receiver| async {
                                let frame = receiver.recv().await?;
                                Some((Ok::<_, Infallible>(frame), receiver))
                            }))
                        })
                    }
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        let body = response.body_mut();
        // Nothing is ready, so the first poll arms the idle bound.
        let first = poll_fn(|cx| Poll::Ready(Pin::new(&mut *body).poll_frame(cx))).await;
        assert!(first.is_pending());
        // The wait expires, and only then does the source become ready.
        tokio::time::advance(Duration::from_secs(6)).await;
        sender.send(Bytes::from_static(b"late;")).unwrap();
        next(body).await.expect("the body ended with an error")
    });
    let error = late.expect_err("a late frame was accepted");
    let source = std::error::Error::source(&error).unwrap().to_string();
    assert_eq!(source, "response body ended: idle_timeout");
    let event = assert_finished(&capture, "WARN", "idle_timeout");
    assert!(event.contains("bytes=0"), "{event}");
}

/// Answers its first poll with a frame and its second with `Pending`, waking
/// itself at once; its third poll blocks the task for `blocks` before
/// answering with another frame, as synchronous work inside a poll may.
struct SlowToAnswer {
    polls: u8,
    blocks: Duration,
}

impl HttpBody for SlowToAnswer {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.polls += 1;
        match self.polls {
            1 => Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"first;"))))),
            2 => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            3 => {
                std::thread::sleep(self.blocks);
                Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"late;")))))
            }
            _ => Poll::Pending,
        }
    }
}

/// The bounds are checked again once the source answers: a frame returned by
/// a poll that began inside the armed wait but outlasted the idle bound, or
/// the total bound, is not accepted. On real time, because a paused clock
/// cannot move during a poll.
#[test]
fn a_frame_returned_by_a_poll_that_outlasted_a_bound_is_not_accepted() {
    let blocks = Duration::from_millis(400);
    for (total, outcome) in [(60_000, "idle_timeout"), (200, "deadline_exceeded")] {
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
                                Duration::from_millis(200),
                            )
                            .unwrap();
                            admitted.into_streaming_response(budget, move |scope| {
                                escaped.keep(scope.context());
                                Body::new(SlowToAnswer { polls: 0, blocks })
                            })
                        }),
                    ),
                )
                .await;
                let mut response = get(&app, "/").await;
                until_error(response.body_mut()).await
            }
        });
        assert_eq!(frames, 1, "{outcome}");
        assert_eq!(error, format!("response body ended: {outcome}"));
        assert!(escaped.context().check().is_err());
        let event = assert_finished(&capture, "WARN", outcome);
        assert!(event.contains("bytes=6"), "{event}");
    }
}

#[test]
fn a_source_that_never_stops_is_ended_at_the_total_bound() {
    let capture = Capture::new();
    let (frames, error, elapsed) = paused(&capture, async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    // Every wait is well inside the idle bound.
                    admitted.into_streaming_response(budget(10, 5), |_scope| {
                        endless(Duration::from_secs(3))
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        until_error(response.body_mut()).await
    });
    assert_eq!(frames, 3);
    assert_eq!(error, "response body ended: deadline_exceeded");
    assert_eq!(elapsed, Duration::from_secs(10));
    let event = assert_finished(&capture, "WARN", "deadline_exceeded");
    assert!(event.contains("bytes=15"), "{event}");
}

/// A frame restarts the idle wait: three seconds apart never trips five.
#[test]
fn each_frame_restarts_the_idle_wait() {
    let capture = Capture::new();
    let (frames, error, elapsed) = paused(&capture, async {
        let handle = ready();
        let app = client(
            &handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(20, 5), |_scope| {
                        endless(Duration::from_secs(3))
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        until_error(response.body_mut()).await
    });
    assert_eq!(frames, 6);
    assert_eq!(error, "response body ended: deadline_exceeded");
    assert_eq!(elapsed, Duration::from_secs(20));
}
