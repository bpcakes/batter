//! The idle and total bounds end a body the application left running.

use super::support::{Escaped, assert_finished, budget, client, get, next, paused, ready};
use crate::capture::Capture;
use axum::{
    body::{Body, Bytes},
    routing::get as route,
};
use batter_axum::{AdmittedRequest, BodyBudget, GuardedRouter};
use batter_core::{ConfigurationError, operation::Interruption};
use futures_util::stream;
use std::{convert::Infallible, time::Duration};
use tokio::time::Instant;

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
