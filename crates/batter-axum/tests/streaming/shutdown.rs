//! Drain is the stream's decision; forced cancellation is the library's.

use super::support::{
    Process, SECOND, Stops, assert_finished, budget, client, drain, get, next, paused,
};
use crate::capture::Capture;
use axum::{
    body::{Body, Bytes},
    routing::get as route,
};
use batter_axum::{AdmittedRequest, GuardedRouter};
use batter_core::operation::OperationError;
use futures_util::stream;
use std::{convert::Infallible, time::Duration};

/// One frame, then pending forever, ignoring drain and cancellation alike.
fn stuck() -> Body {
    Body::from_stream(stream::unfold(true, |first| async move {
        if !first {
            std::future::pending::<()>().await;
        }
        Some((Ok::<_, Infallible>(Bytes::from_static(b"first;")), false))
    }))
}

#[test]
fn forced_cancellation_ends_a_stream_that_ignores_every_signal() {
    let capture = Capture::new();
    let (error, report) = paused(&capture, async {
        let process = Process::start(Duration::ZERO, Stops::AtForcedCancellation).await;
        let app = client(
            &process.handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(600, 300), |_scope| stuck())
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        assert!(next(response.body_mut()).await.unwrap().is_ok());
        // The transport is already parked in the next frame when cancellation
        // arrives, so only a wakeup from the body itself can end the wait.
        let parked = tokio::spawn(async move {
            let error = next(response.body_mut()).await.unwrap().unwrap_err();
            std::error::Error::source(&error).unwrap().to_string()
        });
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        assert!(!parked.is_finished());
        let requested = tokio::time::Instant::now();
        process.handle.request();
        let error = parked.await.unwrap();
        // Woken by cancellation itself, not later by the idle timer.
        assert_eq!(requested.elapsed(), Duration::ZERO);
        (error, process.report().await)
    });
    assert_eq!(error, "response body ended: cancelled");
    assert!(report.forced_cancellation);
    assert!(report.is_success(), "{report:?}");
    assert_finished(&capture, "WARN", "cancelled");
}

#[test]
fn a_stream_that_watches_drain_ends_complete_before_any_forced_cancellation() {
    let capture = Capture::new();
    let (text, report) = paused(&capture, async {
        let process = Process::start(60 * SECOND, Stops::AtDrain).await;
        let app = client(
            &process.handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(600, 300), |scope| {
                        Body::from_stream(stream::unfold(Some(scope), |scope| async move {
                            let scope = scope?;
                            tokio::select! {
                                biased;
                                () = scope.shutdown().draining() => {
                                    Some((Ok::<_, Infallible>(Bytes::from_static(b"closing;")), None))
                                }
                                () = tokio::time::sleep(SECOND) => {
                                    Some((Ok(Bytes::from_static(b"tick;")), Some(scope)))
                                }
                            }
                        }))
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        assert!(next(response.body_mut()).await.unwrap().is_ok());
        process.handle.request();
        let rest = drain(response.body_mut()).await;
        (rest, process.report().await)
    });
    assert_eq!(text, ("closing;".to_owned(), None));
    assert!(!report.forced_cancellation);
    assert!(report.is_success(), "{report:?}");
    assert_finished(&capture, "INFO", "completed");
}

/// The handler was admitted before drain. Its body starts from that admission,
/// so beginning to stream while the process drains is not refused.
#[test]
fn a_body_that_starts_during_drain_continues_its_admitted_request() {
    let capture = Capture::new();
    let (text, report) = paused(&capture, async {
        // The component outlives drain, as a server does while it still has
        // connections, so the drain allowance is there for the body to use.
        let process = Process::start(60 * SECOND, Stops::AtForcedCancellation).await;
        let draining = process.handle.clone();
        let app = client(
            &process.handle,
            GuardedRouter::new().route(
                "/",
                route(move |admitted: AdmittedRequest| async move {
                    draining.request();
                    admitted.into_streaming_response(budget(600, 300), |scope| {
                        Body::from_stream(stream::once(async move {
                            let step = scope
                                .context()
                                .run("stream.step", |_step| async {
                                    tokio::time::sleep(SECOND).await;
                                    Ok::<_, Infallible>(Bytes::from_static(b"served;"))
                                })
                                .await;
                            match step {
                                Ok(chunk) => Ok(chunk),
                                Err(OperationError::Interrupted(reason)) => {
                                    Err(std::io::Error::other(reason.to_string()))
                                }
                                Err(OperationError::Failed(never)) => match never {},
                            }
                        }))
                    })
                }),
            ),
        )
        .await;
        let mut response = get(&app, "/").await;
        let text = drain(response.body_mut()).await;
        (text, process.report().await)
    });
    assert_eq!(text, ("served;".to_owned(), None));
    assert!(report.is_success(), "{report:?}");
    assert_finished(&capture, "INFO", "completed");
}

/// A source that is always ready never parks, so the bounds are checked
/// before every frame rather than only when a wait begins.
#[test]
fn an_always_ready_source_is_still_ended_by_cancellation_and_by_its_deadline() {
    fn ready_forever() -> Body {
        Body::from_stream(stream::repeat_with(|| {
            Ok::<_, Infallible>(Bytes::from_static(b"tick;"))
        }))
    }
    async fn error_of(body: &mut Body) -> String {
        let error = next(body).await.unwrap().unwrap_err();
        std::error::Error::source(&error).unwrap().to_string()
    }
    let capture = Capture::new();
    let (deadline, cancelled) = paused(&capture, async {
        let process = Process::start(Duration::ZERO, Stops::AtForcedCancellation).await;
        let app = client(
            &process.handle,
            GuardedRouter::new().route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(10, 5), |_scope| ready_forever())
                }),
            ),
        )
        .await;
        let mut expiring = get(&app, "/").await;
        assert!(next(expiring.body_mut()).await.unwrap().is_ok());
        tokio::time::advance(11 * SECOND).await;
        let deadline = error_of(expiring.body_mut()).await;

        let mut cancelling = get(&app, "/").await;
        assert!(next(cancelling.body_mut()).await.unwrap().is_ok());
        process.handle.request();
        // Let the supervisor reach forced cancellation; the body never waits.
        process.handle.signal().cancelled().await;
        (deadline, error_of(cancelling.body_mut()).await)
    });
    assert_eq!(deadline, "response body ended: deadline_exceeded");
    assert_eq!(cancelled, "response body ended: cancelled");
}
