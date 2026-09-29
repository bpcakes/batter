//! Handlers extract the admitted request as one value that only admission
//! inside server correlation records; everywhere else the extractor answers a
//! sanitized rejection, and raw extensions can neither supply nor replace it.

use crate::capture::Capture;
use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use batter_axum::{
    AdmittedRequest, AdmittedRequestRejection, CorrelationId, GuardedRouter, HttpBoundary,
    RequestPolicy, ResponseConstructionBudget, RouteGroup, operational_http, request_admission,
    request_scope,
};
use batter_core::{
    lifecycle::ShutdownHandle,
    operation::{Interruption, OperationContext, OperationOwner},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;

const SECOND: Duration = Duration::from_secs(1);
const PROBLEM: &str = r#"{"type":"about:blank","title":"Internal Server Error","status":500,"code":"internal_error"}"#;

fn ready() -> ShutdownHandle {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    handle
}

fn policy(handle: &ShutdownHandle, budget: Duration) -> RequestPolicy {
    RequestPolicy::new(
        handle.operation_admission(),
        ResponseConstructionBudget::new(budget).unwrap(),
    )
    .with_infrastructure_json()
}

fn get_request(path: &str) -> Request {
    Request::builder()
        .uri(path)
        .header("x-request-id", "secret-forged-header")
        .body(Body::empty())
        .unwrap()
}

async fn text(response: Response) -> String {
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

fn header_text(response: &Response, name: &str) -> String {
    response.headers()[name].to_str().unwrap().to_owned()
}

/// The response of a handler that extracted an admitted request outside
/// admission: the fixed Problem JSON of an internal failure and nothing else.
async fn assert_rejected(response: Response) {
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(text(response).await, PROBLEM);
}

type Seen = Arc<Mutex<Vec<OperationContext>>>;

/// Report what a guarded handler extracted, keeping its context for later checks.
async fn report(
    State(seen): State<Seen>,
    admitted: AdmittedRequest,
    Extension(native): Extension<OperationContext>,
) -> Response {
    seen.lock().unwrap().push(admitted.context().clone());
    let remaining = admitted.context().remaining().as_millis().to_string();
    let same = (admitted.context().deadline() == native.deadline()).to_string();
    (
        [
            ("x-admitted-id", admitted.correlation_id().to_string()),
            ("x-remaining-ms", remaining),
            ("x-native-deadline", same),
        ],
        "admitted",
    )
        .into_response()
}

#[test]
fn boundary_handlers_extract_their_groups_admission_and_generated_identity() {
    let capture = Capture::new();
    let seen = Seen::default();
    let responses = capture.block_on({
        let seen = seen.clone();
        async move {
            let handle = ready();
            let uploads = GuardedRouter::new()
                .route("/uploads", get(report))
                .with_state(seen.clone());
            let default = GuardedRouter::new()
                .route("/work", get(report))
                .route(
                    "/interrupt",
                    get(|admitted: AdmittedRequest| async move {
                        admitted
                            .interruption_responder()
                            .render(Interruption::Cancelled)
                    }),
                )
                .fallback(report)
                .with_state(seen);
            let app = HttpBoundary::new(policy(&handle, 2 * SECOND))
                .with_group(RouteGroup::new(
                    "uploads",
                    policy(&handle, 90 * SECOND),
                    uploads,
                ))
                .unwrap()
                .assemble(default)
                .await
                .unwrap()
                .into_router();
            let mut responses = Vec::new();
            for path in ["/work", "/missing", "/uploads", "/interrupt"] {
                responses.push(app.clone().oneshot(get_request(path)).await.unwrap());
            }
            responses
        }
    });
    let [work, missing, uploads, interrupt] = <[Response; 4]>::try_from(responses).unwrap();
    let mut ids = Vec::new();
    for (response, budget) in [
        (work, 2 * SECOND),
        (missing, 2 * SECOND),
        (uploads, 90 * SECOND),
    ] {
        assert_eq!(response.status(), StatusCode::OK);
        let id = header_text(&response, "x-request-id");
        assert_eq!(id.len(), 36);
        assert_eq!(header_text(&response, "x-admitted-id"), id);
        assert_eq!(header_text(&response, "x-native-deadline"), "true");
        let remaining =
            Duration::from_millis(header_text(&response, "x-remaining-ms").parse().unwrap());
        // Each group's own budget: only the upload group's exceeds 2 s.
        assert!(remaining <= budget, "{remaining:?}");
        assert_eq!(remaining > 2 * SECOND, budget > 2 * SECOND, "{remaining:?}");
        ids.push(id);
    }
    // Admission cancels each request's context once its response is built.
    for context in seen.lock().unwrap().iter() {
        assert_eq!(context.check(), Err(Interruption::Cancelled));
    }
    assert_eq!(seen.lock().unwrap().len(), 3);

    // The responder renders with the admission policy's envelope and identity.
    assert_eq!(interrupt.status(), StatusCode::SERVICE_UNAVAILABLE);
    let id = header_text(&interrupt, "x-request-id");
    assert_eq!(
        capture.block_on(text(interrupt)),
        format!(
            r#"{{"code":"operation_cancelled","message":"The operation was cancelled","request_id":"{id}"}}"#
        )
    );
    ids.push(id);

    let text = capture.text();
    assert!(!text.contains("secret-"), "{text}");
    for id in ids {
        let events = text
            .lines()
            .filter(|line| {
                line.contains("HTTP response boundary finished")
                    && line.contains(&format!("request_id=\"{id}\""))
            })
            .count();
        assert_eq!(events, 1, "{text}");
    }
}

#[test]
fn handlers_outside_admission_receive_the_sanitized_rejection() {
    let capture = Capture::new();
    let (late, correlated, (native_status, native_body), typed) = capture.block_on(async {
        let handle = ready();
        let unreachable = |_: AdmittedRequest| async { StatusCode::IM_A_TEAPOT };
        let boundary = HttpBoundary::new(policy(&handle, SECOND))
            .assemble(GuardedRouter::new().route("/work", get(|| async { "admitted" })))
            .await
            .unwrap()
            .into_router()
            // Routes added to the assembled router sit outside the boundary.
            .route("/late", get(unreachable));
        let correlated = Router::new()
            .route("/correlated", get(unreachable))
            .route(
                "/native",
                get(|_: Extension<OperationContext>| async { StatusCode::IM_A_TEAPOT }),
            )
            .route(
                "/typed",
                get(
                    |admitted: Result<AdmittedRequest, AdmittedRequestRejection>| async move {
                        admitted.unwrap_err().to_string()
                    },
                ),
            )
            .layer(middleware::from_fn(operational_http));
        let late = boundary.oneshot(get_request("/late")).await.unwrap();
        let mut responses = Vec::new();
        for path in ["/correlated", "/native", "/typed"] {
            responses.push(correlated.clone().oneshot(get_request(path)).await.unwrap());
        }
        let [correlated, native, typed] = <[Response; 3]>::try_from(responses).unwrap();
        (
            late,
            correlated,
            (native.status(), text(native).await),
            text(typed).await,
        )
    });
    assert!(late.headers().get("x-request-id").is_none());
    capture.block_on(assert_rejected(late));
    let id = header_text(&correlated, "x-request-id");
    capture.block_on(assert_rejected(correlated));

    // Axum's own extension rejection, which the extractor replaces, names the type.
    assert_eq!(native_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        native_body.contains("batter_core::operation::OperationContext"),
        "{native_body}"
    );
    assert_eq!(typed, "request was not admitted inside server correlation");

    let text = capture.text();
    let event = text
        .lines()
        .find(|line| {
            line.contains("HTTP response boundary finished")
                && line.contains(&format!("request_id=\"{id}\""))
        })
        .unwrap();
    assert!(event.trim_start().starts_with("WARN"), "{text}");
    assert!(event.contains("status=500"), "{text}");
    assert!(event.contains("route=\"/correlated\""), "{text}");
}

fn identity_route() -> Router {
    Router::new().route(
        "/work",
        get(|admitted: AdmittedRequest| async move { admitted.correlation_id().to_string() }),
    )
}

#[tokio::test]
async fn lower_level_admission_records_the_request_only_inside_server_correlation() {
    let handle = ready();
    let admission = || middleware::from_fn_with_state(policy(&handle, SECOND), request_admission);
    // The identity comes from the wrapper that generated it, not from the
    // public extension that an application layer in between can remove.
    let remove_public_identity = || {
        middleware::from_fn(|mut request: Request, next: Next| async move {
            request.extensions_mut().remove::<CorrelationId>();
            next.run(request).await
        })
    };
    let supported = [
        identity_route()
            .route_layer(admission())
            .layer(middleware::from_fn(operational_http)),
        identity_route()
            .route_layer(middleware::from_fn_with_state(
                policy(&handle, SECOND),
                request_scope,
            ))
            .layer(middleware::from_fn(operational_http)),
        identity_route()
            .route_layer(admission())
            .layer(remove_public_identity())
            .layer(middleware::from_fn(operational_http)),
    ];
    for app in supported {
        let response = app.oneshot(get_request("/work")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let id = header_text(&response, "x-request-id");
        assert_eq!(text(response).await, id);
    }

    let without_correlation = [
        identity_route().route_layer(admission()),
        // Correlation inside admission is the documented unsupported order.
        identity_route()
            .route_layer(middleware::from_fn(operational_http))
            .layer(admission()),
    ];
    for app in without_correlation {
        assert_rejected(app.oneshot(get_request("/work")).await.unwrap()).await;
    }
}

#[tokio::test]
async fn raw_extensions_cannot_supply_or_replace_the_admitted_request() {
    let handle = ready();
    let forged = OperationOwner::new(600 * SECOND).unwrap().into_context();

    // Outside admission a native context from an application layer satisfies
    // `Extension<OperationContext>` but not the admitted request.
    let outside = Router::new()
        .route("/admitted", get(|_: AdmittedRequest| async { "forged" }))
        .route(
            "/native",
            get(
                |Extension(context): Extension<OperationContext>| async move {
                    context.remaining().as_secs().to_string()
                },
            ),
        )
        .layer(Extension(forged.clone()))
        .layer(middleware::from_fn(operational_http));
    let native = outside
        .clone()
        .oneshot(get_request("/native"))
        .await
        .unwrap();
    assert_eq!(native.status(), StatusCode::OK);
    assert!(text(native).await.parse::<u64>().unwrap() > 500);
    assert_rejected(outside.oneshot(get_request("/admitted")).await.unwrap()).await;

    // Inside admission an application layer can replace or remove the native
    // extensions, but the admitted request keeps admission's own values.
    let guarded = GuardedRouter::new()
        .route(
            "/work",
            get(
                |admitted: AdmittedRequest,
                 Extension(native): Extension<OperationContext>,
                 correlation: Option<Extension<CorrelationId>>| async move {
                    assert!(correlation.is_none());
                    [
                        ("x-admitted-id", admitted.correlation_id().to_string()),
                        (
                            "x-admitted-ms",
                            admitted.context().remaining().as_millis().to_string(),
                        ),
                        ("x-native-ms", native.remaining().as_millis().to_string()),
                    ]
                },
            ),
        )
        .route_layer(middleware::from_fn(
            move |mut request: Request, next: Next| {
                let forged = forged.clone();
                async move {
                    request.extensions_mut().insert(forged);
                    request.extensions_mut().remove::<CorrelationId>();
                    next.run(request).await
                }
            },
        ));
    let response = HttpBoundary::new(policy(&handle, SECOND))
        .assemble(guarded)
        .await
        .unwrap()
        .into_router()
        .oneshot(get_request("/work"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header_text(&response, "x-admitted-id"),
        header_text(&response, "x-request-id")
    );
    let admitted: u128 = header_text(&response, "x-admitted-ms").parse().unwrap();
    let native: u128 = header_text(&response, "x-native-ms").parse().unwrap();
    assert!(admitted <= 1_000 && native > 500_000, "{admitted} {native}");
}
