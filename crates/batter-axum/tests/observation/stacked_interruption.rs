use super::{
    capture::Capture,
    support::{Boundary, request},
};
use axum::{
    Router, body::to_bytes, http::StatusCode, middleware, response::IntoResponse, routing::get,
};
use batter_axum::{
    AdmittedRequest, RequestPolicy, ResponseConstructionBudget, low_level::operational_http,
};
use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, ShutdownHandle, Supervisor},
    operation::Interruption,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;
use tracing::instrument::WithSubscriber;

#[derive(Clone, Copy)]
enum Finish {
    Handler(Interruption),
    Deadline,
    Cancel,
}

#[tokio::test(start_paused = true)]
async fn stacked_admission_consumes_both_returned_and_boundary_owned_interruptions() {
    for mode in [Boundary::Split, Boundary::Combined] {
        for finish in [
            Finish::Handler(Interruption::Cancelled),
            Finish::Handler(Interruption::DeadlineExceeded),
            Finish::Deadline,
            Finish::Cancel,
        ] {
            let capture = Capture::new();
            let one = Duration::from_secs(1);
            let mut supervisor = Supervisor::new(
                ShutdownBudget::new(
                    Duration::ZERO,
                    one,
                    one,
                    CleanupBudget::new(one, one, one).unwrap(),
                )
                .unwrap(),
            );
            let handle = supervisor.handle();
            supervisor
                .register("request-owner", |startup| async move {
                    let shutdown = startup.acknowledge_started();
                    shutdown.cancelled().await;
                    Ok(shutdown.stopped())
                })
                .unwrap();
            let running = supervisor.start();
            handle.status().wait_ready().await.unwrap();
            let (outer, approval) = ShutdownHandle::new_with_readiness_approval();
            approval.approve();
            let calls = Arc::new(AtomicUsize::new(0));
            let renderer_calls = calls.clone();
            let inner_policy = RequestPolicy::new(
                handle.operation_admission(),
                ResponseConstructionBudget::new(one).unwrap(),
            )
            .with_failure_renderer(move |failure, _| {
                renderer_calls.fetch_add(1, Ordering::SeqCst);
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    [("x-rendered", "retained")],
                    failure.code(),
                )
                    .into_response()
            });
            let inside = handle.clone();
            let guarded = Router::new().route(
                "/work",
                get(move |admitted: AdmittedRequest| {
                    let inside = inside.clone();
                    async move {
                        match finish {
                            Finish::Handler(reason) => {
                                admitted.interruption_responder().render(reason)
                            }
                            Finish::Deadline => std::future::pending().await,
                            Finish::Cancel => {
                                inside.request();
                                std::future::pending().await
                            }
                        }
                    }
                }),
            );
            let app = mode
                .apply(
                    mode.apply(guarded, inner_policy),
                    RequestPolicy::new(
                        outer.operation_admission(),
                        ResponseConstructionBudget::new(Duration::from_secs(60)).unwrap(),
                    ),
                )
                .layer(middleware::from_fn(operational_http));
            let response = app
                .oneshot(request("GET", "/work"))
                .with_subscriber(capture.dispatch.clone())
                .await
                .unwrap();
            // Join native supervision before assertions, including a handler-only reply.
            handle.request();
            let report = running.wait().await.unwrap();
            assert!(report.is_success());
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(response.headers()["x-rendered"], "retained");
            let (inner_outcome, code) = match finish {
                Finish::Handler(Interruption::Cancelled) | Finish::Cancel => {
                    ("cancelled", "operation_cancelled")
                }
                Finish::Handler(Interruption::DeadlineExceeded) | Finish::Deadline => {
                    ("deadline_exceeded", "deadline_exceeded")
                }
            };
            assert_eq!(to_bytes(response.into_body(), 4096).await.unwrap(), code);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_outcomes(&capture, inner_outcome);
        }
    }
}

fn assert_outcomes(capture: &Capture, inner_outcome: &str) {
    let text = capture.text();
    let operations: Vec<_> = text
        .lines()
        .filter_map(|line| {
            line.split_once("operation boundary finished")
                .map(|(_, fields)| fields)
        })
        .collect();
    assert_eq!(operations.len(), 2, "{text}");
    assert!(
        operations[0].contains(&format!("outcome=\"{inner_outcome}\"")),
        "{text}"
    );
    assert!(operations[1].contains("outcome=\"succeeded\""), "{text}");
    super::support::assert_completion(&text, 429, "client_error", "/work", "GET");
}
