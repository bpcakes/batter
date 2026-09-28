use super::{
    BoundaryAssemblyError, GuardedRouter, HttpBoundary, ProbePath, ProbePathError,
    ProbeRegistrationError,
};
use crate::{ReadinessPolicy, RequestPolicy, ResponseConstructionBudget};
use batter_core::{
    health::{HealthMonitor, HealthPolicy},
    lifecycle::ShutdownHandle,
};
use std::{convert::Infallible, time::Duration};

#[test]
fn probe_path_accepts_only_static_absolute_routes() {
    for valid in ["/", "/live", "/health/ready-v2", "/_internal/~probe"] {
        assert_eq!(ProbePath::new(valid).unwrap().as_str(), valid);
    }
    for (invalid, expected) in [
        ("", ProbePathError::NotAbsolute),
        ("live", ProbePathError::NotAbsolute),
        ("/live/", ProbePathError::EmptySegment),
        ("/health//live", ProbePathError::EmptySegment),
        ("/./live", ProbePathError::DotSegment),
        ("/../live", ProbePathError::DotSegment),
        ("/{probe}", ProbePathError::NonLiteralSegment),
        ("/{*path}", ProbePathError::NonLiteralSegment),
        ("/:probe", ProbePathError::NonLiteralSegment),
        ("/*path", ProbePathError::NonLiteralSegment),
        ("/live?full=1", ProbePathError::NonLiteralSegment),
        ("/live#details", ProbePathError::NonLiteralSegment),
        ("/health%2Flive", ProbePathError::NonLiteralSegment),
    ] {
        assert_eq!(ProbePath::new(invalid), Err(expected), "{invalid}");
    }
    let long = format!("/{}", "a".repeat(256));
    let leaked = Box::leak(long.into_boxed_str());
    assert_eq!(ProbePath::new(leaked), Err(ProbePathError::TooLong));
}

#[test]
fn boundary_rejects_duplicate_probe_paths_before_axum_routing() {
    let second = Duration::from_secs(1);
    let request_policy = || {
        RequestPolicy::new(
            ShutdownHandle::new_unapproved().operation_admission(),
            ResponseConstructionBudget::new(second).unwrap(),
        )
    };
    let path = ProbePath::new("/health").unwrap();
    let duplicate_liveness = HttpBoundary::new(request_policy())
        .with_liveness(path)
        .unwrap()
        .with_liveness(path);
    assert!(matches!(
        duplicate_liveness,
        Err(ProbeRegistrationError::DuplicatePath)
    ));

    let health_policy = HealthPolicy::new(second, second, second * 3, second).unwrap();
    let monitor = HealthMonitor::new(health_policy, || async { Ok::<_, Infallible>(()) });
    let duplicate_cross_kind = HttpBoundary::new(request_policy())
        .with_liveness(path)
        .unwrap()
        .with_readiness(
            path,
            ReadinessPolicy::new(ShutdownHandle::new_unapproved().status(), monitor.reader()),
        );
    assert!(matches!(
        duplicate_cross_kind,
        Err(ProbeRegistrationError::DuplicatePath)
    ));
}

#[tokio::test]
async fn boundary_rejects_guarded_probe_paths_without_polling_application_code() {
    use axum::routing::post;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let second = Duration::from_secs(1);
    let path = ProbePath::new("/health").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let route_calls = calls.clone();
    let fallback_calls = calls.clone();
    let guarded = GuardedRouter::new()
        .route(
            "/health",
            post(move || {
                route_calls.fetch_add(1, Ordering::SeqCst);
                async { "guarded" }
            }),
        )
        .fallback(move || {
            fallback_calls.fetch_add(1, Ordering::SeqCst);
            async { "fallback" }
        });
    let result = HttpBoundary::new(RequestPolicy::new(
        ShutdownHandle::new_unapproved().operation_admission(),
        ResponseConstructionBudget::new(second).unwrap(),
    ))
    .with_liveness(path)
    .unwrap()
    .assemble(guarded)
    .await;

    assert!(matches!(
        result,
        Err(BoundaryAssemblyError::GuardedProbePath)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn rendered_probes_share_duplicate_path_rejection_with_every_probe_kind() {
    use axum::response::{IntoResponse, Response};

    let second = Duration::from_secs(1);
    let boundary = || {
        HttpBoundary::new(RequestPolicy::new(
            ShutdownHandle::new_unapproved().operation_admission(),
            ResponseConstructionBudget::new(second).unwrap(),
        ))
    };
    let health_policy = HealthPolicy::new(second, second, second * 3, second).unwrap();
    let monitor = HealthMonitor::new(health_policy, || async { Ok::<_, Infallible>(()) });
    let readiness =
        || ReadinessPolicy::new(ShutdownHandle::new_unapproved().status(), monitor.reader());
    let live = |_: &axum::http::request::Parts| -> Response { "live".into_response() };
    let ready = |_, _: &axum::http::request::Parts| -> Response { "ready".into_response() };
    let path = ProbePath::new("/health").unwrap();

    let results = [
        boundary()
            .with_rendered_liveness(path, live)
            .unwrap()
            .with_liveness(path)
            .err(),
        boundary()
            .with_liveness(path)
            .unwrap()
            .with_rendered_liveness(path, live)
            .err(),
        boundary()
            .with_readiness(path, readiness())
            .unwrap()
            .with_rendered_readiness(path, readiness(), ready)
            .err(),
        boundary()
            .with_rendered_readiness(path, readiness(), ready)
            .unwrap()
            .with_rendered_liveness(path, live)
            .err(),
        boundary()
            .with_rendered_readiness(path, readiness(), ready)
            .unwrap()
            .with_rendered_readiness(path, readiness(), ready)
            .err(),
    ];
    for result in results {
        assert_eq!(result, Some(ProbeRegistrationError::DuplicatePath));
    }
}

#[tokio::test]
async fn boundary_rejects_guarded_rendered_probe_paths_without_polling_application_code() {
    use axum::{
        response::{IntoResponse, Response},
        routing::{get, post},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let second = Duration::from_secs(1);
    let calls = Arc::new(AtomicUsize::new(0));
    let health_policy = HealthPolicy::new(second, second, second * 3, second).unwrap();
    let monitor = HealthMonitor::new(health_policy, || async { Ok::<_, Infallible>(()) });
    for (guarded_path, liveness) in [("/live", true), ("/{probe}", false)] {
        let route_calls = calls.clone();
        let fallback_calls = calls.clone();
        let (live_calls, ready_calls) = (calls.clone(), calls.clone());
        let guarded = GuardedRouter::new()
            .route(
                guarded_path,
                post(move || {
                    route_calls.fetch_add(1, Ordering::SeqCst);
                    async { "guarded" }
                }),
            )
            .route("/work", get(|| async { "work" }))
            .fallback(move || {
                fallback_calls.fetch_add(1, Ordering::SeqCst);
                async { "fallback" }
            });
        let boundary = HttpBoundary::new(RequestPolicy::new(
            ShutdownHandle::new_unapproved().operation_admission(),
            ResponseConstructionBudget::new(second).unwrap(),
        ));
        let boundary = if liveness {
            boundary.with_rendered_liveness(
                ProbePath::new("/live").unwrap(),
                move |_| -> Response {
                    live_calls.fetch_add(1, Ordering::SeqCst);
                    "live".into_response()
                },
            )
        } else {
            boundary.with_rendered_readiness(
                ProbePath::new("/ready").unwrap(),
                ReadinessPolicy::new(ShutdownHandle::new_unapproved().status(), monitor.reader()),
                move |_, _| -> Response {
                    ready_calls.fetch_add(1, Ordering::SeqCst);
                    "ready".into_response()
                },
            )
        };
        let result = boundary.unwrap().assemble(guarded).await;

        assert!(
            matches!(result, Err(BoundaryAssemblyError::GuardedProbePath)),
            "{guarded_path}"
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn boundary_rejects_nested_guarded_probe_paths_without_polling_application_code() {
    use axum::routing::post;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let second = Duration::from_secs(1);
    let path = ProbePath::new("/api/health").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let route_calls = calls.clone();
    let nested = GuardedRouter::new().route(
        "/health",
        post(move || {
            route_calls.fetch_add(1, Ordering::SeqCst);
            async { "guarded" }
        }),
    );
    let guarded = GuardedRouter::new().nest("/api", nested);
    let result = HttpBoundary::new(RequestPolicy::new(
        ShutdownHandle::new_unapproved().operation_admission(),
        ResponseConstructionBudget::new(second).unwrap(),
    ))
    .with_liveness(path)
    .unwrap()
    .assemble(guarded)
    .await;

    assert!(matches!(
        result,
        Err(BoundaryAssemblyError::GuardedProbePath)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
