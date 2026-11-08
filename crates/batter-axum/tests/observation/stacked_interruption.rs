use super::{
    capture::Capture,
    support::{Boundary, request},
};
use axum::{
    Router,
    body::to_bytes,
    http::{Extensions, StatusCode},
    middleware,
    response::IntoResponse,
    routing::get,
};
use batter_axum::{
    AdmittedRequest, RequestPolicy, ResponseConstructionBudget, low_level::operational_http,
};
use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{RunningSupervisor, ShutdownBudget, ShutdownHandle, Supervisor},
    operation::Interruption,
};
use std::{
    sync::{
        Arc, Mutex,
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
    Rejected,
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
            assert_case(mode, finish, Extensions::new()).await;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn stacked_admission_clears_recycled_evidence_from_every_rendering_path() {
    for old_reason in [Interruption::Cancelled, Interruption::DeadlineExceeded] {
        let extensions = interrupted_extensions(old_reason).await;
        for mode in [Boundary::Split, Boundary::Combined] {
            for finish in [
                Finish::Rejected,
                Finish::Deadline,
                Finish::Cancel,
                Finish::Handler(Interruption::Cancelled),
                Finish::Handler(Interruption::DeadlineExceeded),
            ] {
                assert_case(mode, finish, extensions.clone()).await;
            }
        }
    }
}

#[derive(Clone)]
struct RetainedMetadata;

async fn assert_case(mode: Boundary, finish: Finish, extensions: Extensions) {
    let capture = Capture::new();
    let running = ready_owner().await;
    let handle = running.handle();
    if matches!(finish, Finish::Rejected) {
        handle.request();
    }
    let (outer, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let calls = Arc::new(AtomicUsize::new(0));
    let renderer_calls = calls.clone();
    let inner_policy = RequestPolicy::new(
        handle.operation_admission(),
        ResponseConstructionBudget::new(Duration::from_secs(1)).unwrap(),
    )
    .with_failure_renderer(move |failure, _| {
        renderer_calls.fetch_add(1, Ordering::SeqCst);
        let mut response = (
            StatusCode::TOO_MANY_REQUESTS,
            [("x-rendered", "retained")],
            failure.code(),
        )
            .into_response();
        *response.extensions_mut() = extensions.clone();
        response.extensions_mut().insert(RetainedMetadata);
        response
    });
    let inside = handle.clone();
    let guarded = Router::new().route(
        "/work",
        get(move |admitted: AdmittedRequest| {
            let inside = inside.clone();
            async move {
                match finish {
                    Finish::Handler(reason) => admitted.interruption_responder().render(reason),
                    Finish::Rejected => panic!("rejected request reached handler"),
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
    assert!(response.extensions().get::<RetainedMetadata>().is_some());
    let (inner_outcome, code) = match finish {
        Finish::Rejected => (None, "service_unavailable"),
        Finish::Handler(Interruption::Cancelled) | Finish::Cancel => {
            (Some("cancelled"), "operation_cancelled")
        }
        Finish::Handler(Interruption::DeadlineExceeded) | Finish::Deadline => {
            (Some("deadline_exceeded"), "deadline_exceeded")
        }
    };
    assert_eq!(to_bytes(response.into_body(), 4096).await.unwrap(), code);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_outcomes(&capture, inner_outcome);
}

fn assert_outcomes(capture: &Capture, inner_outcome: Option<&str>) {
    let text = capture.text();
    let operations: Vec<_> = text
        .lines()
        .filter_map(|line| {
            line.split_once("operation boundary finished")
                .map(|(_, fields)| fields)
        })
        .collect();
    assert_eq!(
        operations.len(),
        1 + usize::from(inner_outcome.is_some()),
        "{text}"
    );
    if let Some(inner_outcome) = inner_outcome {
        assert!(
            operations[0].contains(&format!("outcome=\"{inner_outcome}\"")),
            "{text}"
        );
    }
    assert!(
        operations.last().unwrap().contains("outcome=\"succeeded\""),
        "{text}"
    );
    super::support::assert_completion(&text, 429, "client_error", "/work", "GET");
}

async fn ready_owner() -> RunningSupervisor {
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
    supervisor
        .register("request-owner", |startup| async move {
            let shutdown = startup.acknowledge_started();
            shutdown.cancelled().await;
            Ok(shutdown.stopped())
        })
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    running
}

async fn interrupted_extensions(reason: Interruption) -> Extensions {
    let saved = Arc::new(Mutex::new(Extensions::new()));
    let store = saved.clone();
    let running = ready_owner().await;
    let app = Boundary::Split
        .apply(
            Router::new().route(
                "/work",
                get(move |admitted: AdmittedRequest| {
                    let store = store.clone();
                    async move {
                        let response = admitted.interruption_responder().render(reason);
                        *store.lock().unwrap() = response.extensions().clone();
                        response
                    }
                }),
            ),
            RequestPolicy::new(
                running.operation_admission(),
                ResponseConstructionBudget::new(Duration::from_secs(1)).unwrap(),
            ),
        )
        .layer(middleware::from_fn(operational_http));
    let response = app.oneshot(request("GET", "/work")).await.unwrap();
    running.handle().request();
    assert!(running.wait().await.unwrap().is_success());
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let extensions = saved.lock().unwrap().clone();
    assert!(
        !extensions.is_empty(),
        "the producer must supply interruption evidence"
    );
    extensions
}
