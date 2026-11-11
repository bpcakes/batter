use super::{capture::Capture, support::request};
use axum::{body::to_bytes, http::StatusCode, response::IntoResponse, routing::get};
use batter_axum::{
    AdmittedRequest, GuardedRouter, HttpBoundary, RequestPolicy, ResponseConstructionBudget,
};
use batter_core::{
    admission::{Admission, AdmissionError, Bulkhead, BulkheadCapacity, BulkheadError},
    lifecycle::ShutdownHandle,
    operation::Interruption,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tracing::instrument::WithSubscriber;

#[tokio::test]
async fn expiry_before_bulkhead_execution_retains_correlated_deadline_telemetry() {
    for filter in ["info", "info,batter=warn,batter::request=info"] {
        let capture = Capture::with_filter(filter);
        let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
        approval.approve();
        let policy = RequestPolicy::new(
            handle.operation_admission(),
            ResponseConstructionBudget::new(Duration::from_millis(5)).unwrap(),
        )
        .with_infrastructure_json();
        let client = HttpBoundary::new(policy)
            .assemble(GuardedRouter::new().route(
                "/work",
                get(|admitted: AdmittedRequest| async move {
                    // Exhaust the budget within one poll. The enclosing timer cannot
                    // observe expiry before admission returns its interruption response.
                    while tokio::time::Instant::now() < admitted.context().deadline() {
                        std::hint::spin_loop();
                    }
                    let bulkhead = Bulkhead::new(BulkheadCapacity::new(1).unwrap());
                    let result = bulkhead
                        .run(
                            admitted.context(),
                            Admission::Reject,
                            "forbidden.work",
                            |_| -> std::future::Ready<Result<(), std::convert::Infallible>> {
                                panic!("expired admission must not invoke the factory");
                            },
                        )
                        .await;
                    assert!(matches!(
                        result,
                        Err(BulkheadError::NotAdmitted(AdmissionError::Interrupted(
                            Interruption::DeadlineExceeded
                        )))
                    ));
                    admitted
                        .interruption_responder()
                        .render(Interruption::DeadlineExceeded)
                }),
            ))
            .await
            .unwrap()
            .in_process();
        let response = client
            .request(request("GET", "/work"))
            .with_subscriber(capture.dispatch.clone())
            .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(
            std::str::from_utf8(&body)
                .unwrap()
                .contains("deadline_exceeded")
        );
        let text = capture.text();
        let operations: Vec<_> = text
            .lines()
            .filter(|line| line.contains("operation boundary finished"))
            .collect();
        assert_eq!(operations.len(), 1, "{text}");
        assert!(
            operations[0].contains("outcome=\"deadline_exceeded\""),
            "{text}"
        );
        assert!(operations[0].contains(&id), "{text}");
        assert!(operations[0].trim_start().starts_with("WARN "), "{text}");
        super::support::assert_completion(&text, 503, "server_error", "/work", "GET");
    }
}

#[tokio::test]
async fn only_the_returned_interruption_response_classifies_the_operation() {
    for reason in [Interruption::Cancelled, Interruption::DeadlineExceeded] {
        for return_interruption in [false, true] {
            let capture = Capture::new();
            let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
            approval.approve();
            let calls = Arc::new(AtomicUsize::new(0));
            let rendered = calls.clone();
            let policy = RequestPolicy::new(
                handle.operation_admission(),
                ResponseConstructionBudget::new(Duration::from_secs(60)).unwrap(),
            )
            .with_failure_renderer(move |failure, _| {
                rendered.fetch_add(1, Ordering::SeqCst);
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    [("x-rendered", "retained")],
                    failure.code(),
                )
                    .into_response()
            });
            let client = HttpBoundary::new(policy)
                .assemble(GuardedRouter::new().route(
                    "/work",
                    get(move |admitted: AdmittedRequest| async move {
                        let response = admitted.interruption_responder().render(reason);
                        if return_interruption {
                            response
                        } else {
                            // A discarded rendering must not taint an unrelated 503.
                            StatusCode::SERVICE_UNAVAILABLE.into_response()
                        }
                    }),
                ))
                .await
                .unwrap()
                .in_process();
            let response = client
                .request(request("GET", "/work"))
                .with_subscriber(capture.dispatch.clone())
                .await;
            assert_eq!(
                calls.load(Ordering::SeqCst),
                1,
                "renderer must not run twice"
            );
            let expected = if return_interruption {
                assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
                assert_eq!(response.headers()["x-rendered"], "retained");
                let body = to_bytes(response.into_body(), 4096).await.unwrap();
                match reason {
                    Interruption::Cancelled => {
                        assert_eq!(body, "operation_cancelled");
                        "cancelled"
                    }
                    Interruption::DeadlineExceeded => {
                        assert_eq!(body, "deadline_exceeded");
                        "deadline_exceeded"
                    }
                }
            } else {
                assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
                "succeeded"
            };
            let text = capture.text();
            let operations: Vec<_> = text
                .lines()
                .filter(|line| line.contains("operation boundary finished"))
                .collect();
            assert_eq!(operations.len(), 1, "{text}");
            assert!(
                operations[0].contains(&format!("outcome=\"{expected}\"")),
                "{text}"
            );
        }
    }
}
