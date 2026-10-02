//! Existing boundary guarantees hold across route groups.

use super::{
    capture::Capture,
    support::{
        ORIGIN, SECOND, assert_not_private, assert_private, body_text, browser_group, exact_origin,
        from_origin, ready_handle, request, request_policy,
    },
};
use axum::{
    Extension, Router,
    http::{Method, StatusCode},
    routing::{get, put},
};
use batter_axum::{GuardedRouter, HttpBoundary, RouteGroup, browser::PrivateResponsePolicy};
use batter_core::{
    lifecycle::ShutdownHandle,
    operation::{Interruption, OperationContext},
};
use std::sync::{Arc, Mutex};
use tokio::time::Instant;
use tower::ServiceExt;
use tracing::instrument::WithSubscriber;

async fn grouped(
    handle: &ShutdownHandle,
    uploads: GuardedRouter,
    account: GuardedRouter,
) -> Router {
    HttpBoundary::new(request_policy(handle, SECOND))
        .with_group(RouteGroup::new(
            "uploads",
            request_policy(handle, 3 * SECOND),
            uploads,
        ))
        .unwrap()
        .with_group(RouteGroup::new(
            "account",
            browser_group(
                handle,
                2 * SECOND,
                PrivateResponsePolicy::NoReferrer,
                exact_origin(),
            ),
            account,
        ))
        .unwrap()
        .assemble(
            GuardedRouter::new()
                .route("/work", get(std::future::pending::<&'static str>))
                .route("/done", get(|| async { "done" })),
        )
        .await
        .unwrap()
        .into_router()
}

fn completions(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| line.contains("HTTP response boundary finished"))
        .collect()
}

#[test]
fn every_group_response_has_one_correlated_completion() {
    let capture = Capture::new();
    let responses = capture.block_on(async {
        let handle = ready_handle();
        let uploads = GuardedRouter::new().route("/uploads/{name}", put(|| async { "stored" }));
        let account = GuardedRouter::new().route("/account", get(|| async { "account" }));
        let app = grouped(&handle, uploads, account).await;
        let mut responses = Vec::new();
        for request in [
            request(Method::GET, "/done"),
            request(Method::PUT, "/uploads/a.csv"),
            from_origin(Method::POST, "/account", "https://attacker.example"),
            from_origin(Method::PUT, "/account", ORIGIN),
            request(Method::GET, "/missing"),
        ] {
            let response = app.clone().oneshot(request).await.unwrap();
            let id = response.headers()["x-request-id"]
                .to_str()
                .unwrap()
                .to_owned();
            responses.push((id, response.status().as_u16()));
        }
        handle.request();
        let draining = app
            .oneshot(request(Method::PUT, "/uploads/a.csv"))
            .await
            .unwrap();
        let id = draining.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        responses.push((id, draining.status().as_u16()));
        responses
    });
    let statuses: Vec<u16> = responses.iter().map(|(_, status)| *status).collect();
    assert_eq!(statuses, [200, 200, 403, 405, 404, 503]);
    let text = capture.text();
    let events = completions(&text);
    assert_eq!(events.len(), responses.len(), "{text}");
    for (id, status) in responses {
        let matching: Vec<&&str> = events
            .iter()
            .filter(|event| event.contains(&format!("request_id=\"{id}\"")))
            .collect();
        assert_eq!(matching.len(), 1, "{text}");
        assert!(matching[0].contains(&format!("status={status}")), "{text}");
    }
}

#[tokio::test(start_paused = true)]
async fn each_group_expires_at_its_own_budget() {
    let pending = || get(std::future::pending::<&'static str>);
    let handle = ready_handle();
    let uploads = GuardedRouter::new().route("/uploads", pending());
    let account = GuardedRouter::new().route("/account", pending());
    let app = grouped(&handle, uploads, account).await;
    for (path, budget, private) in [
        ("/work", SECOND, false),
        ("/uploads", 3 * SECOND, false),
        ("/account", 2 * SECOND, true),
    ] {
        let started = Instant::now();
        let response = app
            .clone()
            .oneshot(request(Method::GET, path))
            .await
            .unwrap();
        assert_eq!(started.elapsed(), budget, "{path}");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
        if private {
            assert_private(response.headers(), "no-referrer");
        } else {
            assert_not_private(response.headers());
        }
        assert!(body_text(response).await.contains("deadline_exceeded"));
    }
}

#[tokio::test]
async fn escaped_group_context_is_cancelled_after_response_construction() {
    let escaped = Arc::new(Mutex::new(None));
    let inside = escaped.clone();
    let uploads = GuardedRouter::new().route(
        "/uploads/{name}",
        put(move |Extension(context): Extension<OperationContext>| {
            let inside = inside.clone();
            async move {
                *inside.lock().unwrap() = Some(context);
                "stored"
            }
        }),
    );
    let account = GuardedRouter::new().route("/account", get(|| async { "account" }));
    let app = grouped(&ready_handle(), uploads, account).await;
    let response = app
        .oneshot(request(Method::PUT, "/uploads/a.csv"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let context = escaped.lock().unwrap().take().unwrap();
    assert_eq!(context.check(), Err(Interruption::Cancelled));
}

struct DropTrace;

impl Drop for DropTrace {
    fn drop(&mut self) {
        tracing::info!(resource = "group.resource", "captured resource dropped");
    }
}

#[test]
fn aborted_group_request_is_destroyed_under_its_first_poll_dispatch() {
    let scoped = Capture::new();
    let ambient = Capture::new();
    ambient.block_on(async {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started = Arc::new(Mutex::new(Some(started_tx)));
        let account = GuardedRouter::new().route(
            "/account",
            put(move || {
                let started = started.clone();
                async move {
                    let _resource = DropTrace;
                    started.lock().unwrap().take().unwrap().send(()).unwrap();
                    std::future::pending::<&'static str>().await
                }
            }),
        );
        let uploads = GuardedRouter::new().route("/uploads", put(|| async { "stored" }));
        let app = grouped(&ready_handle(), uploads, account).await;
        let task = tokio::spawn(
            app.oneshot(from_origin(Method::PUT, "/account", ORIGIN))
                .with_subscriber(scoped.dispatch.clone()),
        );
        started_rx.await.unwrap();
        task.abort();
        let error = task.await.unwrap_err();
        assert!(error.is_cancelled(), "{error}");
    });
    let text = scoped.text();
    assert_eq!(
        text.matches("operation boundary finished").count(),
        1,
        "{text}"
    );
    let events = completions(&text);
    assert_eq!(events.len(), 1, "{text}");
    assert!(events[0].contains("http_outcome=\"dropped\""), "{text}");
    assert!(text.contains("group.resource"), "{text}");
    assert!(ambient.text().is_empty(), "{}", ambient.text());
}
