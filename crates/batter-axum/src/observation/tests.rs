#[path = "../../tests/support/capture.rs"]
mod capture;

use super::*;
use axum::body::Body;
use batter::{operation::OperationContext, telemetry::with_current_dispatch};
use capture::Capture;
use std::{future::poll_fn, task::Poll};

#[test]
fn observation_preserves_failure_response_without_admission_policy() {
    let capture = Capture::new();
    capture.block_on(async {
        let request = Request::new(Body::empty());
        let response = with_current_dispatch(observe_response(request, |request| async move {
            assert!(request.extensions().get::<OperationContext>().is_none());
            let mut response = Response::new(Body::from("application failure"));
            *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
            response
                .extensions_mut()
                .insert(HttpObservationLevel(Level::ERROR));
            response
                .headers_mut()
                .insert("x-application", "retained".parse().unwrap());
            response
        }))
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()["x-application"], "retained");
        assert_eq!(
            response.extensions().get::<HttpObservationLevel>(),
            Some(&HttpObservationLevel(Level::ERROR))
        );
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap(),
            "application failure"
        );
    });
    let text = capture.text();
    assert_eq!(text.matches("HTTP response boundary finished").count(), 1);
    assert!(text.contains("ERROR"), "{text}");
    assert!(text.contains("status=503"), "{text}");
    assert!(text.contains("http_outcome=\"server_error\""), "{text}");
    assert!(!text.contains("operation boundary finished"), "{text}");
}

#[test]
fn cancelled_observation_keeps_destruction_under_the_owning_dispatch() {
    struct CaptureDrop;
    impl Drop for CaptureDrop {
        fn drop(&mut self) {
            tracing::info!("response future dropped");
        }
    }
    let owner = Capture::new();
    let other = Capture::new();
    let mut pending_observation = None;
    owner.block_on(async {
        let request = Request::new(Body::empty());
        let mut future = Box::pin(with_current_dispatch(observe_response(
            request,
            |_| async {
                let _capture = CaptureDrop;
                std::future::pending::<Response>().await
            },
        )));
        poll_fn(|cx| {
            assert!(future.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        pending_observation = Some(future);
    });
    other.block_on(async {
        drop(pending_observation);
    });
    let text = owner.text();
    assert_eq!(text.matches("response future dropped").count(), 1);
    assert_eq!(text.matches("HTTP response boundary finished").count(), 1);
    assert!(text.contains("WARN"), "{text}");
    assert!(text.contains("http_outcome=\"dropped\""), "{text}");
    assert!(!text.contains("status="), "{text}");
    assert!(other.text().is_empty());
}
