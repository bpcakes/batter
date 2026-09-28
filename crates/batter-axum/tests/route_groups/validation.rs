//! Route groups that registration or assembly must reject before any routing.

use super::support::{SECOND, request_policy};
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    routing::{MethodFilter, on, post, put},
};
use batter_axum::{
    AssembledHttp, BoundaryAssemblyError, GuardedRouter, HttpBoundary, ProbePath, RouteGroup,
    RouteGroupError,
};
use batter_core::lifecycle::ShutdownHandle;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const GET: MethodFilter = MethodFilter::GET;
const PUT: MethodFilter = MethodFilter::PUT;

/// Application code whose handlers and middleware share one call counter.
struct Calls(Arc<AtomicUsize>);

impl Calls {
    fn new() -> Self {
        Self(Arc::new(AtomicUsize::new(0)))
    }

    fn routes(&self, routes: &[(&str, MethodFilter)]) -> GuardedRouter {
        let layer_calls = self.0.clone();
        routes
            .iter()
            .fold(GuardedRouter::new(), |router, &(path, filter)| {
                let calls = self.0.clone();
                router.route(
                    path,
                    on(filter, move || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        async { "handled" }
                    }),
                )
            })
            .layer(middleware::from_fn(move |request: Request, next: Next| {
                layer_calls.fetch_add(1, Ordering::SeqCst);
                async move { next.run(request).await }
            }))
    }

    fn assert_untouched(&self) {
        assert_eq!(self.0.load(Ordering::SeqCst), 0);
    }
}

async fn assemble(
    default: GuardedRouter,
    groups: Vec<(&'static str, GuardedRouter)>,
    probes: &[&'static str],
) -> Result<AssembledHttp, BoundaryAssemblyError> {
    let handle = ShutdownHandle::new_unapproved();
    let mut boundary = HttpBoundary::new(request_policy(&handle, SECOND));
    for probe in probes {
        boundary = boundary
            .with_liveness(ProbePath::new(probe).unwrap())
            .unwrap();
    }
    for (name, routes) in groups {
        let group = RouteGroup::new(name, request_policy(&handle, SECOND), routes);
        boundary = boundary.with_group(group).unwrap();
    }
    boundary.assemble(default).await
}

fn overlap(first: &'static str, second: &'static str) -> BoundaryAssemblyError {
    BoundaryAssemblyError::OverlappingGroupPaths { first, second }
}

type Routes = &'static [(&'static str, MethodFilter)];

#[tokio::test]
async fn overlapping_group_paths_are_rejected_without_polling_application_code() {
    let calls = Calls::new();
    let cases: [(Routes, Routes, Routes, BoundaryAssemblyError); 4] = [
        // A capture in one group would shadow a literal in another.
        (
            &[("/files/{id}", GET)],
            &[("/files/new", PUT)],
            &[],
            overlap("default", "uploads"),
        ),
        // Different methods on one path still overlap.
        (
            &[("/files", GET)],
            &[("/files", MethodFilter::POST)],
            &[],
            overlap("default", "uploads"),
        ),
        // Named groups are reported in declaration order.
        (
            &[("/items", GET)],
            &[("/assets/{*path}", GET)],
            &[("/assets/private", GET)],
            overlap("uploads", "account"),
        ),
        (
            &[("/items", GET)],
            &[("/uploads/{id}", PUT)],
            &[("/{tenant}/reports", GET)],
            overlap("uploads", "account"),
        ),
    ];
    for (default, uploads, account, expected) in cases {
        let mut groups = vec![("uploads", calls.routes(uploads))];
        if !account.is_empty() {
            groups.push(("account", calls.routes(account)));
        }
        let result = assemble(calls.routes(default), groups, &[]).await;
        assert_eq!(result.err(), Some(expected));
    }
    calls.assert_untouched();
    assert_eq!(
        overlap("uploads", "account").to_string(),
        "route groups uploads and account can match the same path"
    );
}

#[tokio::test]
async fn unrepresentable_overlap_witnesses_are_rejected_without_polling() {
    let calls = Calls::new();
    for path in ["/a b", "/a\tb", "/caf\u{e9}", "/a?b", "/a#b"] {
        for method in [GET, PUT] {
            let result = assemble(
                calls.routes(&[(path, GET)]),
                vec![("uploads", calls.routes(&[(path, method)]))],
                &[],
            )
            .await;
            assert_eq!(result.err(), Some(overlap("default", "uploads")));

            let result = assemble(
                GuardedRouter::new(),
                vec![
                    ("uploads", calls.routes(&[(path, GET)])),
                    ("account", calls.routes(&[(path, method)])),
                ],
                &[],
            )
            .await;
            assert_eq!(result.err(), Some(overlap("uploads", "account")));
        }
    }
    calls.assert_untouched();
}

#[tokio::test]
async fn disjoint_group_paths_assemble() {
    let calls = Calls::new();
    let default = calls.routes(&[("/files/{id}", GET), ("/uploads", GET)]);
    // A wildcard needs a non-empty tail, so it cannot match `/uploads`.
    let uploads = calls.routes(&[("/files/{id}/content", PUT), ("/uploads/{*path}", PUT)]);
    let account = calls.routes(&[("/account/{section}", GET)]);
    let groups = vec![("uploads", uploads), ("account", account)];
    assert!(assemble(default, groups, &["/live"]).await.is_ok());
    calls.assert_untouched();
}

#[tokio::test]
async fn probe_paths_inside_a_named_group_are_rejected_without_polling() {
    let calls = Calls::new();
    for (routes, probe) in [
        (calls.routes(&[("/uploads/{id}", PUT)]), "/uploads/status"),
        (
            GuardedRouter::new().nest("/internal", calls.routes(&[("/{*path}", GET)])),
            "/internal/live",
        ),
    ] {
        let default = calls.routes(&[("/work", GET)]);
        let result = assemble(default, vec![("uploads", routes)], &[probe]).await;
        assert_eq!(
            result.err(),
            Some(BoundaryAssemblyError::GuardedProbePath),
            "{probe}"
        );
    }
    calls.assert_untouched();
}

fn with_group(routes: GuardedRouter, name: &'static str) -> Result<HttpBoundary, RouteGroupError> {
    let handle = ShutdownHandle::new_unapproved();
    HttpBoundary::new(request_policy(&handle, SECOND)).with_group(RouteGroup::new(
        name,
        request_policy(&handle, SECOND),
        routes,
    ))
}

fn one_route() -> GuardedRouter {
    GuardedRouter::new().route("/uploads", put(|| async { "stored" }))
}

#[test]
fn named_groups_need_a_valid_unique_name() {
    let long: &'static str = Box::leak("a".repeat(97).into_boxed_str());
    for name in ["", "has space", "a/b", "caf\u{e9}", long] {
        assert_eq!(
            with_group(one_route(), name).err(),
            Some(RouteGroupError::InvalidName),
            "{name}"
        );
    }
    assert!(with_group(one_route(), &long[..96]).is_ok());
    assert_eq!(
        with_group(one_route(), "default").err(),
        Some(RouteGroupError::DuplicateName("default"))
    );
    let handle = ShutdownHandle::new_unapproved();
    let group = || RouteGroup::new("uploads", request_policy(&handle, SECOND), one_route());
    let repeated = HttpBoundary::new(request_policy(&handle, SECOND))
        .with_group(group())
        .unwrap()
        .with_group(group());
    assert_eq!(
        repeated.err(),
        Some(RouteGroupError::DuplicateName("uploads"))
    );
    assert_eq!(
        RouteGroupError::DuplicateName("uploads").to_string(),
        "route group name is already used: uploads"
    );
    assert_eq!(
        RouteGroupError::InvalidName.to_string(),
        "route group name is invalid"
    );
}

#[test]
fn named_groups_need_routes_and_no_fallback() {
    assert_eq!(
        with_group(GuardedRouter::new(), "uploads").err(),
        Some(RouteGroupError::NoRoutes)
    );
    let fallback = || async { StatusCode::NOT_FOUND };
    let service = tower::service_fn(|_request: Request| async {
        Ok::<_, std::convert::Infallible>(StatusCode::NOT_FOUND)
    });
    let merged = GuardedRouter::new()
        .route("/other", post(|| async {}))
        .fallback(fallback);
    for routes in [
        one_route().fallback(fallback),
        one_route().fallback_service(service),
        one_route().nest("/nested", one_route().fallback(fallback)),
        one_route().merge(merged),
    ] {
        assert_eq!(
            with_group(routes, "uploads").err(),
            Some(RouteGroupError::DeclaresFallback)
        );
    }
    assert_eq!(
        RouteGroupError::DeclaresFallback.to_string(),
        "only the default route group can declare a fallback"
    );
}
