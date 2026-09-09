use super::{
    capture::Capture,
    support::{assert_completion, request},
};
use axum::{
    Extension, Router,
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use batter::{
    lifecycle::ShutdownHandle, operation::OperationContext, telemetry::with_current_dispatch,
};
use batter_axum::{RequestPolicy, liveness, observe_http, readiness, request_admission};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;
use tracing::{Instrument, instrument::WithSubscriber};

pub(super) async fn identity(mut request: Request, next: Next) -> Response {
    with_current_dispatch(async move {
        request
            .extensions_mut()
            .insert(TrustedId("trusted-request-17"));
        let span = tracing::info_span!("application.request", request_id = "trusted-request-17");
        let mut response = next.run(request).instrument(span).await;
        response
            .headers_mut()
            .insert("x-request-id", "trusted-request-17".parse().unwrap());
        response
    })
    .await
}

#[derive(Clone)]
pub(super) struct TrustedId(pub &'static str);

fn assemble(handle: ShutdownHandle, calls: Arc<AtomicUsize>) -> Router {
    let guarded = Router::new()
        .route(
            "/work",
            get(move |Extension(context): Extension<OperationContext>| {
                let calls = calls.clone();
                async move {
                    context.check().unwrap();
                    calls.fetch_add(1, Ordering::SeqCst);
                    "ok"
                }
            }),
        )
        .route(
            "/records/{id}",
            any(
                |Extension(context): Extension<OperationContext>| async move {
                    let result: Result<(), _> = context
                        .run("domain.load", |_| async {
                            tracing::info!("nested dependency called");
                            Err(io::Error::other("secret-provider-error"))
                        })
                        .await;
                    assert!(result.is_err());
                    (StatusCode::INTERNAL_SERVER_ERROR, "secret-response-body")
                },
            ),
        )
        // Adding an application route here needs no per-route observation layer.
        .route("/new", get(|| async { StatusCode::CREATED }))
        .route_layer(middleware::from_fn_with_state(
            RequestPolicy::new(handle.clone(), Duration::from_secs(1)).unwrap(),
            request_admission,
        ));
    async fn rejected_handler() -> StatusCode {
        panic!("rejected handler ran")
    }
    let denied = Router::new()
        .route("/denied", get(rejected_handler))
        .route_layer(middleware::from_fn(
            |_request: Request, _next: Next| async {
                (StatusCode::FORBIDDEN, "denied").into_response()
            },
        ));
    let changed = Router::new()
        .route("/changed", get(|| async { "ok" }))
        .route_layer(middleware::from_fn(|request: Request, next: Next| async {
            let mut response = next.run(request).await;
            *response.status_mut() = StatusCode::ACCEPTED;
            response
        }));
    Router::new()
        .route("/live", get(liveness))
        .route("/ready", get(readiness))
        .with_state(handle)
        .merge(guarded)
        .merge(denied)
        .merge(changed)
        .fallback(|| async { (StatusCode::NOT_FOUND, "missing") })
        .layer(middleware::from_fn(observe_http))
        .layer(middleware::from_fn(identity))
}

async fn check(
    router: &Router,
    method: &str,
    path: &str,
    status: u16,
    outcome: &str,
    route: &str,
) -> String {
    let capture = Capture::new();
    let response = router
        .clone()
        .oneshot(request(method, path))
        .with_subscriber(capture.dispatch.clone())
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), status);
    assert_eq!(response.headers()["x-request-id"], "trusted-request-17");
    let text = capture.text();
    let normalized = if method == "SECRET-METHOD" {
        "OTHER"
    } else {
        method
    };
    assert_completion(&text, status, outcome, route, normalized);
    assert!(text.contains("trusted-request-17"), "{text}");
    text
}

#[tokio::test]
async fn complete_router_observes_probes_rejections_fallback_and_new_routes_once() {
    let handle = ShutdownHandle::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let router = assemble(handle.clone(), calls.clone());
    for phase in ["starting", "ready", "draining"] {
        match phase {
            "ready" => {
                handle.mark_ready();
            }
            "draining" => handle.request(),
            _ => {}
        }
        let (status, outcome) = if phase == "ready" {
            (200, "completed")
        } else {
            (503, "server_error")
        };
        let live = check(&router, "GET", "/live", 200, "completed", "/live").await;
        let ready = check(&router, "GET", "/ready", status, outcome, "/ready").await;
        for probe in [live, ready] {
            assert!(!probe.contains("operation boundary finished"), "{probe}");
        }
        let work = check(&router, "GET", "/work", status, outcome, "/work").await;
        assert_eq!(
            work.matches("operation boundary finished").count(),
            usize::from(phase == "ready")
        );
        check(
            &router,
            "GET",
            "/secret-missing?secret-query=value",
            404,
            "client_error",
            "<unmatched>",
        )
        .await;
        check(&router, "GET", "/denied", 403, "client_error", "/denied").await;
        let (method_status, method_outcome, new_status, new_outcome) = if phase == "ready" {
            (405, "client_error", 201, "completed")
        } else {
            (503, "server_error", 503, "server_error")
        };
        // Router::route_layer also gates a matched path's method fallback.
        for text in [
            check(
                &router,
                "POST",
                "/work",
                method_status,
                method_outcome,
                "/work",
            )
            .await,
            check(&router, "GET", "/new", new_status, new_outcome, "/new").await,
        ] {
            assert_eq!(
                text.matches("operation boundary finished").count(),
                usize::from(phase == "ready"),
                "{text}"
            );
        }
        if phase == "ready" {
            check(&router, "GET", "/changed", 202, "completed", "/changed").await;
            for method in ["GET", "SECRET-METHOD"] {
                let text = check(
                    &router,
                    method,
                    "/records/secret-id?secret-query=value",
                    500,
                    "server_error",
                    "/records/{id}",
                )
                .await;
                let nested = text
                    .lines()
                    .find(|line| line.contains("nested dependency called"))
                    .unwrap();
                for span in ["application.request", "batter.http", "domain.load"] {
                    assert!(nested.contains(span), "{nested}");
                }
            }
        }
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "rejected requests must not run the handler"
    );
}
